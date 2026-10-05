// Phase 3 step 3d: Garry's Mod's hits on Skyrim's NPCs (DESIGN.md "3d in detail").
//
// Garry's Mod puts every hit on an NPC's stand-in into the GmskyHitEvents ring (already scaled:
// gmsky_damage_scale, head x gmsky_headshot_scale). Here, every frame while the puppet drives,
// the new events are applied the way SkyCraft applies Minecraft's hits (SkyCraft, MIT, Combat.cpp
// ResolveHitPipeline / StandInWeapon / HitNode / ApplyHit): through Skyrim's own melee hit
// function, so the NPC reacts, bleeds, gets angry and it counts as a crime like a Skyrim hit; a
// vanilla weapon stands in for the impact effect and sound; explosions use plain damage and
// Skyrim's explosion knock-back. Corpses: step 3d-2 (logged only here).
#include "Combat.h"

#include "Link.h"

#include <array>

namespace gmsky::Combat
{
	namespace
	{
		constexpr double kNoAnchor[3] = { 0.0, 0.0, 0.0 };

		// ID 38627 is Skyrim's melee "HitFrame" handler: it builds a HitData (Ctor 43995, Populate
		// 44001) and hands it to ID 38586, which applies the hit to the victim: damage, blocking,
		// hit reactions and stagger, pain voice, the hit event (crime, quests, the enemy health
		// bar), kill credit. (SkyCraft's notes, same game build.)
		using ProcessHitFn = void(RE::Actor*, RE::HitData&);
		using HitDataCtorFn = RE::HitData*(RE::HitData*);
		ProcessHitFn* g_processHit = nullptr;
		HitDataCtorFn* g_hitDataCtor = nullptr;

		bool g_haveTail = false;
		uint32_t g_tail = 0;

		struct Second
		{
			uint32_t events{ 0 }, pipeline{ 0 }, plain{ 0 }, explosions{ 0 }, corpses{ 0 }, missing{ 0 }, otherOrigin{ 0 }, lost{ 0 },
				notDriving{ 0 }, kills{ 0 };
			float damage{ 0 };
		} g_sec;
		uint32_t g_lines = 0;  // hit lines this second (capped)

		void ResolveHitPipeline()
		{
			if (!REL::Module::IsAE()) {
				logger::info("COMBAT hit pipeline: not on AE; hits use plain damage");
				return;
			}
			// Only trust the address if the engine's own melee code really calls it.
			const auto site = REL::ID(38627).address() + 0x4A8;
			const auto* code = reinterpret_cast<const std::uint8_t*>(site);
			const auto target = REL::ID(38586).address();
			std::int32_t rel = 0;
			std::memcpy(&rel, code + 1, 4);
			if (code[0] != 0xE8 || site + 5 + rel != target) {
				logger::warn("COMBAT hit pipeline: the melee call site doesn't match this game build; hits use plain damage");
				return;
			}
			g_processHit = reinterpret_cast<ProcessHitFn*>(target);
			g_hitDataCtor = reinterpret_cast<HitDataCtorFn*>(REL::ID(43995).address());
			logger::info("COMBAT hit pipeline: Garry's Mod hits go through Skyrim's own hit processing (call site checked)");
		}

		// A vanilla weapon standing in for Garry's Mod's: its impact set gives the blood and sound.
		RE::TESObjectWEAP* StandInWeapon(const GmskyHitEvent& a_ev)
		{
			RE::FormID id = 0x00013982;  // Iron Mace: clubs, the crowbar, anything else
			if (a_ev.kind == GMSKY_HITK_BULLET) {
				id = 0x00013985;  // Hunting Bow: arrow-like impacts for bullets
			} else if (a_ev.dmg_type & 4u) {
				id = 0x00012EB7;  // Iron Sword: slashes
			}
			return RE::TESForm::LookupByID<RE::TESObjectWEAP>(id);
		}

		// The bone the hit landed on (the part's index in the same collision walk as Hitboxes.cpp),
		// else the upper spine as SkyCraft's HitNode.
		RE::NiAVObject* HitNode(RE::Actor* a_actor, uint8_t a_bone)
		{
			auto* root = a_actor->Get3D();
			if (!root) {
				return nullptr;
			}
			if (a_bone != 255) {
				RE::NiAVObject* found = nullptr;
				uint32_t i = 0;
				RE::BSVisit::TraverseScenegraphCollision(root, [&](RE::bhkNiCollisionObject* a_collision) {
					auto* rigid = a_collision && a_collision->body ? netimmerse_cast<RE::bhkRigidBody*>(a_collision->body.get()) : nullptr;
					if (!rigid || !rigid->GetRigidBody()) {
						return RE::BSVisit::BSVisitControl::kContinue;
					}
					if (i++ == a_bone) {
						found = a_collision->sceneObject;
						return RE::BSVisit::BSVisitControl::kStop;
					}
					return RE::BSVisit::BSVisitControl::kContinue;
				});
				if (found) {
					return found;
				}
			}
			for (const char* name : { "NPC Spine2 [Spn2]", "NPC Spine1 [Spn1]", "NPC Spine [Spn0]", "NPC Pelvis [Pelv]" }) {
				if (auto* node = root->GetObjectByName(name)) {
					return node;
				}
			}
			return root;
		}

		const char* KindName(uint8_t a_kind)
		{
			switch (a_kind) {
			case GMSKY_HITK_BULLET: return "bullet";
			case GMSKY_HITK_MELEE: return "melee";
			case GMSKY_HITK_EXPLOSION: return "explosion";
			default: return "other";
			}
		}

		void Apply(RE::PlayerCharacter* a_player, const GmskyHitEvent& a_ev, const GmskyOrigin& a_origin)
		{
			auto* actor = RE::TESForm::LookupByID<RE::Actor>(a_ev.form_id);
			if (!actor || !actor->Is3DLoaded()) {
				++g_sec.missing;
				return;
			}
			if (a_ev.origin_epoch != a_origin.epoch) {
				++g_sec.otherOrigin;
				return;
			}
			if (actor->IsDead()) {
				++g_sec.corpses;  // step 3d-2: push the ragdoll
				return;
			}
			const double g[3] = { a_ev.pos[0], a_ev.pos[1], a_ev.pos[2] };
			const auto s = GmodToSkyrim(g, kNoAnchor, a_origin.skyrim_origin);
			RE::NiPoint3 hitPos{ float(s.x), float(s.y), float(s.z) };
			RE::NiPoint3 dir{ a_ev.dir[0], a_ev.dir[1], a_ev.dir[2] };  // both games: Z up, same axes
			const float damage = a_ev.damage;
			auto* av = actor->AsActorValueOwner();
			const float before = av->GetActorValue(RE::ActorValue::kHealth);
			auto* node = HitNode(actor, a_ev.bone);
			const char* path = "plain damage";
			auto* weapon = StandInWeapon(a_ev);
			if (a_ev.kind == GMSKY_HITK_EXPLOSION) {
				actor->DoDamage(damage, a_player, true);
				if (auto* process = actor->GetActorRuntimeData().currentProcess; process && !actor->IsDead()) {
					process->KnockExplosion(actor, hitPos, std::clamp(a_ev.gmod_damage * 0.25f, 5.0f, 40.0f));
				}
				weapon = nullptr;  // no blood spray from a stand-in weapon for a blast
				path = "explosion (plain damage + Skyrim's knock-back)";
				++g_sec.explosions;
			} else if (g_processHit && damage > 0.0f) {
				if (node) {
					hitPos = node->world.translate;
				}
				alignas(16) std::array<std::byte, sizeof(RE::HitData)> storage{};
				auto* hit = reinterpret_cast<RE::HitData*>(storage.data());
				g_hitDataCtor(hit);
				hit->Populate(a_player, actor, nullptr);
				hit->weapon = weapon;
				hit->hitPosition = hitPos;
				hit->hitDirection = dir;
				hit->totalDamage = damage;
				hit->physicalDamage = damage;
				hit->resistedPhysicalDamage = 0.0f;
				hit->resistedTypedDamage = 0.0f;
				hit->sneakAttackBonus = 1.0f;
				hit->bonusHealthDamageMult = 1.0f;
				hit->stagger = 0.0f;  // bullets don't stun-lock (SkyCraft: only big hits stagger)
				hit->pushBack = 0.0f;
				hit->skill = RE::ActorValue::kNone;
				hit->flags.reset(RE::HitData::Flag::kPowerAttack, RE::HitData::Flag::kCritical, RE::HitData::Flag::kSneakAttack, RE::HitData::Flag::kMeleeAttack);
				if (a_ev.kind == GMSKY_HITK_MELEE) {
					hit->flags.set(RE::HitData::Flag::kMeleeAttack);
				}
				g_processHit(actor, *hit);
				path = "Skyrim's hit function";
				++g_sec.pipeline;
			} else if (damage > 0.0f) {
				actor->DoDamage(damage, a_player, true);
				++g_sec.plain;
			}
			// Blood and the impact sound at the bone that was hit, like a Skyrim weapon connecting.
			if (auto* impacts = RE::BGSImpactManager::GetSingleton(); impacts && weapon && weapon->impactDataSet && node && damage > 0.0f) {
				RE::NiPoint3 pick = dir;
				impacts->PlayImpactEffect(actor, weapon->impactDataSet, node->name.c_str(), pick, 128.0f, false, false);
			}
			if (!actor->IsDead() && !actor->IsPlayerTeammate() && !actor->IsInCombat()) {
				actor->StartCombat(a_player);
			}
			const float after = av->GetActorValue(RE::ActorValue::kHealth);
			// Skyrim marks the death later in the frame (test 60: IsDead() was still false here), so a
			// kill is this hit taking the health from above 0 to 0 or below.
			const bool killed = before > 0.0f && after <= 0.0f;
			g_sec.kills += killed ? 1 : 0;
			g_sec.damage += damage;
			if (g_lines < 20) {
				++g_lines;
				logger::info("COMBAT hit {} ({:08X}, level {}) {} at {} ({}): {:.1f} Garry's Mod -> {:.1f} Skyrim damage via {}; health {:.0f} -> {:.0f}{}",
					actor->GetDisplayFullName(), a_ev.form_id, actor->GetLevel(), KindName(a_ev.kind), node ? node->name.c_str() : "?", a_ev.group,
					a_ev.gmod_damage, damage, path, before, after, killed ? ", KILLED" : "");
			}
		}
	}

	void Install()
	{
		ResolveHitPipeline();
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, bool a_puppeting)
	{
		auto& link = Link::Get();
		if (!link.IsOpen() || !a_player) {
			return;
		}
		auto& ring = link.Shared().hit_events;
		const uint32_t head = reinterpret_cast<std::atomic<uint32_t>*>(&ring.head)->load(std::memory_order_acquire);
		if (!g_haveTail || head < g_tail) {
			g_tail = head;  // first read (or the block was made new): hits from before don't land
			g_haveTail = true;
			return;
		}
		if (head - g_tail > GMSKY_EV_MAX) {
			g_sec.lost += head - g_tail - GMSKY_EV_MAX;
			g_tail = head - GMSKY_EV_MAX;
		}
		while (g_tail != head) {
			const GmskyHitEvent ev = ring.ev[g_tail % GMSKY_EV_MAX];
			++g_tail;
			++g_sec.events;
			if (!a_puppeting) {
				++g_sec.notDriving;  // Garry's Mod isn't driving the player: drop, as SkyCraft does
				continue;
			}
			Apply(a_player, ev, a_origin);
		}
	}

	void LogSecond()
	{
		auto& s = g_sec;
		if (s.events > 0 || s.lost > 0) {
			logger::info("COMBAT second: {} hits from Garry's Mod ({:.0f} Skyrim damage): {} through Skyrim's hit function, {} plain, {} explosions, "
						 "{} kills | not applied: {} on corpses (3d-2), {} NPC not loaded, {} other origin, {} while not driving, {} lost",
				s.events, s.damage, s.pipeline, s.plain, s.explosions, s.kills, s.corpses, s.missing, s.otherOrigin, s.notDriving, s.lost);
		}
		s = Second{};
		g_lines = 0;
	}
}
