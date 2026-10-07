// S2k-K0: Skyrim's loose objects near the player, logged (DESIGN.md "S2k"). Nothing is changed.
// The shape walk uses the Havok layouts SkyCraft found on 1.7.104 (MIT, (c) 2026 chasmlol,
// skse/src/Collision.cpp), as our Collision.cpp does.
#include "Loose.h"

#include "Collision.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <map>
#include <string>
#include <vector>

namespace gmsky::Loose
{
	namespace
	{
		constexpr float kRange = 1500.0f;  // Skyrim units around the player
		constexpr int kListed = 40;        // bodies listed in the detail lines
		constexpr int kDetailEvery = 5;    // seconds
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
					if (Collision::SentAsGround(layer)) {
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
}
