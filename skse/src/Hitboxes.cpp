// Phase 3 step 3c: Skyrim's NPC hitboxes for Garry's Mod (DESIGN.md "Hitboxes: how bullets hit
// Skyrim's NPCs").
//
// Every collision body in an actor's 3D is read (the walk over the scene graph's collision objects
// is SkyCraft's, Combat.cpp FlingRagdoll, MIT; the Havok layouts are the ones Collision.cpp already
// uses): the bone that owns it, its physics layer, shape, motion kind, size, whether it is in
// Skyrim's physics world. Test 56 (3c-1): people have 19 capsules, wolves 22, all following the
// animation.
// - 3c-2: the nearest GMSKY_HB_MAX_ACTORS actors (living and dead) go to Garry's Mod every frame
//   (GmskyHitboxes), each part with Garry's Mod's hit group worked out from its bone's name.
// - Once per NPC kind (race), alive and dead: the full list of parts with their groups.
// - Every frame, for the nearest three living NPCs and the nearest corpse: each part's centre in
//   its bone's own frame (stays the same while the bones move = the parts follow the animation).
#include "Hitboxes.h"

#include "Link.h"

#include <algorithm>
#include <set>
#include <string>
#include <unordered_map>
#include <vector>

namespace gmsky::Hitboxes
{
	namespace
	{
		constexpr float kRange = 3000.0f;  // Skyrim units
		constexpr int kLiving = 3;         // drift measurement
		constexpr double kNoAnchor[3] = { 0.0, 0.0, 0.0 };

		// One collision body, read under SEH (Havok layouts are partly reverse-engineered).
		struct PartRead
		{
			int layer{ -1 };
			int shapeType{ -1 };
			int motionType{ -1 };
			bool inWorld{ false };
			float lo[3]{}, hi[3]{};  // world box, Skyrim units
			bool capsule{ false };
			float a[3]{}, b[3]{}, r{ 0 };  // capsule / sphere, Skyrim units
		};

		struct Part
		{
			std::string bone;
			PartRead read;
			uint8_t group{ GMSKY_HBG_GENERIC };
			RE::NiPoint3 centre;  // world, Skyrim units
			RE::NiPoint3 local;   // centre in the bone's own frame
			RE::NiPoint3 bonePos;
		};

		void XfPoint(const float* a_xf, const float* a_p, float* a_out)
		{
			for (int i = 0; i < 3; ++i) {
				a_out[i] = a_xf[i] * a_p[0] + a_xf[4 + i] * a_p[1] + a_xf[8 + i] * a_p[2] + a_xf[12 + i];
			}
		}

		bool ReadPart(const RE::hkpRigidBody* a_body, float a_k, PartRead& a_out)
		{
			__try {
				const auto& collidable = a_body->collidable;
				a_out.layer = static_cast<int>(collidable.GetCollisionLayer());
				a_out.inWorld = a_body->world != nullptr;
				a_out.motionType = static_cast<int>(a_body->motion.type.get());
				const auto* shape = collidable.shape;
				const auto* xf = static_cast<const float*>(collidable.motion);
				if (!shape || !xf) {
					return false;
				}
				a_out.shapeType = static_cast<int>(shape->type);
				RE::hkAabb box;
				shape->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(xf), 0.0f, box);
				alignas(16) float mn[4], mx[4];
				_mm_store_ps(mn, box.min.quad);
				_mm_store_ps(mx, box.max.quad);
				for (int i = 0; i < 3; ++i) {
					a_out.lo[i] = mn[i] * a_k;
					a_out.hi[i] = mx[i] * a_k;
				}
				if (shape->type == RE::hkpShapeType::kCapsule || shape->type == RE::hkpShapeType::kSphere) {
					const auto* base = reinterpret_cast<const uint8_t*>(shape);
					float radius;
					std::memcpy(&radius, base + 0x20, sizeof(float));
					const float zero[3] = { 0, 0, 0 };
					float w[3];
					const bool cap = shape->type == RE::hkpShapeType::kCapsule;
					XfPoint(xf, cap ? reinterpret_cast<const float*>(base + 0x30) : zero, w);
					for (int i = 0; i < 3; ++i) a_out.a[i] = w[i] * a_k;
					XfPoint(xf, cap ? reinterpret_cast<const float*>(base + 0x40) : zero, w);
					for (int i = 0; i < 3; ++i) a_out.b[i] = w[i] * a_k;
					a_out.r = radius * a_k;
					a_out.capsule = true;
				}
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		bool Has(const std::string& a_s, const char* a_part)
		{
			return a_s.find(a_part) != std::string::npos;
		}

		// Left / right from the bone's name: "NPC L Thigh [LThg]", "Canine_LFrontLeg1", "... Left ...".
		int Side(const std::string& a_b)
		{
			if (Has(a_b, " L ") || Has(a_b, "_L") || Has(a_b, "Left") || Has(a_b, "[L")) return -1;
			if (Has(a_b, " R ") || Has(a_b, "_R") || Has(a_b, "Right") || Has(a_b, "[R")) return 1;
			return 0;
		}

		// Garry's Mod's hit group for a bone (DESIGN.md "3c-2 in detail"). Logged per NPC kind.
		uint8_t GroupOf(const std::string& a_bone)
		{
			for (const char* gear : { "Bow", "Weapon", "Shield", "Quiver", "Arrow", "Sword", "Dagger", "Axe", "Mace", "Staff", "Scb", "Magic" }) {
				if (Has(a_bone, gear)) return GMSKY_HBG_GEAR;
			}
			if (Has(a_bone, "Head") || Has(a_bone, "Skull") || Has(a_bone, "Jaw")) return GMSKY_HBG_HEAD;
			if (Has(a_bone, "Tail")) return GMSKY_HBG_GENERIC;
			const int side = Side(a_bone);
			const bool arm = Has(a_bone, "Arm") || Has(a_bone, "arm") || Has(a_bone, "Hand") || Has(a_bone, "Clavicle") || Has(a_bone, "Finger") || Has(a_bone, "Front");
			const bool leg = Has(a_bone, "Thigh") || Has(a_bone, "Calf") || Has(a_bone, "Foot") || Has(a_bone, "Toe") || Has(a_bone, "Leg") || Has(a_bone, "Palm");
			if (arm && side != 0) return side < 0 ? GMSKY_HBG_LEFTARM : GMSKY_HBG_RIGHTARM;
			if (leg && side != 0) return side < 0 ? GMSKY_HBG_LEFTLEG : GMSKY_HBG_RIGHTLEG;
			if (Has(a_bone, "Neck") || Has(a_bone, "Spine2") || Has(a_bone, "Spine3") || Has(a_bone, "Chest") || Has(a_bone, "Ribcage")) return GMSKY_HBG_CHEST;
			if (Has(a_bone, "Spine") || Has(a_bone, "COM") || Has(a_bone, "Pelvis") || Has(a_bone, "Root")) return GMSKY_HBG_STOMACH;
			return GMSKY_HBG_GENERIC;
		}

		const char* GroupName(uint8_t a_g)
		{
			switch (a_g) {
			case GMSKY_HBG_HEAD: return "head";
			case GMSKY_HBG_CHEST: return "chest";
			case GMSKY_HBG_STOMACH: return "stomach";
			case GMSKY_HBG_LEFTARM: return "left arm";
			case GMSKY_HBG_RIGHTARM: return "right arm";
			case GMSKY_HBG_LEFTLEG: return "left leg";
			case GMSKY_HBG_RIGHTLEG: return "right leg";
			case GMSKY_HBG_GEAR: return "gear";
			default: return "generic";
			}
		}

		const char* LayerName(int a_layer)
		{
			switch (static_cast<RE::COL_LAYER>(a_layer)) {
			case RE::COL_LAYER::kBiped: return "BIPED";
			case RE::COL_LAYER::kBipedNoCC: return "BIPED_NO_CC";
			case RE::COL_LAYER::kDeadBip: return "DEADBIP";
			case RE::COL_LAYER::kCharController: return "CHARCONTROLLER";
			case RE::COL_LAYER::kWeapon: return "WEAPON";
			case RE::COL_LAYER::kClutter: return "CLUTTER";
			case RE::COL_LAYER::kProps: return "PROPS";
			default: return "other";
			}
		}

		const char* ShapeName(int a_type)
		{
			using T = RE::hkpShapeType;
			switch (static_cast<T>(a_type)) {
			case T::kCapsule: return "capsule";
			case T::kSphere: return "sphere";
			case T::kBox: return "box";
			case T::kConvexVertices: return "hull";
			case T::kConvexTransform: return "hull (transformed)";
			case T::kConvexTranslate: return "hull (moved)";
			case T::kList: return "list";
			case T::kMOPP: return "mesh (MOPP)";
			default: return "other";
			}
		}

		const char* MotionName(int a_type)
		{
			using M = RE::hkpMotion::MotionType;
			switch (static_cast<M>(a_type)) {
			case M::kKeyframed: return "keyframed (driven by animation)";
			case M::kFixed: return "fixed";
			case M::kCharacter: return "character";
			case M::kInvalid: return "invalid";
			default: return "dynamic (simulated)";
			}
		}

		// The bone's own frame: local = R^T (p - t) / s.
		RE::NiPoint3 ToLocal(const RE::NiTransform& a_xf, const RE::NiPoint3& a_p)
		{
			const RE::NiPoint3 d = a_p - a_xf.translate;
			const auto& m = a_xf.rotate.entry;
			const float s = a_xf.scale != 0.0f ? a_xf.scale : 1.0f;
			return { (m[0][0] * d.x + m[1][0] * d.y + m[2][0] * d.z) / s, (m[0][1] * d.x + m[1][1] * d.y + m[2][1] * d.z) / s,
				(m[0][2] * d.x + m[1][2] * d.y + m[2][2] * d.z) / s };
		}

		std::vector<Part> ReadActor(RE::Actor* a_actor, uint32_t& a_faults)
		{
			std::vector<Part> parts;
			auto* root = a_actor->Get3D();
			if (!root) {
				return parts;
			}
			const float k = RE::bhkWorld::GetWorldScaleInverse();
			RE::BSVisit::TraverseScenegraphCollision(root, [&](RE::bhkNiCollisionObject* a_collision) {
				auto* rigid = a_collision && a_collision->body ? netimmerse_cast<RE::bhkRigidBody*>(a_collision->body.get()) : nullptr;
				auto* body = rigid ? rigid->GetRigidBody() : nullptr;
				if (!body) {
					return RE::BSVisit::BSVisitControl::kContinue;
				}
				Part p;
				if (!ReadPart(body, k, p.read)) {
					++a_faults;
					return RE::BSVisit::BSVisitControl::kContinue;
				}
				auto* node = a_collision->sceneObject;
				p.bone = node && node->name.c_str() ? node->name.c_str() : "?";
				p.group = GroupOf(p.bone);
				p.centre = { (p.read.lo[0] + p.read.hi[0]) * 0.5f, (p.read.lo[1] + p.read.hi[1]) * 0.5f, (p.read.lo[2] + p.read.hi[2]) * 0.5f };
				if (node) {
					p.bonePos = node->world.translate;
					p.local = ToLocal(node->world, p.centre);
				}
				parts.push_back(std::move(p));
				return RE::BSVisit::BSVisitControl::kContinue;
			});
			return parts;
		}

		std::string KindName(RE::Actor* a_actor)
		{
			auto* race = a_actor->GetRace();
			const char* n = race ? race->GetFullName() : nullptr;
			return fmt::format("{} (race {:08X})", n && *n ? n : "?", race ? race->GetFormID() : 0u);
		}

		void LogKind(RE::Actor* a_actor, const std::vector<Part>& a_parts, bool a_dead)
		{
			logger::info("HITBOX kind {} {}: {} ({:08X}) has {} collision bodies, height {:.0f}, bound radius {:.0f}", KindName(a_actor),
				a_dead ? "DEAD" : "alive", a_actor->GetDisplayFullName(), a_actor->GetFormID(), a_parts.size(), a_actor->GetHeight(),
				a_actor->GetBoundRadius());
			for (const auto& p : a_parts) {
				const auto& r = p.read;
				std::string size = r.capsule
				                       ? fmt::format("radius {:.1f}, length {:.1f}", r.r,
											 std::sqrt((r.a[0] - r.b[0]) * (r.a[0] - r.b[0]) + (r.a[1] - r.b[1]) * (r.a[1] - r.b[1]) + (r.a[2] - r.b[2]) * (r.a[2] - r.b[2])))
				                       : fmt::format("box {:.1f} x {:.1f} x {:.1f}", r.hi[0] - r.lo[0], r.hi[1] - r.lo[1], r.hi[2] - r.lo[2]);
				const float d = p.centre.GetDistance(p.bonePos);
				logger::info("HITBOX   {:<28} {:<9} layer {} ({}), {}, {}, {}, {}, centre {:.0f} from its bone, {:.0f} above the feet", p.bone,
					GroupName(p.group), r.layer, LayerName(r.layer), ShapeName(r.shapeType), size, MotionName(r.motionType),
					r.inWorld ? "in the physics world" : "NOT in the physics world", d, p.centre.z - a_actor->GetPosition().z);
			}
		}

		// Per actor between frames: the previous frame's parts, by bone.
		struct Track
		{
			std::unordered_map<std::string, std::pair<RE::NiPoint3, RE::NiPoint3>> prev;  // bone -> (local, bonePos)
		};
		std::unordered_map<RE::FormID, Track> g_tracks;
		std::set<std::pair<RE::FormID, bool>> g_kindsLogged;  // (race, dead)
		GmskyHitboxes g_out{};

		// Per second.
		struct Second
		{
			uint32_t frames{ 0 }, actorFrames{ 0 }, parts{ 0 }, faults{ 0 }, partFrames{ 0 };
			double drift{ 0 }, driftMax{ 0 }, boneMove{ 0 }, moving{ 0 };  // Skyrim units per frame
			double us{ 0 }, usMax{ 0 };
			uint32_t writes{ 0 }, sentActors{ 0 }, sentParts{ 0 }, approx{ 0 }, gear{ 0 }, full{ 0 }, livingInWorld{ 0 }, livingSent{ 0 };
			std::string nearest;
		} g_sec;

		void ToGmod(const float* a_s, const GmskyOrigin& a_origin, float* a_out)
		{
			const double s[3] = { a_s[0], a_s[1], a_s[2] };
			const auto g = SkyrimToGmod(s, kNoAnchor, a_origin.skyrim_origin);
			a_out[0] = float(g.x);
			a_out[1] = float(g.y);
			a_out[2] = float(g.z);
		}

		// Appends one actor's parts to g_out. False if the table is full.
		bool Send(RE::Actor* a_actor, const std::vector<Part>& a_parts, const GmskyOrigin& a_origin)
		{
			if (g_out.actor_count >= GMSKY_HB_MAX_ACTORS || a_parts.empty()) {
				return a_parts.empty();
			}
			if (g_out.part_count + a_parts.size() > GMSKY_HB_MAX_PARTS) {
				++g_sec.full;
				return false;
			}
			auto& act = g_out.actors[g_out.actor_count++];
			act = GmskyHitActor{};
			act.form_id = a_actor->GetFormID();
			act.first_part = uint16_t(g_out.part_count);
			act.part_count = uint16_t(a_parts.size());
			act.flags = a_actor->IsDead() ? GMSKY_HBA_DEAD : 0u;
			for (std::size_t i = 0; i < a_parts.size(); ++i) {
				const auto& p = a_parts[i];
				auto& out = g_out.parts[g_out.part_count++];
				out = GmskyHitPart{};
				out.group = p.group;
				out.bone = uint8_t(std::min<std::size_t>(i, 255));
				out.flags = p.group == GMSKY_HBG_GEAR ? GMSKY_HBF_GEAR : 0;
				if (p.read.capsule) {
					out.kind = GMSKY_HBK_CAPSULE;
					ToGmod(p.read.a, a_origin, out.a);
					ToGmod(p.read.b, a_origin, out.b);
					out.r = float(p.read.r * kGmodPerSkyrim);
				} else {
					out.kind = GMSKY_HBK_BOX;
					out.flags |= GMSKY_HBF_APPROX;
					ToGmod(p.read.lo, a_origin, out.a);
					ToGmod(p.read.hi, a_origin, out.b);
					++g_sec.approx;
				}
				g_sec.gear += (out.flags & GMSKY_HBF_GEAR) ? 1 : 0;
				if (p.read.inWorld) {
					act.flags |= GMSKY_HBA_IN_WORLD;
				}
			}
			if (!a_actor->IsDead()) {
				++g_sec.livingSent;
				g_sec.livingInWorld += (act.flags & GMSKY_HBA_IN_WORLD) ? 1 : 0;
			}
			return true;
		}

		void Measure(RE::Actor* a_actor, float a_dist, const std::vector<Part>& a_parts, std::unordered_map<RE::FormID, Track>& a_seen)
		{
			++g_sec.actorFrames;
			g_sec.parts += uint32_t(a_parts.size());
			Track& prev = g_tracks[a_actor->GetFormID()];
			Track now;
			for (const auto& p : a_parts) {
				auto it = prev.prev.find(p.bone);
				if (it != prev.prev.end()) {
					const double drift = p.local.GetDistance(it->second.first);
					const double move = p.bonePos.GetDistance(it->second.second);
					++g_sec.partFrames;
					g_sec.drift += drift;
					g_sec.driftMax = std::max(g_sec.driftMax, drift);
					g_sec.boneMove += move;
					g_sec.moving += move > 0.5 ? 1 : 0;
				}
				now.prev[p.bone] = { p.local, p.bonePos };
			}
			a_seen[a_actor->GetFormID()] = std::move(now);
			if (g_sec.nearest.empty()) {
				g_sec.nearest = fmt::format("{} {:08X}{} at {:.0f}: {} bodies", a_actor->GetDisplayFullName(), a_actor->GetFormID(),
					a_actor->IsDead() ? " (dead)" : "", a_dist, a_parts.size());
			}
		}
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame)
	{
		auto* lists = RE::ProcessLists::GetSingleton();
		auto& link = Link::Get();
		if (!lists || !a_player || !link.IsOpen() || !link.OwnsSide()) {
			return;
		}
		LARGE_INTEGER t0, t1, f;
		QueryPerformanceCounter(&t0);
		const auto playerPos = a_player->GetPosition();
		std::vector<std::pair<float, RE::Actor*>> nearby;
		for (auto& handle : lists->highActorHandles) {
			auto ptr = handle.get();
			auto* actor = ptr.get();
			if (!actor || actor == a_player || actor->IsDisabled() || !actor->Is3DLoaded()) {
				continue;
			}
			const float d = actor->GetPosition().GetDistance(playerPos);
			if (d <= kRange) {
				nearby.push_back({ d, actor });
			}
		}
		std::sort(nearby.begin(), nearby.end());
		++g_sec.frames;
		g_out.actor_count = 0;
		g_out.part_count = 0;
		std::unordered_map<RE::FormID, Track> seen;
		int living = 0;
		bool corpse = false;
		for (const auto& [dist, actor] : nearby) {
			if (g_out.actor_count >= GMSKY_HB_MAX_ACTORS) {
				break;
			}
			const auto parts = ReadActor(actor, g_sec.faults);
			const bool isDead = actor->IsDead();
			auto* race = actor->GetRace();
			if (g_kindsLogged.insert({ race ? race->GetFormID() : 0u, isDead }).second) {
				LogKind(actor, parts, isDead);
			}
			if (isDead && !corpse) {
				corpse = true;
				Measure(actor, dist, parts, seen);
			} else if (!isDead && living < kLiving) {
				++living;
				Measure(actor, dist, parts, seen);
			}
			if (a_origin.epoch != 0) {
				Send(actor, parts, a_origin);
			}
		}
		g_tracks = std::move(seen);
		if (a_origin.epoch != 0) {
			g_out.origin_epoch = a_origin.epoch;
			g_out.skyrim_frame = a_frame;
			QueryPerformanceCounter(&t1);
			g_out.qpc = t1.QuadPart;
			SlotWrite(link.Shared().hitboxes, g_out);
			++g_sec.writes;
			g_sec.sentActors += g_out.actor_count;
			g_sec.sentParts += g_out.part_count;
		}
		QueryPerformanceCounter(&t1);
		QueryPerformanceFrequency(&f);
		const double us = double(t1.QuadPart - t0.QuadPart) * 1e6 / double(f.QuadPart);
		g_sec.us += us;
		g_sec.usMax = std::max(g_sec.usMax, us);
	}

	void LogSecond()
	{
		auto& s = g_sec;
		if (s.actorFrames > 0 || s.sentActors > 0) {
			const double n = s.partFrames ? double(s.partFrames) : 1.0;
			const double w = s.writes ? double(s.writes) : 1.0;
			logger::info("HITBOX second: sent {} writes, {:.1f} NPCs and {:.0f} parts per write ({} approximate, {} gear, {} times the table was full), "
						 "living NPCs with parts in the physics world {}/{} | drift check on {:.1f} NPCs: parts in their bone's frame moved {:.2f} avg "
						 "({:.2f} max) per frame while the bones moved {:.2f} avg ({:.0f}% moving) | {} read faults | {:.0f} us avg ({:.0f} max) | nearest {}",
				s.writes, s.sentActors / w, s.sentParts / w, s.approx, s.gear, s.full, s.livingInWorld, s.livingSent,
				double(s.actorFrames) / std::max(1u, s.frames), s.drift / n, s.driftMax, s.boneMove / n, 100.0 * s.moving / n, s.faults,
				s.us / std::max(1u, s.frames), s.usMax, s.nearest);
		}
		s = Second{};
	}
}
