// S2k: Skyrim's loose objects near the player (DESIGN.md "S2k"). K0: logged. K1: sent to Garry's
// Mod (shape once, then moves and removals) through the collision ring. The shape walks use the Havok layouts SkyCraft found on 1.7.104 (MIT, (c) 2026 chasmlol,
// skse/src/Collision.cpp), as our Collision.cpp does.
#include "Loose.h"

#include "Collision.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <map>
#include <string>
#include <unordered_map>
#include <vector>

namespace gmsky::Loose
{
	namespace
	{
		constexpr float kRange = 1500.0f;  // Skyrim units around the player
		constexpr int kListed = 40;        // bodies listed in the detail lines
		constexpr int kDetailEvery = 30;   // seconds (5 in K0)
		constexpr int kMaxKeys = 4096;     // children walked per container

		int g_seconds = 0;

		struct ShapeStats
		{
			int top{ -1 };
			int hulls{ 0 }, hullPlanesMax{ 0 }, boxes{ 0 }, capsules{ 0 }, spheres{ 0 }, triangles{ 0 };
			int meshes{ 0 }, lists{ 0 }, transforms{ 0 }, others{ 0 }, otherType{ -1 };
			bool truncated{ false };
		};

		template <class T>
		T Field(const void* a_base, std::size_t a_offset)
		{
			T value;
			std::memcpy(&value, reinterpret_cast<const std::uint8_t*>(a_base) + a_offset, sizeof(T));
			return value;
		}

		void WalkContainer(const RE::hkpShapeContainer* a_container, ShapeStats& a_st, int a_depth);

		void Walk(const RE::hkpShape* a_shape, ShapeStats& a_st, int a_depth)
		{
			if (!a_shape || a_depth > 8) {
				return;
			}
			using T = RE::hkpShapeType;
			const auto type = a_shape->type;
			if (a_depth == 0) {
				a_st.top = static_cast<int>(type);
			}
			switch (type) {
			case T::kMOPP:
			case T::kBVTree:
				++a_st.meshes;
				WalkContainer(static_cast<const RE::hkpBvTreeShape*>(a_shape)->GetContainer(), a_st, a_depth);
				return;
			case T::kCompressedMesh:
			case T::kExtendedMesh:
			case T::kTriangleCollection:
				++a_st.meshes;
				WalkContainer(a_shape->GetContainer(), a_st, a_depth);
				return;
			case T::kList:
			case T::kCollection:
			case T::kConvexList:
				++a_st.lists;
				WalkContainer(a_shape->GetContainer(), a_st, a_depth);
				return;
			case T::kTriangle:
				++a_st.triangles;
				return;
			case T::kBox:
				++a_st.boxes;
				return;
			case T::kCapsule:
				++a_st.capsules;
				return;
			case T::kSphere:
				++a_st.spheres;
				return;
			case T::kConvexVertices:
				{
					const auto& planes = *reinterpret_cast<const RE::hkArray<RE::hkVector4>*>(reinterpret_cast<const std::uint8_t*>(a_shape) + 0x78);
					++a_st.hulls;
					a_st.hullPlanesMax = std::max(a_st.hullPlanesMax, static_cast<int>(planes.size()));
					return;
				}
			case T::kConvexTransform:
			case T::kConvexTranslate:
				++a_st.transforms;
				Walk(Field<const RE::hkpShape*>(a_shape, 0x30), a_st, a_depth + 1);
				return;
			case T::kTransform:
				++a_st.transforms;
				Walk(Field<const RE::hkpShape*>(a_shape, 0x28), a_st, a_depth + 1);
				return;
			default:
				++a_st.others;
				a_st.otherType = static_cast<int>(type);
				return;
			}
		}

		void WalkContainer(const RE::hkpShapeContainer* a_container, ShapeStats& a_st, int a_depth)
		{
			if (!a_container) {
				return;
			}
			int n = 0;
			for (auto key = a_container->GetFirstKey(); key != RE::HK_INVALID_SHAPE_KEY; key = a_container->GetNextKey(key)) {
				if (++n > kMaxKeys) {
					a_st.truncated = true;
					return;
				}
				RE::hkpShapeBuffer buffer;
				Walk(a_container->GetChildShape(key, buffer), a_st, a_depth + 1);
			}
		}

		// Havok shape layouts are partly reverse-engineered: a bad read must not take down the game.
		bool GuardedWalk(const RE::hkpShape* a_shape, ShapeStats& a_st)
		{
			__try {
				Walk(a_shape, a_st, 0);
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		RE::TESObjectREFR* GuardedRef(const RE::hkpCollidable& a_collidable)
		{
			__try {
				return RE::TESHavokUtilities::FindCollidableRef(a_collidable);
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return nullptr;
			}
		}

		const char* MotionName(RE::hkpMotion::MotionType a_type)
		{
			using M = RE::hkpMotion::MotionType;
			switch (a_type) {
			case M::kDynamic: return "dynamic";
			case M::kSphereInertia: return "sphere-inertia";
			case M::kBoxInertia: return "box-inertia";
			case M::kKeyframed: return "keyframed";
			case M::kFixed: return "fixed";
			case M::kThinBoxInertia: return "thin-box-inertia";
			case M::kCharacter: return "character";
			default: return "invalid";
			}
		}

		// Actors' own bodies: counted, never listed (they are the actor stand-ins' business).
		bool ActorLayer(RE::COL_LAYER a_layer)
		{
			using L = RE::COL_LAYER;
			return a_layer == L::kCharController || a_layer == L::kBiped || a_layer == L::kBipedNoCC || a_layer == L::kDeadBip ||
			       a_layer == L::kCameraSphere;
		}

		std::string ShapeText(const ShapeStats& a_st)
		{
			std::string s = fmt::format("top {}", a_st.top);
			auto add = [&](const char* a_name, int a_n) {
				if (a_n > 0) {
					s += fmt::format(", {} {}", a_n, a_name);
				}
			};
			add("hulls", a_st.hulls);
			if (a_st.hulls > 0) {
				s += fmt::format(" (up to {} planes)", a_st.hullPlanesMax);
			}
			add("boxes", a_st.boxes);
			add("capsules", a_st.capsules);
			add("spheres", a_st.spheres);
			add("meshes", a_st.meshes);
			add("mesh triangles", a_st.triangles);
			add("lists", a_st.lists);
			add("transforms", a_st.transforms);
			if (a_st.others > 0) {
				s += fmt::format(", {} other (type {})", a_st.others, a_st.otherType);
			}
			if (a_st.truncated) {
				s += ", truncated";
			}
			return s;
		}

		struct Seen
		{
			float dist;
			std::string text;
		};
	}

	void LogSecond(RE::PlayerCharacter* a_player)
	{
		auto* cell = a_player ? a_player->GetParentCell() : nullptr;
		auto* bhk = cell ? cell->GetbhkWorld() : nullptr;
		auto* world = bhk ? bhk->GetWorld1() : nullptr;
		if (!world) {
			return;
		}
		const bool detail = (++g_seconds % kDetailEvery) == 0;
		const auto t0 = std::chrono::steady_clock::now();
		const float k = RE::bhkWorld::GetWorldScaleInverse();
		const auto pos = a_player->GetPosition();

		std::map<std::string, int> byKind;   // "layer/motion" -> count
		std::map<std::string, int> baked;    // moving bodies on layers sent as static ground
		std::vector<Seen> listed;
		int bodies = 0, actorBodies = 0, faults = 0;

		{
			RE::BSReadLockGuard lock(bhk->worldLock);
			auto visit = [&](RE::hkpSimulationIsland* a_island, const char* a_islandName) {
				if (!a_island) {
					return;
				}
				auto& entities = a_island->entities;
				for (std::int32_t i = 0; i < entities.size(); ++i) {
					auto* entity = entities.data()[i];
					if (!entity) {
						continue;
					}
					const auto motion = entity->motion.type.get();
					if (motion == RE::hkpMotion::MotionType::kFixed) {
						continue;
					}
					const auto& collidable = entity->collidable;
					const auto* xf = static_cast<const float*>(collidable.motion);
					if (!xf) {
						continue;
					}
					const float p[3] = { xf[12] * k, xf[13] * k, xf[14] * k };
					const float dx = p[0] - pos.x, dy = p[1] - pos.y, dz = p[2] - pos.z;
					const float dist = std::sqrt(dx * dx + dy * dy + dz * dz);
					if (!std::isfinite(dist) || dist > kRange) {
						continue;
					}
					const auto layer = collidable.GetCollisionLayer();
					const std::string layerName{ RE::CollisionLayerToString(layer) };
					++bodies;
					++byKind[layerName + "/" + MotionName(motion)];
					if (Collision::SentAsGround(layer, motion)) {
						++baked[layerName + "/" + MotionName(motion)];
					}
					if (ActorLayer(layer)) {
						++actorBodies;
						continue;
					}
					if (!detail) {
						continue;
					}
					ShapeStats st;
					if (!GuardedWalk(collidable.shape, st)) {
						++faults;
					}
					auto* ref = GuardedRef(collidable);
					auto* base = ref ? ref->GetBaseObject() : nullptr;
					const char* name = ref ? ref->GetName() : nullptr;
					alignas(16) float v[4];
					_mm_store_ps(v, entity->motion.linearVelocity.quad);
					const float speed = std::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]) * k;
					listed.push_back({ dist, fmt::format("dist {:.0f} at ({:.0f}, {:.0f}, {:.0f}) ref {:08X} base {:08X} {} '{}' | {} {} {} | mass {:.2f} speed {:.0f} | {}",
						dist, p[0], p[1], p[2], ref ? ref->GetFormID() : 0u, base ? base->GetFormID() : 0u,
						base ? RE::FormTypeToString(base->GetFormType()) : std::string_view{ "-" }, name ? name : "",
						layerName, MotionName(motion), a_islandName, entity->motion.GetMass(), speed, ShapeText(st)) });
				}
			};
			for (std::int32_t i = 0; i < world->activeSimulationIslands.size(); ++i) {
				visit(world->activeSimulationIslands.data()[i], "awake");
			}
			for (std::int32_t i = 0; i < world->inactiveSimulationIslands.size(); ++i) {
				visit(world->inactiveSimulationIslands.data()[i], "asleep");
			}
			visit(world->fixedIsland, "fixed-island");
		}

		auto join = [](const std::map<std::string, int>& a_m) {
			std::string s;
			for (const auto& [name, n] : a_m) {
				s += fmt::format("{}{} {}", s.empty() ? "" : ", ", name, n);
			}
			return s.empty() ? std::string("none") : s;
		};
		const double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
		logger::info("LOOSE: {} bodies not fixed within {:.0f} ({} are actors' own) | {} | baked into the ground but not fixed: {} | {:.2f} ms{}",
			bodies, kRange, actorBodies, join(byKind), join(baked), ms, faults ? fmt::format(", {} shape reads faulted", faults) : "");
		if (detail) {
			std::ranges::sort(listed, {}, &Seen::dist);
			const int n = std::min<int>(kListed, static_cast<int>(listed.size()));
			for (int i = 0; i < n; ++i) {
				logger::info("LOOSE  [{}] {}", i, listed[i].text);
			}
			if (static_cast<int>(listed.size()) > n) {
				logger::info("LOOSE  ... {} more not listed", listed.size() - n);
			}
		}
	}

	// ---- S2k-K1: the loose objects sent to Garry's Mod ---------------------------------------------
	namespace
	{
		constexpr float kSendRange = 1500.0f;  // Skyrim units: a body comes within this
		constexpr float kKeepRange = 1700.0f;  // ... and is kept until it leaves this
		constexpr std::size_t kMax = 2000;     // nearest bodies sent (the user's number)
		constexpr float kScanEvery = 0.1f;     // seconds
		constexpr double kNewBudgetMs = 2.0;   // shapes read per scan until this much time is spent
		constexpr int kMaxPieces = 256;
		constexpr std::size_t kMaxPoints = 4096;
		constexpr float kMoveEps = 0.25f;      // Skyrim units: a smaller move isn't sent
		constexpr float kTurnEps = 0.002f;     // rotation matrix entries

		using Piece = std::vector<std::array<float, 3>>;  // Havok units, the body's frame
		struct Decoded
		{
			std::vector<Piece> pieces;
			std::size_t points{ 0 };
			int meshTris{ 0 };   // triangles of a mesh (not convex: left out)
			int fallbacks{ 0 };  // unknown convex shapes sent as their box
			bool full{ false };
		};

		struct Sent
		{
			const RE::hkpShape* shape{ nullptr };  // a new body at a reused address is a new object
			float pos[3]{};                       // Skyrim units
			float rot[9]{};
			bool sent{ false };                   // false: it has no convex pieces, nothing sent
			uint64_t seen{ 0 };
		};

		std::unordered_map<uint64_t, Sent> g_sent;
		uint32_t g_epoch = 0;
		float g_timer = 0.0f;
		uint64_t g_scan = 0;

		struct Second
		{
			int scans{ 0 }, inRange{ 0 }, cut{ 0 }, added{ 0 }, moved{ 0 }, gone{ 0 }, noPieces{ 0 }, faults{ 0 }, restarts{ 0 };
			int waiting{ 0 }, meshTris{ 0 }, fallbacks{ 0 }, full{ 0 };
			std::size_t pieces{ 0 }, points{ 0 };
			double ms{ 0 }, worstMs{ 0 };
		} g_sec;

		const float* V(const void* a_base, std::size_t a_offset)
		{
			return reinterpret_cast<const float*>(reinterpret_cast<const uint8_t*>(a_base) + a_offset);
		}
		void XfPt(const float* a_xf, const float* a_p, float* a_out)
		{
			for (int i = 0; i < 3; ++i) {
				a_out[i] = a_xf[i] * a_p[0] + a_xf[4 + i] * a_p[1] + a_xf[8 + i] * a_p[2] + a_xf[12 + i];
			}
		}
		void XfMul(const float* a_parent, const float* a_child, float* a_out)
		{
			for (int c = 0; c < 3; ++c) {
				for (int i = 0; i < 3; ++i) {
					const float* d = a_child + c * 4;
					a_out[c * 4 + i] = a_parent[i] * d[0] + a_parent[4 + i] * d[1] + a_parent[8 + i] * d[2];
				}
				a_out[c * 4 + 3] = 0.0f;
			}
			XfPt(a_parent, a_child + 12, a_out + 12);
			a_out[15] = 1.0f;
		}
		bool XfOk(const float* a_xf)
		{
			for (int i = 0; i < 16; ++i) {
				if (!std::isfinite(a_xf[i]) || std::fabs(a_xf[i]) > 1.0e7f) {
					return false;
				}
			}
			for (int c = 0; c < 3; ++c) {
				const float* col = a_xf + c * 4;
				if (std::fabs(col[0] * col[0] + col[1] * col[1] + col[2] * col[2] - 1.0f) > 0.05f) {
					return false;
				}
			}
			return true;
		}

		// 26 directions: the cube's faces, edges and corners (a capsule or sphere as points on it).
		const std::vector<std::array<float, 3>>& Dirs()
		{
			static const auto dirs = [] {
				std::vector<std::array<float, 3>> d;
				for (int x = -1; x <= 1; ++x) {
					for (int y = -1; y <= 1; ++y) {
						for (int z = -1; z <= 1; ++z) {
							if (x || y || z) {
								const float len = std::sqrt(float(x * x + y * y + z * z));
								d.push_back({ x / len, y / len, z / len });
							}
						}
					}
				}
				return d;
			}();
			return dirs;
		}

		void AddPiece(Decoded& a_out, Piece&& a_piece)
		{
			if (a_piece.size() < 4) {
				return;  // no volume
			}
			if (static_cast<int>(a_out.pieces.size()) >= kMaxPieces || a_out.points + a_piece.size() > kMaxPoints) {
				a_out.full = true;
				return;
			}
			a_out.points += a_piece.size();
			a_out.pieces.push_back(std::move(a_piece));
		}

		void Decode(const RE::hkpShape* a_shape, const float* a_xf, Decoded& a_out, int a_depth);

		void DecodeContainer(const RE::hkpShapeContainer* a_container, const float* a_xf, Decoded& a_out, int a_depth)
		{
			if (!a_container) {
				return;
			}
			int n = 0;
			for (auto key = a_container->GetFirstKey(); key != RE::HK_INVALID_SHAPE_KEY && !a_out.full; key = a_container->GetNextKey(key)) {
				if (++n > kMaxKeys) {
					a_out.full = true;
					return;
				}
				RE::hkpShapeBuffer buffer;
				Decode(a_container->GetChildShape(key, buffer), a_xf, a_out, a_depth + 1);
			}
		}

		void Decode(const RE::hkpShape* a_shape, const float* a_xf, Decoded& a_out, int a_depth)
		{
			if (!a_shape || a_depth > 8 || a_out.full) {
				return;
			}
			using T = RE::hkpShapeType;
			const auto type = a_shape->type;
			switch (type) {
			case T::kMOPP:
			case T::kBVTree:
				DecodeContainer(static_cast<const RE::hkpBvTreeShape*>(a_shape)->GetContainer(), a_xf, a_out, a_depth);
				return;
			case T::kList:
			case T::kCollection:
			case T::kConvexList:
			case T::kCompressedMesh:
			case T::kExtendedMesh:
			case T::kTriangleCollection:
				DecodeContainer(a_shape->GetContainer(), a_xf, a_out, a_depth);
				return;
			case T::kTriangle:
				++a_out.meshTris;
				return;
			case T::kBox:
				{
					const float* half = V(a_shape, 0x30);
					const float r = Field<float>(a_shape, 0x20);
					Piece piece;
					for (int c = 0; c < 8; ++c) {
						const float p[3] = { (c & 1 ? 1 : -1) * (half[0] + r), (c & 2 ? 1 : -1) * (half[1] + r), (c & 4 ? 1 : -1) * (half[2] + r) };
						std::array<float, 3> w;
						XfPt(a_xf, p, w.data());
						piece.push_back(w);
					}
					AddPiece(a_out, std::move(piece));
					return;
				}
			case T::kCapsule:
			case T::kSphere:
				{
					const float r = Field<float>(a_shape, 0x20);
					const float zero[3] = { 0, 0, 0 };
					const float* ends[2] = { type == T::kCapsule ? V(a_shape, 0x30) : zero, type == T::kCapsule ? V(a_shape, 0x40) : zero };
					Piece piece;
					for (int e = 0; e < (type == T::kCapsule ? 2 : 1); ++e) {
						for (const auto& d : Dirs()) {
							const float p[3] = { ends[e][0] + d[0] * r, ends[e][1] + d[1] * r, ends[e][2] + d[2] * r };
							std::array<float, 3> w;
							XfPt(a_xf, p, w.data());
							piece.push_back(w);
						}
					}
					AddPiece(a_out, std::move(piece));
					return;
				}
			case T::kConvexVertices:
				{
					// CommonLib's layout (RE/H/hkpConvexVerticesShape.h): the corner points, four
					// at a time with x, y and z each in one vector.
					const auto* hull = static_cast<const RE::hkpConvexVerticesShape*>(a_shape);
					const int n = hull->numVertices;
					const float r = Field<float>(a_shape, 0x20);
					if (n < 4 || n > 1024 || hull->rotatedVertices.size() * 4 < n) {
						return;
					}
					std::vector<std::array<float, 3>> local(n);
					float c[3] = { 0, 0, 0 };
					for (int i = 0; i < n; ++i) {
						const auto& four = hull->rotatedVertices.data()[i / 4];
						alignas(16) float x[4], y[4], z[4];
						_mm_store_ps(x, four.x.quad);
						_mm_store_ps(y, four.y.quad);
						_mm_store_ps(z, four.z.quad);
						local[i] = { x[i % 4], y[i % 4], z[i % 4] };
						for (int k = 0; k < 3; ++k) {
							c[k] += local[i][k] / n;
						}
					}
					Piece piece;
					for (auto& p : local) {
						// The convex radius: Havok keeps this shell around the corner points.
						const float d[3] = { p[0] - c[0], p[1] - c[1], p[2] - c[2] };
						const float len = std::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
						if (len > 1e-6f) {
							for (int k = 0; k < 3; ++k) {
								p[k] += d[k] / len * r;
							}
						}
						std::array<float, 3> w;
						XfPt(a_xf, p.data(), w.data());
						piece.push_back(w);
					}
					AddPiece(a_out, std::move(piece));
					return;
				}
			case T::kConvexTransform:
			case T::kConvexTranslate:
				{
					const auto* child = Field<const RE::hkpShape*>(a_shape, 0x30);
					alignas(16) float local[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 };
					if (type == T::kConvexTransform) {
						std::memcpy(local, V(a_shape, 0x40), sizeof(local));
					} else {
						std::memcpy(local + 12, V(a_shape, 0x40), sizeof(float) * 3);
					}
					if (!child || !XfOk(local)) {
						return;
					}
					alignas(16) float composed[16];
					XfMul(a_xf, local, composed);
					Decode(child, composed, a_out, a_depth + 1);
					return;
				}
			case T::kTransform:
				{
					const auto* child = Field<const RE::hkpShape*>(a_shape, 0x28);
					alignas(16) float local[16];
					std::memcpy(local, V(a_shape, 0x50), sizeof(local));
					if (!child || !XfOk(local)) {
						return;
					}
					alignas(16) float composed[16];
					XfMul(a_xf, local, composed);
					Decode(child, composed, a_out, a_depth + 1);
					return;
				}
			default:
				if (a_shape->IsConvex()) {
					// Unknown convex shape: its box in the body's frame.
					static const float identity[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 };
					RE::hkAabb box;
					a_shape->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(identity), 0.0f, box);
					alignas(16) float lo[4], hi[4];
					_mm_store_ps(lo, box.min.quad);
					_mm_store_ps(hi, box.max.quad);
					Piece piece;
					for (int c = 0; c < 8; ++c) {
						const float p[3] = { c & 1 ? hi[0] : lo[0], c & 2 ? hi[1] : lo[1], c & 4 ? hi[2] : lo[2] };
						std::array<float, 3> w;
						XfPt(a_xf, p, w.data());
						piece.push_back(w);
					}
					++a_out.fallbacks;
					AddPiece(a_out, std::move(piece));
				}
				return;
			}
		}

		bool GuardedDecode(const RE::hkpShape* a_shape, Decoded& a_out)
		{
			static const float identity[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 };
			__try {
				Decode(a_shape, identity, a_out, 0);
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		bool LooseLayer(RE::COL_LAYER a_layer)
		{
			return a_layer == RE::COL_LAYER::kClutter || a_layer == RE::COL_LAYER::kWeapon || a_layer == RE::COL_LAYER::kProps;
		}

		struct Cand
		{
			RE::hkpEntity* entity;
			float dist;
			bool asleep;
			RE::TESObjectREFR* ref{ nullptr };
		};

		std::vector<uint8_t> Message(const GmskyColLoose& a_h, const Decoded* a_shape, float a_scale)
		{
			std::vector<uint8_t> out(sizeof(a_h));
			std::memcpy(out.data(), &a_h, sizeof(a_h));
			if (a_shape) {
				for (const auto& p : a_shape->pieces) {
					const auto n = static_cast<uint32_t>(p.size());
					const auto* b = reinterpret_cast<const uint8_t*>(&n);
					out.insert(out.end(), b, b + 4);
				}
				for (const auto& p : a_shape->pieces) {
					for (const auto& q : p) {
						const float g[3] = { q[0] * a_scale, q[1] * a_scale, q[2] * a_scale };
						const auto* b = reinterpret_cast<const uint8_t*>(g);
						out.insert(out.end(), b, b + sizeof(g));
					}
				}
			}
			return out;
		}

		std::vector<uint8_t> GoneMessage(uint64_t a_id, uint32_t a_reason)
		{
			GmskyColLooseGone g{};
			g.id = a_id;
			g.col_epoch = g_epoch;
			g.reason = a_reason;
			std::vector<uint8_t> out(sizeof(g));
			std::memcpy(out.data(), &g, sizeof(g));
			return out;
		}
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, float a_delta)
	{
		const uint32_t epoch = Collision::StreamEpoch();
		if (epoch == 0 || a_origin.epoch == 0 || !a_player) {
			g_sent.clear();
			g_epoch = 0;
			return;
		}
		if (epoch != g_epoch) {
			// A new stream (start, new origin): Garry's Mod dropped everything with its "clear".
			g_sent.clear();
			g_epoch = epoch;
			++g_sec.restarts;
		}
		g_timer += a_delta;
		if (g_timer < kScanEvery) {
			return;
		}
		g_timer = 0.0f;
		auto* cell = a_player->GetParentCell();
		auto* bhk = cell ? cell->GetbhkWorld() : nullptr;
		auto* world = bhk ? bhk->GetWorld1() : nullptr;
		if (!world) {
			return;
		}
		const auto t0 = std::chrono::steady_clock::now();
		const float k = RE::bhkWorld::GetWorldScaleInverse();  // Skyrim units per Havok unit
		const auto pos = a_player->GetPosition();
		const uint64_t scan = ++g_scan;
		std::vector<std::pair<uint32_t, std::vector<uint8_t>>> out;  // (type, payload)

		{
			RE::BSReadLockGuard lock(bhk->worldLock);
			std::vector<Cand> cands;
			auto visit = [&](RE::hkpSimulationIsland* a_island, bool a_asleep) {
				if (!a_island) {
					return;
				}
				auto& entities = a_island->entities;
				for (std::int32_t i = 0; i < entities.size(); ++i) {
					auto* entity = entities.data()[i];
					if (!entity) {
						continue;
					}
					const auto motion = entity->motion.type.get();
					if (motion == RE::hkpMotion::MotionType::kFixed || motion == RE::hkpMotion::MotionType::kCharacter ||
						motion == RE::hkpMotion::MotionType::kInvalid || !LooseLayer(entity->collidable.GetCollisionLayer())) {
						continue;
					}
					const auto* xf = static_cast<const float*>(entity->collidable.motion);
					if (!xf || !entity->collidable.shape) {
						continue;
					}
					const float dx = xf[12] * k - pos.x, dy = xf[13] * k - pos.y, dz = xf[14] * k - pos.z;
					const float dist = std::sqrt(dx * dx + dy * dy + dz * dz);
					const auto it = g_sent.find(reinterpret_cast<uint64_t>(entity));
					if (std::isfinite(dist) && (dist <= kSendRange || (it != g_sent.end() && dist <= kKeepRange))) {
						cands.push_back({ entity, dist, a_asleep });
					}
				}
			};
			for (std::int32_t i = 0; i < world->activeSimulationIslands.size(); ++i) {
				visit(world->activeSimulationIslands.data()[i], false);
			}
			for (std::int32_t i = 0; i < world->inactiveSimulationIslands.size(); ++i) {
				visit(world->inactiveSimulationIslands.data()[i], true);
			}
			std::ranges::sort(cands, {}, &Cand::dist);
			g_sec.inRange = static_cast<int>(cands.size());
			if (cands.size() > kMax) {
				g_sec.cut = static_cast<int>(cands.size() - kMax);
				cands.resize(kMax);
			}
			// Bodies per reference: more than one = joined (hanging pots, chains).
			std::unordered_map<RE::TESObjectREFR*, int> perRef;
			for (auto& c : cands) {
				c.ref = GuardedRef(c.entity->collidable);
				if (c.ref) {
					++perRef[c.ref];
				}
			}

			const double s = kGmodPerSkyrim;
			for (const auto& c : cands) {
				auto* entity = c.entity;
				const auto id = reinterpret_cast<uint64_t>(entity);
				const auto* xf = static_cast<const float*>(entity->collidable.motion);
				const float p[3] = { xf[12] * k, xf[13] * k, xf[14] * k };
				const float rot[9] = { xf[0], xf[1], xf[2], xf[4], xf[5], xf[6], xf[8], xf[9], xf[10] };
				auto it = g_sent.find(id);
				if (it != g_sent.end() && it->second.shape != entity->collidable.shape) {
					// Another body at a freed address: the old one is gone.
					if (it->second.sent) {
						out.emplace_back(GMSKY_COL_LOOSE_GONE, GoneMessage(id, GMSKY_LOOSE_GONE_LEFT));
						++g_sec.gone;
					}
					g_sent.erase(it);
					it = g_sent.end();
				}
				const bool isNew = it == g_sent.end();
				if (!isNew) {
					it->second.seen = scan;
					if (!it->second.sent) {
						continue;
					}
					bool moved = false;
					for (int i = 0; i < 3 && !moved; ++i) {
						moved = std::fabs(p[i] - it->second.pos[i]) > kMoveEps;
					}
					for (int i = 0; i < 9 && !moved; ++i) {
						moved = std::fabs(rot[i] - it->second.rot[i]) > kTurnEps;
					}
					if (!moved) {
						continue;
					}
				} else if (std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count() > kNewBudgetMs) {
					++g_sec.waiting;  // next scan
					continue;
				}
				GmskyColLoose h{};
				h.id = id;
				h.col_epoch = epoch;
				h.form_id = c.ref ? c.ref->GetFormID() : 0u;
				const auto motion = entity->motion.type.get();
				h.flags = (motion == RE::hkpMotion::MotionType::kKeyframed ? GMSKY_LOOSE_KEYFRAMED : 0u) |
				          (c.ref && perRef[c.ref] > 1 ? GMSKY_LOOSE_JOINED : 0u) | (c.asleep ? GMSKY_LOOSE_ASLEEP : 0u);
				h.layer = static_cast<uint16_t>(entity->collidable.GetCollisionLayer());
				h.motion = static_cast<uint8_t>(motion);
				for (int i = 0; i < 3; ++i) {
					h.pos[i] = float((double(p[i]) - a_origin.skyrim_origin[i]) * s);
				}
				std::memcpy(h.rot, rot, sizeof(rot));
				alignas(16) float v[4];
				_mm_store_ps(v, entity->motion.linearVelocity.quad);
				for (int i = 0; i < 3; ++i) {
					h.vel[i] = float(v[i] * k * s);
				}
				h.mass = entity->motion.GetMass();
				if (isNew) {
					Decoded shape;
					Sent rec{};
					rec.shape = entity->collidable.shape;
					rec.seen = scan;
					std::memcpy(rec.pos, p, sizeof(p));
					std::memcpy(rec.rot, rot, sizeof(rot));
					if (!GuardedDecode(entity->collidable.shape, shape)) {
						++g_sec.faults;
					} else if (!shape.pieces.empty()) {
						h.flags |= GMSKY_LOOSE_SHAPE;
						h.pieces = static_cast<uint32_t>(shape.pieces.size());
						h.points = static_cast<uint32_t>(shape.points);
						out.emplace_back(GMSKY_COL_LOOSE, Message(h, &shape, float(k * s)));
						rec.sent = true;
						++g_sec.added;
						g_sec.pieces += shape.pieces.size();
						g_sec.points += shape.points;
					}
					if (!rec.sent) {
						++g_sec.noPieces;
					}
					g_sec.meshTris += shape.meshTris;
					g_sec.fallbacks += shape.fallbacks;
					g_sec.full += shape.full;
					g_sent.emplace(id, rec);
				} else {
					std::memcpy(it->second.pos, p, sizeof(p));
					std::memcpy(it->second.rot, rot, sizeof(rot));
					out.emplace_back(GMSKY_COL_LOOSE, Message(h, nullptr, 0.0f));
					++g_sec.moved;
				}
			}
		}
		// Bodies not seen in this scan: gone (out of range, cut by the limit, or out of Havok's world).
		for (auto it = g_sent.begin(); it != g_sent.end();) {
			if (it->second.seen != scan) {
				if (it->second.sent) {
					out.emplace_back(GMSKY_COL_LOOSE_GONE, GoneMessage(it->first, GMSKY_LOOSE_GONE_LEFT));
					++g_sec.gone;
				}
				it = g_sent.erase(it);
			} else {
				++it;
			}
		}
		for (auto& [type, payload] : out) {
			Collision::Post(epoch, type, std::move(payload));
		}
		const double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
		++g_sec.scans;
		g_sec.ms += ms;
		g_sec.worstMs = std::max(g_sec.worstMs, ms);
	}

	void LogSending()
	{
		if (g_sec.scans == 0 && g_sent.empty()) {
			return;
		}
		std::size_t sent = 0;
		for (const auto& [id, rec] : g_sent) {
			sent += rec.sent;
		}
		logger::info("LOOSE K1: {} scans, {:.1f} ms (slowest {:.2f}); {} bodies in range, {} past the {} nearest; {} known to Garry's Mod | this second: {} added ({} pieces, {} points), {} moved, {} gone, {} waiting for the next scan, {} with no convex pieces ({} mesh triangles left out), {} unknown shapes sent as boxes, {} over the piece limit, {} reads faulted, {} restarts | messages: {}",
			g_sec.scans, g_sec.ms, g_sec.worstMs, g_sec.inRange, g_sec.cut, kMax, sent, g_sec.added, g_sec.pieces, g_sec.points, g_sec.moved, g_sec.gone,
			g_sec.waiting, g_sec.noPieces, g_sec.meshTris, g_sec.fallbacks, g_sec.full, g_sec.faults, g_sec.restarts, Collision::RawText());
		g_sec = Second{};
	}
}
