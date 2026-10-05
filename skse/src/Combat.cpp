// Phase 3 step 3d: Garry's Mod's hits on Skyrim's NPCs (DESIGN.md "3d in detail").
//
// Garry's Mod puts every hit on an NPC's stand-in into the GmskyHitEvents ring (already scaled:
// gmsky_damage_scale, head x gmsky_headshot_scale). Here, every frame while the puppet drives,
// the new events are applied the way SkyCraft applies Minecraft's hits (SkyCraft, MIT, Combat.cpp
// ResolveHitPipeline / StandInWeapon / HitNode / ApplyHit): through Skyrim's own melee hit
// function, so the NPC reacts, bleeds, gets angry and it counts as a crime like a Skyrim hit; a
// vanilla weapon stands in for the impact effect and sound; explosions use plain damage and
// Skyrim's explosion knock-back. Step 3d-2: hits on corpses push the ragdoll (SkyCraft's
// FlingRagdoll / UpdateFlings, MIT); a blast that kills throws the body.
//
// Step 3e (DESIGN.md "3e in detail"): Skyrim's NPCs hurting the player. Garry's Mod's health
// decides, so while the puppet drives the Skyrim player is essential and topped back to full every
// frame; what was topped up goes to Garry's Mod as a hurt event, with the attacker and kind from
// Skyrim's hit event and a HandleHealthDamage hook (SkyCraft, MIT, Combat.cpp HitSink /
// HandleHealthDamageHook / BridgePlayerDamage / SetEssential).
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

		// ---- step 3e: what hurt the player (game thread) ----
		constexpr float kHitMemorySeconds = 0.35f;  // how long a hit event can explain a health drop
		constexpr float kDotFlushSeconds = 0.5f;    // spells / poison / unknown damage is batched

		struct RecentHit
		{
			RE::FormID attacker{ 0 };
			RE::FormID source{ 0 };
			uint8_t kind{ GMSKY_HURTK_MELEE };
			uint8_t flags{ 0 };
			float age{ 99.0f };
		};
		RecentHit g_lastHit;             // from TESHitEvent
		RE::FormID g_lastDamager{ 0 };   // from HandleHealthDamage
		float g_lastDamagerAge{ 99.0f };

		struct PendingHurt
		{
			RE::FormID attacker{ 0 };
			uint8_t kind{ GMSKY_HURTK_OTHER };
			float damage{ 0.0f };
			float age{ 0.0f };
		};
		PendingHurt g_dot;  // spells, poison, unknown: added up, sent every kDotFlushSeconds

		bool g_essentialSet = false;
		bool g_healthPrimed = false;

		// Step 3f: someone fighting the player (SkyCraft's AnyoneFighting, MIT).
		bool g_inCombat = false;
		float g_combatTimer = 0.0f;
		bool AnyoneFighting(RE::PlayerCharacter* a_player)
		{
			auto* lists = RE::ProcessLists::GetSingleton();
			if (!lists) {
				return false;
			}
			const auto playerPos = a_player->GetPosition();
			for (auto& handle : lists->highActorHandles) {
				auto actor = handle.get();
				if (actor && !actor->IsDead() && actor->IsInCombat() && actor->GetActorRuntimeData().currentCombatTarget.get().get() == a_player &&
					actor->GetPosition().GetDistance(playerPos) < 6000.0f) {
					return true;
				}
			}
			return false;
		}

		struct HurtSecond
		{
			uint32_t hitEvents{ 0 }, sent{ 0 };
			float toppedUp{ 0 }, sentDamage{ 0 };
		} g_hurt;
		uint32_t g_hurtLines = 0;

		const char* HurtKindName(uint8_t a_kind)
		{
			switch (a_kind) {
			case GMSKY_HURTK_MELEE: return "melee";
			case GMSKY_HURTK_PROJECTILE: return "projectile";
			case GMSKY_HURTK_MAGIC: return "magic";
			default: return "other";
			}
		}

		class HitSink final : public RE::BSTEventSink<RE::TESHitEvent>
		{
		public:
			static HitSink* Get()
			{
				static HitSink sink;
				return &sink;
			}

			RE::BSEventNotifyControl ProcessEvent(const RE::TESHitEvent* a_event, RE::BSTEventSource<RE::TESHitEvent>*) override
			{
				auto* player = RE::PlayerCharacter::GetSingleton();
				if (!a_event || !player || a_event->target.get() != player || !g_essentialSet) {
					return RE::BSEventNotifyControl::kContinue;
				}
				RecentHit hit;
				hit.attacker = a_event->cause ? a_event->cause->GetFormID() : 0;
				hit.source = a_event->source;
				hit.age = 0.0f;
				if (a_event->projectile != 0) {
					hit.kind = GMSKY_HURTK_PROJECTILE;
				} else if (auto* source = RE::TESForm::LookupByID(a_event->source)) {
					switch (source->GetFormType()) {
					case RE::FormType::Spell:
					case RE::FormType::Enchantment:
					case RE::FormType::Scroll:
					case RE::FormType::Ingredient:
					case RE::FormType::AlchemyItem:
						hit.kind = GMSKY_HURTK_MAGIC;
						break;
					default:
						hit.kind = GMSKY_HURTK_MELEE;
						break;
					}
				}
				if (a_event->flags.any(RE::TESHitEvent::Flag::kPowerAttack)) {
					hit.flags |= GMSKY_HURTF_POWER;
				}
				if (a_event->flags.any(RE::TESHitEvent::Flag::kHitBlocked)) {
					hit.flags |= GMSKY_HURTF_BLOCKED;
				}
				g_lastHit = hit;
				++g_hurt.hitEvents;
				return RE::BSEventNotifyControl::kContinue;
			}
		};

		// Skyrim's damage path calls this on the victim with the attacker; we only need "who".
		struct HandleHealthDamageHook
		{
			static void thunk(RE::Actor* a_this, RE::Actor* a_attacker, float a_damage)
			{
				func(a_this, a_attacker, a_damage);
				if (a_attacker && a_attacker != a_this) {
					g_lastDamager = a_attacker->GetFormID();
					g_lastDamagerAge = 0.0f;
				}
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};

		void SendHurt(uint8_t a_kind, float a_damage, RE::FormID a_attacker, RE::FormID a_source, uint8_t a_flags)
		{
			if (a_damage <= 0.01f) {
				return;
			}
			auto& ring = Link::Get().Shared().hurt_events;
			auto* head = reinterpret_cast<std::atomic<uint32_t>*>(&ring.head);
			const uint32_t h = head->load(std::memory_order_relaxed);
			GmskyHurtEvent ev{};
			ev.attacker = a_attacker;
			ev.damage = a_damage;
			ev.source = a_source;
			ev.kind = a_kind;
			ev.flags = a_flags;
			ring.session = GetCurrentProcessId();
			ring.ev[h % GMSKY_HURT_MAX] = ev;
			head->store(h + 1, std::memory_order_release);
			++g_hurt.sent;
			g_hurt.sentDamage += a_damage;
			if (g_hurtLines < 20) {
				++g_hurtLines;
				auto* who = a_attacker ? RE::TESForm::LookupByID<RE::Actor>(a_attacker) : nullptr;
				auto* src = a_source ? RE::TESForm::LookupByID(a_source) : nullptr;
				logger::info("COMBAT player hurt by {} ({:08X}) with {} ({:08X}): {:.1f} Skyrim damage, {}{}{} -> Garry's Mod (event {})",
					who ? who->GetDisplayFullName() : "nobody", a_attacker, src ? src->GetName() : "?", a_source, a_damage,
					HurtKindName(a_kind), (a_flags & GMSKY_HURTF_POWER) ? ", power attack" : "",
					(a_flags & GMSKY_HURTF_BLOCKED) ? ", blocked" : "", h + 1);
			}
		}

		void SetEssential(RE::PlayerCharacter* a_player, bool a_on)
		{
			if (a_on == g_essentialSet) {
				return;
			}
			auto& flags = a_player->GetActorRuntimeData().boolFlags;
			if (a_on) {
				flags.set(RE::Actor::BOOL_FLAGS::kEssential);
			} else {
				flags.reset(RE::Actor::BOOL_FLAGS::kEssential);
			}
			g_essentialSet = a_on;
			logger::info("COMBAT Skyrim player essential {} (Garry's Mod {} the player's health)", a_on ? "on" : "off",
				a_on ? "owns" : "no longer owns");
		}

		// Garry's Mod owns the player's health: Skyrim damage is topped back up here and sent.
		void BridgePlayerDamage(RE::PlayerCharacter* a_player, float a_delta)
		{
			auto* av = a_player->AsActorValueOwner();
			const float max = a_player->GetActorValueMax(RE::ActorValue::kHealth);
			const float cur = av->GetActorValue(RE::ActorValue::kHealth);
			const float deficit = max - cur;
			g_lastHit.age += a_delta;
			g_lastDamagerAge += a_delta;
			if (!g_healthPrimed) {
				// Whatever damage the save had before Garry's Mod took over isn't a new hit.
				g_healthPrimed = true;
				if (deficit > 0.0f) {
					av->RestoreActorValue(RE::ActorValue::kHealth, deficit);
					logger::info("COMBAT player health topped up from {:.0f} to {:.0f} as Garry's Mod takes over (not sent)", cur, max);
				}
				return;
			}
			if (deficit > 0.01f) {
				av->RestoreActorValue(RE::ActorValue::kHealth, deficit);
				g_hurt.toppedUp += deficit;
				RE::FormID attacker = 0, source = 0;
				uint8_t kind = GMSKY_HURTK_OTHER, flags = 0;
				if (g_lastHit.age < kHitMemorySeconds) {
					attacker = g_lastHit.attacker;
					source = g_lastHit.source;
					kind = g_lastHit.kind;
					flags = g_lastHit.flags;
					g_lastHit.age = 99.0f;  // one hit event explains one health drop
				} else if (g_lastDamagerAge < kHitMemorySeconds) {
					attacker = g_lastDamager;
					kind = GMSKY_HURTK_MAGIC;  // damage over time from someone (spells, poison)
				}
				if (kind == GMSKY_HURTK_MELEE || kind == GMSKY_HURTK_PROJECTILE) {
					SendHurt(kind, deficit, attacker, source, flags);
				} else {
					g_dot.attacker = attacker ? attacker : g_dot.attacker;
					g_dot.kind = kind;
					g_dot.damage += deficit;
				}
			}
			g_dot.age += a_delta;
			if (g_dot.age >= kDotFlushSeconds) {
				SendHurt(g_dot.kind, g_dot.damage, g_dot.attacker, 0, 0);
				g_dot = PendingHurt{};
			}
		}

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

		// ---- step 3d-2: pushing ragdolls the way Garry's Mod pushes its own (after test 63) ----
		// The push is Garry's Mod's own damage force (kg x inch/s, times gmsky_ragdoll_force), as
		// Havok kg x m/s. Bullets / melee: on the part hit, at the point hit (it spins as well as
		// moves). Blasts: shared by mass, each body out from the blast from where it is (the limbs
		// fly apart). SkyCraft's FlingRagdoll / UpdateFlings (MIT) for the body walk and the wait.
		constexpr float kInchToMetre = 0.0254f;
		constexpr float kMaxBodySpeed = 30.0f;  // m/s on any one body

		struct Push
		{
			uint8_t bone{ 255 };     // part hit (the index in Hitboxes.cpp's walk); 255 = a blast: all bodies
			RE::NiPoint3 point;      // Skyrim units: where the shot landed, or the blast's centre
			RE::NiPoint3 dir;        // unit direction of the shot (not used for blasts)
			float impulse{ 0.0f };   // Havok kg x m/s from Garry's Mod's force; 0 = none sent
			float fallback{ 0.0f };  // m/s when there's no force
		};

		struct Pushed
		{
			int bodies{ 0 };
			float mass{ 0.0f };   // kg
			float speed{ 0.0f };  // m/s given (average for a blast)
		};

		float BulletPush(const GmskyHitEvent& a_ev) { return std::clamp(a_ev.gmod_damage * 0.25f, 1.5f, 15.0f); }
		float BlastPush(const GmskyHitEvent& a_ev) { return std::clamp(a_ev.gmod_damage * 0.1f, 3.0f, 15.0f); }

		Push PushFor(const GmskyHitEvent& a_ev, const RE::NiPoint3& a_point, const RE::NiPoint3& a_dir)
		{
			const bool blast = a_ev.kind == GMSKY_HITK_EXPLOSION;
			return Push{ blast ? uint8_t(255) : a_ev.bone, a_point, a_dir, a_ev.force * kInchToMetre, blast ? BlastPush(a_ev) : BulletPush(a_ev) };
		}

		RE::NiPoint3 ToPoint(const RE::hkVector4& a_v)
		{
			alignas(16) float f[4];
			_mm_store_ps(f, a_v.quad);
			return { f[0], f[1], f[2] };
		}

		bool Dynamic(RE::hkpRigidBody* a_body)
		{
			using Motion = RE::hkpMotion::MotionType;
			const auto type = a_body->motion.type.get();
			return type != Motion::kFixed && type != Motion::kKeyframed && type != Motion::kCharacter && type != Motion::kInvalid;
		}

		Pushed PushRagdoll(RE::Actor* a_actor, const Push& a_push)
		{
			Pushed out;
			auto* root = a_actor->Get3D();
			auto* cell = a_actor->GetParentCell();
			auto* world = cell ? cell->GetbhkWorld() : nullptr;
			if (!root || !world) {
				return out;
			}
			const float scale = RE::bhkWorld::GetWorldScale();  // Skyrim units -> Havok metres
			std::vector<RE::hkpRigidBody*> bodies;
			uint32_t i = 0;
			RE::BSWriteLockGuard lock(world->worldLock);
			RE::BSVisit::TraverseScenegraphCollision(root, [&](RE::bhkNiCollisionObject* a_collision) {
				auto* rigid = a_collision && a_collision->body ? netimmerse_cast<RE::bhkRigidBody*>(a_collision->body.get()) : nullptr;
				auto* body = rigid ? rigid->GetRigidBody() : nullptr;
				if (!body) {
					return RE::BSVisit::BSVisitControl::kContinue;
				}
				const bool mine = a_push.bone == 255 || i == a_push.bone;
				++i;
				if (mine && Dynamic(body)) {
					bodies.push_back(body);
				}
				return a_push.bone == 255 || !mine ? RE::BSVisit::BSVisitControl::kContinue : RE::BSVisit::BSVisitControl::kStop;
			});
			for (auto* b : bodies) {
				out.mass += b->motion.GetMass();
			}
			if (bodies.empty() || out.mass <= 0.0f) {
				return out;
			}
			out.bodies = int(bodies.size());
			const RE::NiPoint3 at{ a_push.point.x * scale, a_push.point.y * scale, a_push.point.z * scale };
			if (a_push.bone != 255) {
				auto* b = bodies.front();
				const float speed = std::min(a_push.impulse > 0.0f ? a_push.impulse / out.mass : a_push.fallback, kMaxBodySpeed);
				const float j = speed * out.mass;
				b->ApplyLinearImpulse(RE::hkVector4(0.0f));  // wakes it (hkpEntity::Activate is protected here)
				b->motion.ApplyPointImpulse(RE::hkVector4(a_push.dir.x * j, a_push.dir.y * j, a_push.dir.z * j, 0.0f), RE::hkVector4(at));
				out.speed = speed;
				return out;
			}
			const float speed = std::min(a_push.impulse > 0.0f ? a_push.impulse / out.mass : a_push.fallback, kMaxBodySpeed);
			for (auto* b : bodies) {
				RE::NiPoint3 d = ToPoint(b->motion.motionState.sweptTransform.centerOfMass1) - at;
				const float len = d.Length();
				d.z += 0.3f * len + 0.05f;  // a little up
				const float l2 = d.Length();
				d = l2 > 1e-4f ? d / l2 : RE::NiPoint3{ 0.0f, 0.0f, 1.0f };
				const float j = speed * b->motion.GetMass();
				b->ApplyLinearImpulse(RE::hkVector4(d.x * j, d.y * j, d.z * j, 0.0f));  // wakes it too
			}
			out.speed = speed;
			return out;
		}

		// A hit that kills (a blast that catches someone, or a killing shot): the push waits until
		// Skyrim has made the ragdoll (a moment after the death).
		struct PendingPush
		{
			RE::ActorHandle actor;
			Push push;
			float timeLeft;
			const char* why;
		};
		std::vector<PendingPush> g_pending;
		uint32_t g_flung = 0, g_corpseLines = 0;

		void UpdatePending(float a_delta)
		{
			for (auto it = g_pending.begin(); it != g_pending.end();) {
				auto actor = it->actor.get();
				it->timeLeft -= a_delta;
				bool done = !actor || it->timeLeft <= 0.0f;
				if (actor && actor->IsDead()) {
					const auto r = PushRagdoll(actor.get(), it->push);
					if (r.bodies > 0) {
						logger::info("COMBAT {} threw the body of {} ({:08X}): {:.1f} kg m/s -> {:.1f} m/s on {} bodies ({:.0f} kg)", it->why,
							actor->GetDisplayFullName(), actor->GetFormID(), it->push.impulse, r.speed, r.bodies, r.mass);
						++g_flung;
						done = true;
					}
				}
				it = done ? g_pending.erase(it) : it + 1;
			}
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
			const double g[3] = { a_ev.pos[0], a_ev.pos[1], a_ev.pos[2] };
			const auto s = GmodToSkyrim(g, kNoAnchor, a_origin.skyrim_origin);
			RE::NiPoint3 hitPos{ float(s.x), float(s.y), float(s.z) };
			RE::NiPoint3 dir{ a_ev.dir[0], a_ev.dir[1], a_ev.dir[2] };  // both games: Z up, same axes
			if (actor->IsDead()) {
				// Step 3d-2: no damage; the ragdoll is pushed (the part hit, or all of it by a blast).
				++g_sec.corpses;
				const bool blast = a_ev.kind == GMSKY_HITK_EXPLOSION;
				const auto push = PushFor(a_ev, hitPos, dir);
				const auto r = PushRagdoll(actor, push);
				auto* node = blast ? nullptr : HitNode(actor, a_ev.bone);
				auto* weapon = blast ? nullptr : StandInWeapon(a_ev);
				if (auto* impacts = RE::BGSImpactManager::GetSingleton(); impacts && weapon && weapon->impactDataSet && node) {
					RE::NiPoint3 pick = dir;
					impacts->PlayImpactEffect(actor, weapon->impactDataSet, node->name.c_str(), pick, 128.0f, false, false);
				}
				if (g_corpseLines < 20) {
					++g_corpseLines;
					logger::info("COMBAT corpse {} ({:08X}): {} on {} ({:.1f} Garry's Mod damage, force {:.0f} -> {:.1f} kg m/s{}) -> {:.1f} m/s on {} bodies ({:.0f} kg){}",
						actor->GetDisplayFullName(), a_ev.form_id, KindName(a_ev.kind), node ? node->name.c_str() : (blast ? "the whole body" : "?"),
						a_ev.gmod_damage, a_ev.force, push.impulse, push.impulse > 0.0f ? "" : ", none: damage-based speed", r.speed, r.bodies, r.mass,
						r.bodies == 0 ? " (nothing moved: no dynamic body)" : "");
				}
				return;
			}
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
				g_pending.push_back({ actor->GetHandle(), PushFor(a_ev, hitPos, dir), 2.0f, "blast" });  // step 3d-2: if it dies, the body flies
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
			if (killed && a_ev.kind != GMSKY_HITK_EXPLOSION) {
				// Step 3d-2: the killing shot pushes the body once Skyrim has made the ragdoll.
				g_pending.push_back({ actor->GetHandle(), PushFor(a_ev, RE::NiPoint3{ float(s.x), float(s.y), float(s.z) }, dir), 1.0f, "killing shot" });
			}
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
		REL::Relocation<std::uintptr_t> vtbl{ RE::VTABLE_PlayerCharacter[0] };
		HandleHealthDamageHook::func = vtbl.write_vfunc(0x104, HandleHealthDamageHook::thunk);
		if (auto* events = RE::ScriptEventSourceHolder::GetSingleton()) {
			events->AddEventSink<RE::TESHitEvent>(HitSink::Get());
		}
		logger::info("COMBAT player hurt listeners installed (hit event, HandleHealthDamage)");
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, bool a_puppeting, float a_delta)
	{
		auto& link = Link::Get();
		if (!link.IsOpen() || !a_player) {
			return;
		}
		g_combatTimer -= a_delta;
		if (g_combatTimer <= 0.0f) {
			g_combatTimer = 0.25f;
			const bool fighting = AnyoneFighting(a_player);
			if (fighting != g_inCombat) {
				logger::info("COMBAT player {}", fighting ? "IN A FIGHT (someone nearby targets the player)" : "calm (nobody nearby fights the player)");
				g_inCombat = fighting;
			}
		}
		// Step 3e: while Garry's Mod drives, it owns the player's health.
		if (a_puppeting) {
			SetEssential(a_player, true);
			BridgePlayerDamage(a_player, a_delta);
		} else {
			SetEssential(a_player, false);
			g_healthPrimed = false;
			g_dot = PendingHurt{};
		}
		UpdatePending(a_delta);
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

	bool PlayerInCombat()
	{
		return g_inCombat;
	}

	void LogSecond()
	{
		auto& s = g_sec;
		if (s.events > 0 || s.lost > 0 || g_flung > 0) {
			logger::info("COMBAT second: {} hits from Garry's Mod ({:.0f} Skyrim damage): {} through Skyrim's hit function, {} plain, {} explosions, "
						 "{} kills, {} on corpses (pushed), {} bodies thrown by blasts | not applied: {} NPC not loaded, {} other origin, {} while not driving, {} lost",
				s.events, s.damage, s.pipeline, s.plain, s.explosions, s.kills, s.corpses, g_flung, s.missing, s.otherOrigin, s.notDriving, s.lost);
		}
		s = Second{};
		g_lines = 0;
		g_corpseLines = 0;
		g_flung = 0;
		auto& h = g_hurt;
		if (h.hitEvents > 0 || h.toppedUp > 0.0f || h.sent > 0) {
			logger::info("COMBAT player second: {} hit events on the player, {:.1f} health topped up, {} hurts sent to Garry's Mod ({:.1f} Skyrim damage), essential {}",
				h.hitEvents, h.toppedUp, h.sent, h.sentDamage, g_essentialSet ? "on" : "off");
		}
		h = HurtSecond{};
		g_hurtLines = 0;
	}
}
