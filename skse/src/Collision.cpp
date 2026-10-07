// Re-plan step D: Skyrim's real collision shapes for Garry's Mod (DESIGN.md).
//
// Adapted from SkyCraft (MIT, (c) 2026 chasmlol, https://github.com/chasmlol/SkyCraft),
// skse/src/Collision.cpp: the region streaming, the layer list, the Havok shape decoding (MOPP/BV
// trees, lists, meshes, triangles, boxes, capsules, convex hulls, transforms; offsets as SkyCraft
// found them on 1.7.104), the guarded reads, the triangles made from boxes / capsules / hulls,
// the 1 s refresh near the player and the per-frame limits. Changed for Garry's Mod: everything
// stays Z-up in Skyrim units (no Minecraft axis swap), only triangles are sent (no voxels or
// digging; of the materials only "is it a stair material", S2i-1, read with SkyCraft's
// Dig.cpp ShapeMaterial / GuardedMaterial), regions are ordered by where the player is going (velocity look-ahead),
// and the triangles are converted to Garry's Mod units relative to the anchor like the grids.
#include "Collision.h"

#include "Link.h"
#include "Puppet.h"

#include <array>
#include <chrono>
#include <condition_variable>
#include <deque>
#include <mutex>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace gmsky::Collision
{
	namespace
	{
		using Clock = std::chrono::steady_clock;

		constexpr int kRadius = 5;   // regions around the player horizontally (as SkyCraft)
		constexpr int kBelow = 3;    // regions below the player
		constexpr int kAbove = 2;    // regions above the player
		constexpr int kForget = 6;   // a region further than this is no longer "sent" (Garry's Mod drops at 8)
		constexpr auto kRefreshNear = 1000ms;  // re-send regions next to the player this often (doors etc.)
		constexpr int kHarvests = 3;           // regions per frame (SkyCraft)
		constexpr auto kBudget = 2500us;
		constexpr int kHarvestsFast = 6;       // ... while the player moves faster than kFastSpeed
		constexpr auto kBudgetFast = 5000us;
		constexpr float kFastSpeed = 1000.0f;  // Skyrim units per second
		constexpr double kLeadSeconds = 0.5;   // order regions around where the player is in this long
		constexpr float kQueryMargin = 16.0f;  // Skyrim units around a region when collecting shapes
		constexpr float kSendMargin = 24.0f;   // ... and when choosing which triangles go with it
		constexpr int kMaxKeys = 16384;
		constexpr std::size_t kMaxTris = 150000;  // per region (a message may use half the ring)

		struct Tri
		{
			float v[9];  // Skyrim units
			uint32_t flags{ 0 };
		};
		struct Obb
		{
			float c[3];
			float axis[3][3];
			float half[3];
			uint32_t flags{ 0 };  // GMSKY_TRI_* for the triangles made from it
		};
		struct Capsule
		{
			float a[3], b[3];
			float r;
			uint32_t flags{ 0 };
		};
		struct Convex
		{
			std::vector<std::array<float, 4>> planes;  // n.p + d <= 0 inside, Skyrim units
			float lo[3], hi[3];
			uint32_t flags{ 0 };
		};
		struct Job
		{
			bool clear{ false };
			uint32_t rawType{ 0 };      // S2k-K1: a ready message (Loose.cpp), sent as it is
			std::vector<uint8_t> raw;
			int rx{ 0 }, ry{ 0 }, rz{ 0 };
			uint32_t colEpoch{ 0 }, originEpoch{ 0 };
			double origin[3]{};
			std::vector<Tri> tris;
			std::vector<Obb> boxes;
			std::vector<Capsule> capsules;
			std::vector<Convex> convexes;
			bool helper{ false };  // collecting a stair helper body right now
			const RE::hkpShape* top{ nullptr };  // the body's own shape (its material, S2i-1)
		};
		struct Body
		{
			const RE::hkpShape* shape;
			const float* xf;  // hkTransform (column-major rotation + translation), Havok units
			float lo[3], hi[3];  // world AABB, Skyrim units
			bool helper;
		};

		// ---- small math (as SkyCraft) ----
		bool Finite(const float* a_v, int a_n)
		{
			for (int i = 0; i < a_n; ++i) {
				if (!std::isfinite(a_v[i]) || std::fabs(a_v[i]) > 1.0e7f) {
					return false;
				}
			}
			return true;
		}
		void XfPoint(const float* a_xf, const float* a_p, float* a_out)
		{
			for (int i = 0; i < 3; ++i) {
				a_out[i] = a_xf[i] * a_p[0] + a_xf[4 + i] * a_p[1] + a_xf[8 + i] * a_p[2] + a_xf[12 + i];
			}
		}
		void XfDir(const float* a_xf, const float* a_d, float* a_out)
		{
			for (int i = 0; i < 3; ++i) {
				a_out[i] = a_xf[i] * a_d[0] + a_xf[4 + i] * a_d[1] + a_xf[8 + i] * a_d[2];
			}
		}
		void XfCompose(const float* a_parent, const float* a_child, float* a_out)
		{
			for (int c = 0; c < 3; ++c) {
				XfDir(a_parent, a_child + c * 4, a_out + c * 4);
				a_out[c * 4 + 3] = 0.0f;
			}
			XfPoint(a_parent, a_child + 12, a_out + 12);
			a_out[15] = 1.0f;
		}
		bool XfLooksValid(const float* a_xf)
		{
			if (!Finite(a_xf, 16)) {
				return false;
			}
			for (int c = 0; c < 3; ++c) {
				const float* col = a_xf + c * 4;
				if (std::fabs(col[0] * col[0] + col[1] * col[1] + col[2] * col[2] - 1.0f) > 0.05f) {
					return false;
				}
			}
			return true;
		}
		void HkToSky(const float* a_h, float a_k, float* a_out)
		{
			a_out[0] = a_h[0] * a_k;
			a_out[1] = a_h[1] * a_k;
			a_out[2] = a_h[2] * a_k;
		}
		void HkAabbToSky(const RE::hkAabb& a_box, float a_k, float* a_lo, float* a_hi)
		{
			alignas(16) float mn[4], mx[4];
			_mm_store_ps(mn, a_box.min.quad);
			_mm_store_ps(mx, a_box.max.quad);
			for (int i = 0; i < 3; ++i) {
				a_lo[i] = mn[i] * a_k;
				a_hi[i] = mx[i] * a_k;
			}
		}
		bool Overlaps(const float* a_lo, const float* a_hi, const float* b_lo, const float* b_hi)
		{
			return a_lo[0] <= b_hi[0] && a_hi[0] >= b_lo[0] && a_lo[1] <= b_hi[1] && a_hi[1] >= b_lo[1] && a_lo[2] <= b_hi[2] && a_hi[2] >= b_lo[2];
		}
		const float* Vec(const void* a_base, std::size_t a_offset)
		{
			return reinterpret_cast<const float*>(reinterpret_cast<const uint8_t*>(a_base) + a_offset);
		}
		template <class T>
		T Field(const void* a_base, std::size_t a_offset)
		{
			T value;
			std::memcpy(&value, reinterpret_cast<const uint8_t*>(a_base) + a_offset, sizeof(T));
			return value;
		}
		inline void Sub(const float* a, const float* b, float* o) { o[0] = a[0] - b[0], o[1] = a[1] - b[1], o[2] = a[2] - b[2]; }
		inline void Cross(const float* a, const float* b, float* o)
		{
			o[0] = a[1] * b[2] - a[2] * b[1];
			o[1] = a[2] * b[0] - a[0] * b[2];
			o[2] = a[0] * b[1] - a[1] * b[0];
		}
		inline float Dot(const float* a, const float* b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }

		bool Included(RE::COL_LAYER a_layer)
		{
			switch (a_layer) {
			case RE::COL_LAYER::kStairHelper:
			case RE::COL_LAYER::kStatic:
			case RE::COL_LAYER::kAnimStatic:
			case RE::COL_LAYER::kTransparent:
			case RE::COL_LAYER::kTrees:
			case RE::COL_LAYER::kProps:
			case RE::COL_LAYER::kTerrain:
			case RE::COL_LAYER::kGround:
			case RE::COL_LAYER::kInvisibleWall:
				return true;
			default:
				return false;
			}
		}

		// Havok shape layouts are partly reverse-engineered: never let a bad read take down the game.
		bool GuardedAabb(const RE::hkpShape* a_shape, const float* a_xf, RE::hkAabb& a_out)
		{
			__try {
				a_shape->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(a_xf), 0.0f, a_out);
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}
		using CollectFn = void (*)(const RE::hkpShape*, const float*, const float*, const float*, void*);
		bool GuardedCall(CollectFn a_fn, const RE::hkpShape* a_shape, const float* a_xf, const float* a_lo, const float* a_hi, void* a_job)
		{
			__try {
				a_fn(a_shape, a_xf, a_lo, a_hi, a_job);
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		uint64_t RegionKey(int a_x, int a_y, int a_z)
		{
			return (uint64_t(uint32_t(a_x) & 0x1FFFFF) << 42) | (uint64_t(uint32_t(a_y) & 0x1FFFFF) << 21) | (uint32_t(a_z) & 0x1FFFFF);
		}

		// ---- state (main thread unless noted) ----
		bool g_streaming = false;
		uint32_t g_originEpoch = 0;
		std::atomic<uint32_t> g_colEpoch{ 0 };  // also read by the worker
		std::atomic<uint64_t> g_wRawBytes{ 0 }, g_wRawDropped{ 0 };  // S2k-K1: Loose.cpp's messages
		std::unordered_map<uint64_t, Clock::time_point> g_harvested;
		std::vector<Body> g_bodies;
		std::unordered_set<int> g_loggedTypes;
		// Region order: offsets in the window around the player's region, sorted by distance to
		// the look-ahead point's region; rebuilt when either changes.
		std::vector<std::array<int, 3>> g_order;
		std::array<int, 6> g_orderFor{ INT_MIN };

		// Worker.
		std::mutex g_mutex;
		std::condition_variable g_cv;
		std::deque<Job> g_queue;
		bool g_workerStarted = false;
		std::vector<std::array<int, 3>> g_dropped;  // regions the worker couldn't send (ring full); under g_mutex

		// Per second (main thread).
		uint32_t g_secRegions = 0, g_secFaults = 0, g_secFrames = 0;
		double g_secHarvestMs = 0.0, g_secGatherMs = 0.0, g_worstFrameMs = 0.0;
		std::size_t g_lastBodies = 0;
		std::string g_stopReason = "not started";
		// Per second (worker; atomics).
		// S2f: stair ramps found this second: whole bodies on the ramp layer, children of a container
		// on it (the container's own layer differs), triangles sent with the mark; and every layer
		// seen on container children (logged once each).
		uint32_t g_secRampBodies = 0, g_secRampChildren = 0, g_secFilterFaults = 0;
		std::unordered_set<int> g_loggedChildLayers;
		std::atomic<uint64_t> g_wRampTris{ 0 };
		// S2i-1: triangles sent with a stair material, material reads that faulted, and every stair
		// material seen (logged once each).
		std::atomic<uint64_t> g_wStairTris{ 0 };
		uint32_t g_secMaterialFaults = 0;
		std::unordered_set<uint32_t> g_loggedStairMaterials;
		std::atomic<uint64_t> g_wTris{ 0 }, g_wBytes{ 0 }, g_wRegions{ 0 }, g_wFullWaits{ 0 }, g_wDropped{ 0 }, g_wUs{ 0 };
		std::atomic<int64_t> g_playerRegionTris{ -1 };
		std::atomic<int> g_prx{ INT_MIN }, g_pry{ INT_MIN }, g_prz{ INT_MIN };

		float SkyPerHavok() { return RE::bhkWorld::GetWorldScaleInverse(); }

		void Collect(const RE::hkpShape* a_shape, const float* a_xf, const float a_lo[3], const float a_hi[3], Job& a_job, int a_depth, RE::hkpShapeKey a_key);

		// ---- S2i-1: stair materials ----
		// Every Havok triangle has a Skyrim material; stairs have their own. Adapted from SkyCraft
		// (MIT) Dig.cpp ShapeMaterial / GuardedMaterial: the material of the body's shape at the key
		// of its first container level (a MOPP tree's mesh has one material per triangle). Unlike
		// SkyCraft the whole read is inside the fault guard (userData too): a bad read loses only the
		// mark, never the body (test 124's lesson).
		const char* StairMaterialName(RE::MATERIAL_ID a_id)
		{
			using M = RE::MATERIAL_ID;
			switch (a_id) {
			case M::kStoneStairs: return "StoneStairs";
			case M::kWoodStairs: return "WoodStairs";
			case M::kSnowStairs: return "SnowStairs";
			case M::kGlassStairs: return "GlassStairs";
			case M::kStoneStairsBroken: return "StoneStairsBroken";
			case M::kStoneAsStairs: return "StoneAsStairs";
			case M::kWoodAsStairs: return "WoodAsStairs";
			default: return nullptr;
			}
		}

		bool GuardedShapeMaterial(const RE::hkpShape* a_top, RE::hkpShapeKey a_key, RE::MATERIAL_ID& a_out)
		{
			__try {
				const RE::hkpShape* shape = a_top;
				if (shape && shape->type == RE::hkpShapeType::kMOPP) {
					if (const auto* child = static_cast<const RE::hkpMoppBvTreeShape*>(shape)->child.childShape; child && child->userData) {
						shape = child;
					}
				}
				if (!shape || !shape->userData) {
					a_out = a_top && a_top->userData ? a_top->userData->materialID : RE::MATERIAL_ID::kNone;
					return true;
				}
				const auto id = shape->userData->GetMaterialID(a_key);
				a_out = id != RE::MATERIAL_ID::kNone ? id : shape->userData->materialID;
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		// GMSKY_TRI_* for a piece of the body being collected (a_key as in SkyCraft's FlagsFor).
		uint32_t FlagsFor(const Job& a_job, RE::hkpShapeKey a_key)
		{
			uint32_t flags = a_job.helper ? GMSKY_TRI_STAIR_HELPER : 0;
			RE::MATERIAL_ID id = RE::MATERIAL_ID::kNone;
			if (!GuardedShapeMaterial(a_job.top, a_key, id)) {
				++g_secMaterialFaults;
				return flags;
			}
			if (const char* name = StairMaterialName(id)) {
				flags |= GMSKY_TRI_STAIRS;
				if (g_loggedStairMaterials.insert(static_cast<uint32_t>(id)).second) {
					logger::info("COLLISION stair material seen: {} ({})", name, static_cast<uint32_t>(id));
				}
			}
			return flags;
		}

		// A container's child can carry its own layer (Havok's per-key filter info), e.g. a stair ramp
		// inside a building's mesh (S2f; SkyCraft reads only the whole body's layer). Marks the job
		// as collecting a ramp while that child is collected; returns the mark before, to restore.
		// The child's filter info, from the container's function table slot 4
		// (getCollisionFilterInfo), called as returning a plain number. Not through CommonLib's
		// GetCollisionFilterInfo: it returns the class CFilter, which MSVC returns from a member
		// function through a hidden pointer, so Havok got that pointer as the key (test 124: 7,078
		// faults, the ground stopped arriving). Own fault guard: a bad read loses only the mark.
		bool GuardedFilterInfo(const RE::hkpShapeContainer* a_container, RE::hkpShapeKey a_key, uint32_t& a_out)
		{
			using Fn = uint32_t (*)(const RE::hkpShapeContainer*, RE::hkpShapeKey);
			__try {
				const auto* vtable = *reinterpret_cast<Fn* const*>(a_container);
				a_out = vtable[4](a_container, a_key);
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		bool ChildRamp(const RE::hkpShapeContainer* a_container, RE::hkpShapeKey a_key, Job& a_job)
		{
			const bool was = a_job.helper;
			uint32_t info = 0;
			if (!GuardedFilterInfo(a_container, a_key, info)) {
				++g_secFilterFaults;
				return was;
			}
			const auto layer = static_cast<int>(info & 0x7F);
			if (layer != 0 && g_loggedChildLayers.insert(layer).second) {
				logger::info("COLLISION a container child carries its own layer {}{}", layer,
					layer == static_cast<int>(RE::COL_LAYER::kStairHelper) ? " (stair ramp)" : "");
			}
			if (!was && layer == static_cast<int>(RE::COL_LAYER::kStairHelper)) {
				a_job.helper = true;
				++g_secRampChildren;
			}
			return was;
		}

		void EmitAabbFallback(const RE::hkpShape* a_shape, const float* a_xf, Job& a_job, RE::hkpShapeKey a_key)
		{
			RE::hkAabb box;
			a_shape->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(a_xf), 0.0f, box);
			float lo[3], hi[3];
			HkAabbToSky(box, SkyPerHavok(), lo, hi);
			if (!Finite(lo, 3) || !Finite(hi, 3) || hi[0] - lo[0] > 4096 || hi[1] - lo[1] > 4096 || hi[2] - lo[2] > 4096) {
				return;
			}
			Obb obb{};
			obb.flags = FlagsFor(a_job, a_key);
			for (int i = 0; i < 3; ++i) {
				obb.c[i] = (lo[i] + hi[i]) * 0.5f;
				obb.half[i] = (hi[i] - lo[i]) * 0.5f;
				obb.axis[i][0] = obb.axis[i][1] = obb.axis[i][2] = 0.0f;
				obb.axis[i][i] = 1.0f;
			}
			a_job.boxes.push_back(obb);
		}

		void Collect(const RE::hkpShape* a_shape, const float* a_xf, const float a_lo[3], const float a_hi[3], Job& a_job, int a_depth, RE::hkpShapeKey a_key)
		{
			if (!a_shape || a_depth > 8 || a_job.tris.size() > kMaxTris) {
				return;
			}
			const float k = SkyPerHavok();
			using T = RE::hkpShapeType;
			const auto type = a_shape->type;
			switch (type) {
			case T::kMOPP:
			case T::kBVTree:
				{
					auto* bv = static_cast<const RE::hkpBvTreeShape*>(a_shape);
					// Query box: Skyrim -> Havok world -> shape-local (inverse transform of the corners).
					const float hlo[3] = { a_lo[0] / k, a_lo[1] / k, a_lo[2] / k };
					const float hhi[3] = { a_hi[0] / k, a_hi[1] / k, a_hi[2] / k };
					float llo[3] = { FLT_MAX, FLT_MAX, FLT_MAX }, lhi[3] = { -FLT_MAX, -FLT_MAX, -FLT_MAX };
					for (int c = 0; c < 8; ++c) {
						const float p[3] = { (c & 1) ? hhi[0] : hlo[0], (c & 2) ? hhi[1] : hlo[1], (c & 4) ? hhi[2] : hlo[2] };
						const float d[3] = { p[0] - a_xf[12], p[1] - a_xf[13], p[2] - a_xf[14] };
						for (int i = 0; i < 3; ++i) {
							const float v = a_xf[i * 4] * d[0] + a_xf[i * 4 + 1] * d[1] + a_xf[i * 4 + 2] * d[2];  // R^T d
							llo[i] = std::min(llo[i], v);
							lhi[i] = std::max(lhi[i], v);
						}
					}
					RE::hkAabb local;
					local.min = RE::hkVector4(llo[0], llo[1], llo[2], 0.0f);
					local.max = RE::hkVector4(lhi[0], lhi[1], lhi[2], 0.0f);
					static thread_local std::vector<RE::hkpShapeKey> keys(kMaxKeys);
					const auto found = std::min<uint32_t>(bv->QueryAabbImpl(local, keys.data(), kMaxKeys), kMaxKeys);
					const auto* container = bv->GetContainer();
					if (!container) {
						return;
					}
					for (uint32_t i = 0; i < found; ++i) {
						RE::hkpShapeBuffer buffer;
						const bool was = ChildRamp(container, keys[i], a_job);
						Collect(container->GetChildShape(keys[i], buffer), a_xf, a_lo, a_hi, a_job, a_depth + 1, a_depth == 0 ? keys[i] : a_key);
						a_job.helper = was;
					}
					return;
				}
			case T::kList:
			case T::kCollection:
			case T::kCompressedMesh:
			case T::kExtendedMesh:
			case T::kTriangleCollection:
			case T::kConvexList:
				{
					const auto* container = a_shape->GetContainer();
					if (!container) {
						EmitAabbFallback(a_shape, a_xf, a_job, a_key);
						return;
					}
					int guard = 0;
					for (auto key = container->GetFirstKey(); key != RE::HK_INVALID_SHAPE_KEY && guard < 200000; key = container->GetNextKey(key), ++guard) {
						RE::hkpShapeBuffer buffer;
						const auto* child = container->GetChildShape(key, buffer);
						if (!child) {
							continue;
						}
						RE::hkAabb box;
						child->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(a_xf), 0.0f, box);
						float lo[3], hi[3];
						HkAabbToSky(box, k, lo, hi);
						if (Overlaps(lo, hi, a_lo, a_hi)) {
							const bool was = ChildRamp(container, key, a_job);
							Collect(child, a_xf, a_lo, a_hi, a_job, a_depth + 1, a_depth == 0 ? key : a_key);
							a_job.helper = was;
						}
					}
					return;
				}
			case T::kTriangle:
				{
					Tri tri{};
					for (int v = 0; v < 3; ++v) {
						float w[3];
						XfPoint(a_xf, Vec(a_shape, 0x30 + v * 0x10), w);
						HkToSky(w, k, tri.v + v * 3);
					}
					if (Finite(tri.v, 9)) {
						tri.flags = FlagsFor(a_job, a_key);
						a_job.tris.push_back(tri);
					}
					return;
				}
			case T::kBox:
				{
					const float* half = Vec(a_shape, 0x30);
					const float radius = Field<float>(a_shape, 0x20);
					Obb obb{};
					obb.flags = FlagsFor(a_job, a_key);
					const float zero[3] = { 0, 0, 0 };
					float centre[3];
					XfPoint(a_xf, zero, centre);
					HkToSky(centre, k, obb.c);
					for (int i = 0; i < 3; ++i) {
						std::memcpy(obb.axis[i], a_xf + i * 4, sizeof(float) * 3);
						obb.half[i] = (half[i] + radius) * k;
					}
					if (Finite(obb.c, 3) && Finite(obb.half, 3)) {
						a_job.boxes.push_back(obb);
					}
					return;
				}
			case T::kCapsule:
			case T::kSphere:
				{
					const float radius = Field<float>(a_shape, 0x20);
					Capsule cap{};
					cap.flags = FlagsFor(a_job, a_key);
					float w[3];
					const float zero[3] = { 0, 0, 0 };
					XfPoint(a_xf, type == T::kCapsule ? Vec(a_shape, 0x30) : zero, w);
					HkToSky(w, k, cap.a);
					XfPoint(a_xf, type == T::kCapsule ? Vec(a_shape, 0x40) : zero, w);
					HkToSky(w, k, cap.b);
					cap.r = radius * k;
					if (Finite(cap.a, 3) && Finite(cap.b, 3) && std::isfinite(cap.r) && cap.r < 70000.0f) {
						a_job.capsules.push_back(cap);
					}
					return;
				}
			case T::kConvexVertices:
				{
					const auto& planes = *reinterpret_cast<const RE::hkArray<RE::hkVector4>*>(reinterpret_cast<const uint8_t*>(a_shape) + 0x78);
					const float radius = Field<float>(a_shape, 0x20);
					if (planes.size() <= 0 || planes.size() > 512) {
						EmitAabbFallback(a_shape, a_xf, a_job, a_key);
						return;
					}
					Convex cvx{};
					cvx.flags = FlagsFor(a_job, a_key);
					RE::hkAabb box;
					a_shape->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(a_xf), 0.0f, box);
					HkAabbToSky(box, k, cvx.lo, cvx.hi);
					for (int32_t i = 0; i < planes.size(); ++i) {
						alignas(16) float p[4];
						_mm_store_ps(p, planes.data()[i].quad);
						float nw[3];
						XfDir(a_xf, p, nw);
						const float dw = p[3] - (nw[0] * a_xf[12] + nw[1] * a_xf[13] + nw[2] * a_xf[14]) - radius;
						cvx.planes.push_back({ nw[0], nw[1], nw[2], dw * k });
					}
					if (Finite(cvx.lo, 3) && Finite(cvx.hi, 3)) {
						a_job.convexes.push_back(std::move(cvx));
					}
					return;
				}
			case T::kConvexTransform:
			case T::kConvexTranslate:
				{
					const auto* child = Field<const RE::hkpShape*>(a_shape, 0x30);
					alignas(16) float local[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 };
					if (type == T::kConvexTransform) {
						std::memcpy(local, Vec(a_shape, 0x40), sizeof(local));
					} else {
						std::memcpy(local + 12, Vec(a_shape, 0x40), sizeof(float) * 3);
					}
					if (!child || !XfLooksValid(local)) {
						EmitAabbFallback(a_shape, a_xf, a_job, a_key);
						return;
					}
					alignas(16) float composed[16];
					XfCompose(a_xf, local, composed);
					Collect(child, composed, a_lo, a_hi, a_job, a_depth + 1, a_key);
					return;
				}
			case T::kTransform:
				{
					const auto* child = Field<const RE::hkpShape*>(a_shape, 0x28);
					alignas(16) float local[16];
					std::memcpy(local, Vec(a_shape, 0x50), sizeof(local));
					if (!child || !XfLooksValid(local)) {
						EmitAabbFallback(a_shape, a_xf, a_job, a_key);
						return;
					}
					alignas(16) float composed[16];
					XfCompose(a_xf, local, composed);
					Collect(child, composed, a_lo, a_hi, a_job, a_depth + 1, a_key);
					return;
				}
			default:
				if (g_loggedTypes.insert(static_cast<int>(type)).second) {
					logger::info("COLLISION shape type {} handled as its bounding box (convex={})", static_cast<int>(type), a_shape->IsConvex());
				}
				if (a_shape->IsConvex()) {
					EmitAabbFallback(a_shape, a_xf, a_job, a_key);
				}
				return;
			}
		}

		void GatherBodies(RE::hkpWorld* a_world)
		{
			g_bodies.clear();
			const float k = SkyPerHavok();
			auto addIsland = [&](RE::hkpSimulationIsland* a_island) {
				if (!a_island) {
					return;
				}
				auto& entities = a_island->entities;
				for (int32_t i = 0; i < entities.size(); ++i) {
					auto* entity = entities.data()[i];
					if (!entity) {
						continue;
					}
					const auto& collidable = entity->collidable;
					const auto layer = collidable.GetCollisionLayer();
					if (!SentAsGround(layer, entity->motion.type.get())) {
						continue;
					}
					const auto* shape = collidable.shape;
					const auto* xf = static_cast<const float*>(collidable.motion);
					if (!shape || !xf || !Finite(xf, 16)) {
						continue;
					}
					RE::hkAabb box;
					if (!GuardedAabb(shape, xf, box)) {
						continue;
					}
					Body body{ shape, xf, {}, {}, layer == RE::COL_LAYER::kStairHelper };
					HkAabbToSky(box, k, body.lo, body.hi);
					if (Finite(body.lo, 3) && Finite(body.hi, 3)) {
						g_bodies.push_back(body);
					}
				}
			};
			addIsland(a_world->fixedIsland);
			for (int32_t i = 0; i < a_world->activeSimulationIslands.size(); ++i) {
				addIsland(a_world->activeSimulationIslands.data()[i]);
			}
			for (int32_t i = 0; i < a_world->inactiveSimulationIslands.size(); ++i) {
				addIsland(a_world->inactiveSimulationIslands.data()[i]);
			}
		}

		void Harvest(int a_rx, int a_ry, int a_rz, const GmskyOrigin& a_origin)
		{
			Job job{};
			job.rx = a_rx;
			job.ry = a_ry;
			job.rz = a_rz;
			job.colEpoch = g_colEpoch.load();
			job.originEpoch = a_origin.epoch;
			std::memcpy(job.origin, a_origin.skyrim_origin, sizeof(job.origin));
			const float lo[3] = { a_rx * kRegionSize - kQueryMargin, a_ry * kRegionSize - kQueryMargin, a_rz * kRegionSize - kQueryMargin };
			const float hi[3] = { (a_rx + 1) * kRegionSize + kQueryMargin, (a_ry + 1) * kRegionSize + kQueryMargin, (a_rz + 1) * kRegionSize + kQueryMargin };
			static constexpr CollectFn collect = [](const RE::hkpShape* a_shape, const float* a_xf, const float* a_lo, const float* a_hi, void* a_job) {
				Collect(a_shape, a_xf, a_lo, a_hi, *static_cast<Job*>(a_job), 0, RE::HK_INVALID_SHAPE_KEY);
			};
			for (const auto& body : g_bodies) {
				if (!Overlaps(body.lo, body.hi, lo, hi)) {
					continue;
				}
				job.helper = body.helper;
				job.top = body.shape;
				g_secRampBodies += body.helper;
				if (!GuardedCall(collect, body.shape, body.xf, lo, hi, &job)) {
					++g_secFaults;
					if (g_loggedTypes.insert(-1000 - static_cast<int>(body.shape->type)).second) {
						logger::warn("COLLISION faulted reading a Havok shape (type {}); skipping it", static_cast<int>(body.shape->type));
					}
				}
			}
			std::scoped_lock lock(g_mutex);
			g_queue.push_back(std::move(job));
			g_cv.notify_one();
		}

		// ---- worker ----

		// Boxes, capsules (as boxes) and convex hulls -> triangles (as SkyCraft's Triangulate).
		void Triangulate(const Job& a_src, std::vector<Tri>& a_out)
		{
			a_out.insert(a_out.end(), a_src.tris.begin(), a_src.tris.end());
			uint32_t flags = 0;  // the shape being turned into triangles: a stair ramp or not
			auto emit = [&](const float* a, const float* b, const float* c) {
				a_out.push_back(Tri{ { a[0], a[1], a[2], b[0], b[1], b[2], c[0], c[1], c[2] }, flags });
			};
			auto quad = [&](const float* a, const float* b, const float* c, const float* d) {
				emit(a, b, c);
				emit(a, c, d);
			};
			auto box = [&](const float* c, const float (*axis)[3], const float* half) {
				float corner[8][3];
				for (int i = 0; i < 8; ++i) {
					const float sx = (i & 1) ? 1.0f : -1.0f, sy = (i & 2) ? 1.0f : -1.0f, sz = (i & 4) ? 1.0f : -1.0f;
					for (int k = 0; k < 3; ++k) {
						corner[i][k] = c[k] + axis[0][k] * half[0] * sx + axis[1][k] * half[1] * sy + axis[2][k] * half[2] * sz;
					}
				}
				quad(corner[0], corner[1], corner[3], corner[2]);
				quad(corner[4], corner[5], corner[7], corner[6]);
				quad(corner[0], corner[1], corner[5], corner[4]);
				quad(corner[2], corner[3], corner[7], corner[6]);
				quad(corner[0], corner[2], corner[6], corner[4]);
				quad(corner[1], corner[3], corner[7], corner[5]);
			};
			for (const auto& b : a_src.boxes) {
				flags = b.flags;
				box(b.c, b.axis, b.half);
			}
			for (const auto& cap : a_src.capsules) {
				flags = cap.flags;
				float ab[3];
				Sub(cap.b, cap.a, ab);
				const float len = std::sqrt(Dot(ab, ab));
				float axis[3][3]{};
				if (len > 1e-3f) {
					for (int k = 0; k < 3; ++k) {
						axis[2][k] = ab[k] / len;
					}
				} else {
					axis[2][2] = 1.0f;
				}
				const float ref[3] = { std::fabs(axis[2][2]) < 0.9f ? 0.0f : 1.0f, 0.0f, std::fabs(axis[2][2]) < 0.9f ? 1.0f : 0.0f };
				Cross(ref, axis[2], axis[0]);
				const float l0 = std::sqrt(Dot(axis[0], axis[0]));
				for (int k = 0; k < 3; ++k) {
					axis[0][k] /= l0;
				}
				Cross(axis[2], axis[0], axis[1]);
				const float c[3] = { (cap.a[0] + cap.b[0]) * 0.5f, (cap.a[1] + cap.b[1]) * 0.5f, (cap.a[2] + cap.b[2]) * 0.5f };
				const float half[3] = { cap.r, cap.r, len * 0.5f + cap.r };
				box(c, axis, half);
			}
			// Convex hull from planes: clip a big square on each plane by all the other planes.
			for (const auto& cvx : a_src.convexes) {
				flags = cvx.flags;
				const float ex = cvx.hi[0] - cvx.lo[0], ey = cvx.hi[1] - cvx.lo[1], ez = cvx.hi[2] - cvx.lo[2];
				const float diag = std::sqrt(ex * ex + ey * ey + ez * ez) + 1.0f;
				const float mid[3] = { (cvx.lo[0] + cvx.hi[0]) * 0.5f, (cvx.lo[1] + cvx.hi[1]) * 0.5f, (cvx.lo[2] + cvx.hi[2]) * 0.5f };
				for (std::size_t i = 0; i < cvx.planes.size(); ++i) {
					const auto& pl = cvx.planes[i];
					const float n[3] = { pl[0], pl[1], pl[2] };
					const float nl = std::sqrt(Dot(n, n));
					if (nl < 1e-6f) {
						continue;
					}
					const float dist = (Dot(n, mid) + pl[3]) / (nl * nl);
					const float o[3] = { mid[0] - n[0] * dist, mid[1] - n[1] * dist, mid[2] - n[2] * dist };
					const float ref[3] = { std::fabs(n[2]) < 0.9f * nl ? 0.0f : 1.0f, 0.0f, std::fabs(n[2]) < 0.9f * nl ? 1.0f : 0.0f };
					float t1[3], t2[3];
					Cross(ref, n, t1);
					const float lt = std::sqrt(Dot(t1, t1));
					for (int k = 0; k < 3; ++k) {
						t1[k] /= lt;
					}
					Cross(n, t1, t2);
					const float l2 = std::sqrt(Dot(t2, t2));
					for (int k = 0; k < 3; ++k) {
						t2[k] /= l2;
					}
					std::vector<std::array<float, 3>> poly;
					const float sgn1[4] = { -1, 1, 1, -1 }, sgn2[4] = { -1, -1, 1, 1 };
					for (int q = 0; q < 4; ++q) {
						const float s1 = sgn1[q] * diag, s2 = sgn2[q] * diag;
						poly.push_back({ o[0] + t1[0] * s1 + t2[0] * s2, o[1] + t1[1] * s1 + t2[1] * s2, o[2] + t1[2] * s1 + t2[2] * s2 });
					}
					for (std::size_t j = 0; j < cvx.planes.size() && poly.size() >= 3; ++j) {
						if (j == i) {
							continue;
						}
						const auto& cp = cvx.planes[j];
						std::vector<std::array<float, 3>> out;
						for (std::size_t v = 0; v < poly.size(); ++v) {
							const auto& A = poly[v];
							const auto& B = poly[(v + 1) % poly.size()];
							const float da = cp[0] * A[0] + cp[1] * A[1] + cp[2] * A[2] + cp[3];
							const float db = cp[0] * B[0] + cp[1] * B[1] + cp[2] * B[2] + cp[3];
							if (da <= 0.0f) {
								out.push_back(A);
							}
							if ((da <= 0.0f) != (db <= 0.0f)) {
								const float t = da / (da - db);
								out.push_back({ A[0] + (B[0] - A[0]) * t, A[1] + (B[1] - A[1]) * t, A[2] + (B[2] - A[2]) * t });
							}
						}
						poly.swap(out);
					}
					for (std::size_t v = 1; v + 1 < poly.size(); ++v) {
						emit(poly[0].data(), poly[v].data(), poly[v + 1].data());
					}
				}
			}
		}

		bool Send(uint32_t a_type, const std::vector<uint8_t>& a_payload)
		{
			auto& link = Link::Get();
			for (int attempt = 0; attempt < 2000; ++attempt) {
				if (link.RingWrite(a_type, a_payload.data(), uint32_t(a_payload.size()))) {
					return true;
				}
				++g_wFullWaits;
				std::this_thread::sleep_for(1ms);  // ring full: Garry's Mod is behind (or not running)
			}
			return false;
		}

		void SendRegion(const Job& a_job)
		{
			std::vector<Tri> all;
			Triangulate(a_job, all);
			const float lo[3] = { a_job.rx * kRegionSize - kSendMargin, a_job.ry * kRegionSize - kSendMargin, a_job.rz * kRegionSize - kSendMargin };
			const float hi[3] = { (a_job.rx + 1) * kRegionSize + kSendMargin, (a_job.ry + 1) * kRegionSize + kSendMargin, (a_job.rz + 1) * kRegionSize + kSendMargin };
			const double s = kGmodPerSkyrim;
			auto g = [&](float a_v, int a_axis) { return float((double(a_v) - a_job.origin[a_axis]) * s); };
			std::vector<GmskyColTri> out;
			out.reserve(all.size());
			for (const auto& t : all) {
				float tlo[3], thi[3];
				for (int k = 0; k < 3; ++k) {
					tlo[k] = std::min({ t.v[k], t.v[3 + k], t.v[6 + k] });
					thi[k] = std::max({ t.v[k], t.v[3 + k], t.v[6 + k] });
				}
				if (!Overlaps(tlo, thi, lo, hi) || !Finite(t.v, 9)) {
					continue;
				}
				GmskyColTri o{};
				for (int k = 0; k < 9; ++k) {
					o.v[k] = g(t.v[k], k % 3);
				}
				o.flags = t.flags;
				out.push_back(o);
				g_wRampTris += (t.flags & GMSKY_TRI_STAIR_HELPER) != 0;
				g_wStairTris += (t.flags & GMSKY_TRI_STAIRS) != 0;
				if (out.size() >= kMaxTris) {
					break;
				}
			}
			GmskyColRegion h{};
			h.rx = a_job.rx;
			h.ry = a_job.ry;
			h.rz = a_job.rz;
			h.col_epoch = a_job.colEpoch;
			h.count = uint32_t(out.size());
			h.lo[0] = g(a_job.rx * kRegionSize, 0);
			h.lo[1] = g(a_job.ry * kRegionSize, 1);
			h.lo[2] = g(a_job.rz * kRegionSize, 2);
			h.size = float(kRegionSize * s);
			std::vector<uint8_t> payload(sizeof(h) + out.size() * sizeof(GmskyColTri));
			std::memcpy(payload.data(), &h, sizeof(h));
			if (!out.empty()) {
				std::memcpy(payload.data() + sizeof(h), out.data(), out.size() * sizeof(GmskyColTri));
			}
			if (Send(GMSKY_COL_REGION, payload)) {
				g_wTris += out.size();
				g_wBytes += payload.size();
				++g_wRegions;
				if (a_job.rx == g_prx.load() && a_job.ry == g_pry.load() && a_job.rz == g_prz.load()) {
					g_playerRegionTris = int64_t(out.size());
				}
			} else {
				++g_wDropped;
				std::scoped_lock lock(g_mutex);
				g_dropped.push_back({ a_job.rx, a_job.ry, a_job.rz });
			}
		}

		void WorkerLoop()
		{
			while (true) {
				Job job;
				{
					std::unique_lock lock(g_mutex);
					g_cv.wait(lock, [] { return !g_queue.empty(); });
					job = std::move(g_queue.front());
					g_queue.pop_front();
				}
				const auto t0 = Clock::now();
				try {
					if (job.clear) {
						const GmskyColClear c{ job.colEpoch, job.originEpoch };
						std::vector<uint8_t> payload(sizeof(c));
						std::memcpy(payload.data(), &c, sizeof(c));
						if (!Send(GMSKY_COL_CLEAR, payload)) {
							logger::warn("COLLISION ring stayed full; the clear message was dropped");
						}
					} else if (job.colEpoch != g_colEpoch.load()) {
						// an older stream's job
					} else if (job.rawType != 0) {
						if (Send(job.rawType, job.raw)) {
							g_wRawBytes += job.raw.size();
						} else {
							++g_wRawDropped;
						}
					} else {
						SendRegion(job);
					}
				} catch (const std::exception& e) {
					logger::error("COLLISION worker: {}", e.what());
				}
				g_wUs += uint64_t(std::chrono::duration_cast<std::chrono::microseconds>(Clock::now() - t0).count());
			}
		}

		void Start(const GmskyOrigin& a_origin)
		{
			if (!g_workerStarted) {
				std::thread(WorkerLoop).detach();
				g_workerStarted = true;
			}
			g_harvested.clear();
			g_orderFor = { INT_MIN };
			g_playerRegionTris = -1;
			const uint32_t epoch = g_colEpoch.load() + 1;
			g_colEpoch = epoch;
			g_originEpoch = a_origin.epoch;
			g_streaming = true;
			std::scoped_lock lock(g_mutex);
			g_queue.clear();
			g_dropped.clear();
			Job job{};
			job.clear = true;
			job.colEpoch = epoch;
			job.originEpoch = a_origin.epoch;
			g_queue.push_back(std::move(job));
			g_cv.notify_one();
			logger::info("COLLISION streaming starts: collision epoch {}, origin epoch {}", epoch, a_origin.epoch);
		}

		void Stop(std::string a_reason)
		{
			if (g_streaming) {
				logger::info("COLLISION streaming stops: {}", a_reason);
				std::scoped_lock lock(g_mutex);
				g_queue.clear();
			}
			g_streaming = false;
			g_stopReason = std::move(a_reason);
		}

		int FloorRegion(double a_v) { return int(std::floor(a_v / kRegionSize)); }
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin)
	{
		GmskyPlayerState g{};
		if (!a_player || !Puppet::IsActive()) {
			return Stop("puppet off");
		}
		if (!SlotRead(Link::Get().Shared().player_state, g) || g.origin_epoch != a_origin.epoch) {
			return;  // Garry's Mod hasn't anchored to this origin yet
		}
		if (!(g.flags & GMSKY_PF_SHAPES)) {
			return Stop("Garry's Mod asks for the ray grid (gmsky_ground grid)");
		}
		auto* cell = a_player->GetParentCell();
		auto* bhk = cell ? cell->GetbhkWorld() : nullptr;
		auto* world = bhk ? bhk->GetWorld1() : nullptr;
		if (!world) {
			return;
		}
		if (!g_streaming || g_originEpoch != a_origin.epoch) {
			Start(a_origin);
		}
		const auto t0 = Clock::now();
		const auto pos = a_player->GetPosition();
		const int prx = FloorRegion(pos.x), pry = FloorRegion(pos.y), prz = FloorRegion(pos.z);
		g_prx = prx, g_pry = pry, g_prz = prz;
		// Where the player will be (Garry's Mod's velocity, in Skyrim units), at most 3 regions on.
		const double vx = g.vel[0] / kGmodPerSkyrim, vy = g.vel[1] / kGmodPerSkyrim, vz = g.vel[2] / kGmodPerSkyrim;
		const double speed = std::sqrt(vx * vx + vy * vy + vz * vz);
		auto lead = [&](int a_p, double a_v, double a_c) { return std::clamp(FloorRegion(a_c + a_v * kLeadSeconds), a_p - 3, a_p + 3); };
		const int lx = lead(prx, vx, pos.x), ly = lead(pry, vy, pos.y), lz = lead(prz, vz, pos.z);

		// Forget regions far away: they are sent again when needed (Garry's Mod drops them at 8).
		{
			std::scoped_lock lock(g_mutex);
			for (const auto& r : g_dropped) {
				g_harvested.erase(RegionKey(r[0], r[1], r[2]));
			}
			g_dropped.clear();
		}
		if (prx != g_orderFor[0] || pry != g_orderFor[1] || prz != g_orderFor[2]) {
			std::erase_if(g_harvested, [&](const auto& a_e) {
				auto unpack = [](uint64_t a_key, int a_shift) {
					const int v = int((a_key >> a_shift) & 0x1FFFFF);
					return v >= 0x100000 ? v - 0x200000 : v;
				};
				return std::abs(unpack(a_e.first, 42) - prx) > kForget || std::abs(unpack(a_e.first, 21) - pry) > kForget ||
				       std::abs(unpack(a_e.first, 0) - prz) > kForget;
			});
		}

		// The window around the player's region, nearest the look-ahead point first; the 3 x 3 x 3
		// around the player before everything else.
		const std::array<int, 6> orderFor{ prx, pry, prz, lx, ly, lz };
		if (orderFor != g_orderFor) {
			g_orderFor = orderFor;
			g_order.clear();
			for (int dx = -kRadius; dx <= kRadius; ++dx) {
				for (int dy = -kRadius; dy <= kRadius; ++dy) {
					for (int dz = -kBelow; dz <= kAbove; ++dz) {
						g_order.push_back({ prx + dx, pry + dy, prz + dz });
					}
				}
			}
			auto key = [&](const std::array<int, 3>& r) {
				const int nx = r[0] - prx, ny = r[1] - pry, nz = r[2] - prz;
				if (std::abs(nx) <= 1 && std::abs(ny) <= 1 && std::abs(nz) <= 1) {
					return nx * nx + ny * ny + nz * nz * 2 - 1000;
				}
				const int ax = r[0] - lx, ay = r[1] - ly, az = r[2] - lz;
				return ax * ax + ay * ay + az * az * 2;
			};
			std::ranges::stable_sort(g_order, {}, key);
		}

		const bool fast = speed > kFastSpeed;
		const int maxHarvests = fast ? kHarvestsFast : kHarvests;
		const auto budget = fast ? kBudgetFast : kBudget;
		const auto now = Clock::now();
		int done = 0;
		bool gathered = false;
		for (const auto& r : g_order) {
			const auto k = RegionKey(r[0], r[1], r[2]);
			const auto it = g_harvested.find(k);
			const bool isNear = std::abs(r[0] - prx) <= 1 && std::abs(r[1] - pry) <= 1 && r[2] - prz >= -1 && r[2] - prz <= 0;
			if (it != g_harvested.end() && !(isNear && now - it->second > kRefreshNear)) {
				continue;
			}
			if (!gathered) {
				const auto tg = Clock::now();
				RE::BSReadLockGuard lock(bhk->worldLock);
				GatherBodies(world);
				g_lastBodies = g_bodies.size();
				gathered = true;
				g_secGatherMs += std::chrono::duration<double, std::milli>(Clock::now() - tg).count();
			}
			{
				RE::BSReadLockGuard lock(bhk->worldLock);
				Harvest(r[0], r[1], r[2], a_origin);
			}
			g_harvested[k] = now;
			++g_secRegions;
			if (++done >= maxHarvests || Clock::now() - t0 > budget) {
				break;
			}
		}
		const double ms = std::chrono::duration<double, std::milli>(Clock::now() - t0).count();
		g_secHarvestMs += ms;
		g_worstFrameMs = std::max(g_worstFrameMs, ms);
		++g_secFrames;
	}

	void Reset()
	{
		Stop("game loaded");
	}

	bool SentAsGround(RE::COL_LAYER a_layer, RE::hkpMotion::MotionType a_motion)
	{
		// S2k-K1: moving bodies on the Props layer (moveable statics) are loose objects (Loose.cpp).
		if (a_layer == RE::COL_LAYER::kProps && a_motion != RE::hkpMotion::MotionType::kFixed) {
			return false;
		}
		return Included(a_layer);
	}

	uint32_t StreamEpoch() { return g_streaming ? g_colEpoch.load() : 0u; }

	void Post(uint32_t a_epoch, uint32_t a_type, std::vector<uint8_t> a_payload)
	{
		if (!g_streaming || a_epoch != g_colEpoch.load()) {
			return;
		}
		Job job{};
		job.colEpoch = a_epoch;
		job.rawType = a_type;
		job.raw = std::move(a_payload);
		std::scoped_lock lock(g_mutex);
		g_queue.push_back(std::move(job));
		g_cv.notify_one();
	}

	std::string RawText()
	{
		std::size_t queued = 0;
		{
			std::scoped_lock lock(g_mutex);
			for (const auto& j : g_queue) {
				queued += j.rawType != 0;
			}
		}
		return fmt::format("{:.0f} KB written, {} dropped (ring full), {} waiting in the queue", g_wRawBytes.exchange(0) / 1024.0, g_wRawDropped.exchange(0), queued);
	}

	void LogSecond()
	{
		if (!g_streaming) {
			if (g_secFrames == 0 && g_wRegions.load() == 0) {
				return;
			}
		}
		std::size_t queued = 0;
		{
			std::scoped_lock lock(g_mutex);
			queued = g_queue.size();
		}
		int known = 0;
		const int prx = g_prx.load(), pry = g_pry.load(), prz = g_prz.load();
		for (int dx = -kRadius; dx <= kRadius; ++dx) {
			for (int dy = -kRadius; dy <= kRadius; ++dy) {
				for (int dz = -kBelow; dz <= kAbove; ++dz) {
					known += g_harvested.contains(RegionKey(prx + dx, pry + dy, prz + dz));
				}
			}
		}
		const int window = (2 * kRadius + 1) * (2 * kRadius + 1) * (kBelow + kAbove + 1);
		const int64_t here = g_playerRegionTris.load();
		logger::info("COLLISION {}: harvested {} regions in {:.1f} ms over {} frames (slowest frame {:.2f} ms, gathering bodies {:.1f} ms, {} bodies), {} faults | sent {} regions, {} triangles, {:.0f} KB, worker {:.1f} ms, {} queued, ring {:.0f} KB behind, {} full waits, {} dropped | stair ramps: {} bodies, {} container children, {} triangles sent, {} child layer reads faulted | stair materials: {} triangles sent, {} material reads faulted | window {}/{} regions sent; player's region ({}, {}, {}): {}",
			g_streaming ? fmt::format("epoch {}", g_colEpoch.load()) : "stopped (" + g_stopReason + ")",
			g_secRegions, g_secHarvestMs, g_secFrames, g_worstFrameMs, g_secGatherMs, g_lastBodies, g_secFaults,
			g_wRegions.exchange(0), g_wTris.exchange(0), g_wBytes.exchange(0) / 1024.0, g_wUs.exchange(0) / 1000.0, queued,
			Link::Get().RingPending() / 1024.0, g_wFullWaits.exchange(0), g_wDropped.exchange(0),
			g_secRampBodies, g_secRampChildren, g_wRampTris.exchange(0), g_secFilterFaults, g_wStairTris.exchange(0), g_secMaterialFaults, known, window, prx, pry, prz, here < 0 ? std::string("not sent yet") : fmt::format("{} triangles at its last send", here));
		g_secRegions = g_secFaults = g_secFrames = g_secRampBodies = g_secRampChildren = g_secFilterFaults = g_secMaterialFaults = 0;
		g_secHarvestMs = g_secGatherMs = g_worstFrameMs = 0.0;
	}
}
