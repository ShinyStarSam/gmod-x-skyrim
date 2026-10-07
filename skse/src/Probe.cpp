// The per-frame driver. Every frame: publish Skyrim's player state, manage the origin (a new one
// on game load, location change, or when Skyrim moves the player itself), and run the puppet
// (Puppet.cpp). Once a second, log Skyrim's numbers next to Garry's Mod's.
#include "Probe.h"

#include "Actors.h"
#include "Arms.h"
#include "Body.h"
#include "Camera.h"
#include "Collision.h"
#include "Combat.h"
#include "Draw.h"
#include "Render.h"
#include "World.h"
#include "Hitboxes.h"
#include "Hud.h"
#include "Input.h"
#include "Link.h"
#include "Loose.h"
#include "Overlay.h"
#include "Puppet.h"
#include "Terrain.h"

namespace gmsky::Probe
{
	namespace
	{
		constexpr double kRadToDeg = 180.0 / kPi;

		uint64_t g_frame = 0;
		GmskyOrigin g_origin{};  // the origin we published last (epoch 0 = none yet)
		uint64_t g_lastOpenTry = 0;
		bool g_lostOwnershipLogged = false;
		bool g_needOrigin = true;
		// Where the player is: the worldspace for exteriors, the cell for interiors. A change means
		// Skyrim moved the player somewhere else (load door, fast travel): new origin.
		uint32_t g_placeKey = 0;
		float g_logTimer = 0.0f;
		float g_halfWidth = 0.0f;  // S2j: the player's half width sent last frame, Skyrim units
		RE::NiPoint3 g_lastLogPos{};
		bool g_haveLastLogPos = false;

		// Skyrim convention (assumed): heading 0 = north (+Y), clockwise. So moving along (dx, dy)
		// has heading atan2(dx, dy).
		double SkyrimMoveHeadingDeg(double dx, double dy)
		{
			return WrapRad(std::atan2(dx, dy)) * kRadToDeg;
		}

		// Smallest signed difference between two angles in degrees, in (-180, 180].
		double AngleDiffDeg(double a, double b)
		{
			double d = std::fmod(a - b, 360.0);
			if (d > 180.0) d -= 360.0;
			if (d <= -180.0) d += 360.0;
			return d;
		}

		// After test 136 ("the player is too big"): Skyrim's own player collision, to size Garry's
		// Mod's box from it. Everything read inside a fault guard; Skyrim units (Havok x a_k).
		struct ControllerRead
		{
			float radius = 0, height = 0, actorHeight = 0, scale = 0, center = 0;
			float bound[6]{};   // collisionBound centre, extents
			float bumper[6]{};  // bumperCollisionBound centre, extents
			int   type[2]{ -1, -1 };
			float size[2][3]{};  // each shape's box (no rotation): width x, depth y, height z
		};

		bool ReadController(const RE::bhkCharacterController* a_c, float a_k, ControllerRead& a_out)
		{
			__try {
				a_out.radius = a_c->radius;
				a_out.height = a_c->height;
				a_out.actorHeight = a_c->actorHeight;
				a_out.scale = a_c->scale;
				a_out.center = a_c->center;
				const RE::BSBound* bounds[2] = { &a_c->collisionBound, &a_c->bumperCollisionBound };
				float* outs[2] = { a_out.bound, a_out.bumper };
				for (int b = 0; b < 2; ++b) {
					outs[b][0] = bounds[b]->center.x;
					outs[b][1] = bounds[b]->center.y;
					outs[b][2] = bounds[b]->center.z;
					outs[b][3] = bounds[b]->extents.x;
					outs[b][4] = bounds[b]->extents.y;
					outs[b][5] = bounds[b]->extents.z;
				}
				alignas(16) const float identity[16] = { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0 };
				for (int i = 0; i < 2; ++i) {
					const RE::bhkShape* bs = a_c->shapes[i].get();
					const auto* shape = bs ? static_cast<const RE::hkpShape*>(bs->referencedObject.get()) : nullptr;
					if (!shape) {
						continue;
					}
					a_out.type[i] = static_cast<int>(shape->type);
					RE::hkAabb box;
					shape->GetAabbImpl(*reinterpret_cast<const RE::hkTransform*>(identity), 0.0f, box);
					alignas(16) float mn[4], mx[4];
					_mm_store_ps(mn, box.min.quad);
					_mm_store_ps(mx, box.max.quad);
					for (int k = 0; k < 3; ++k) {
						a_out.size[i][k] = (mx[k] - mn[k]) * a_k;
					}
				}
				return true;
			} __except (EXCEPTION_EXECUTE_HANDLER) {
				return false;
			}
		}

		// S2j: the player's capsule radius in Garry's Mod units, sent to Garry's Mod (which sizes
		// its box from it): half the narrower side of the first (standing) shape's box, which for a
		// capsule is its radius whatever unit the controller's own `radius` field is in. 0 if
		// unknown.
		float RadiusGmod(RE::PlayerCharacter* a_player)
		{
			auto* c = a_player->GetCharController();
			ControllerRead r;
			if (!c || !ReadController(c, RE::bhkWorld::GetWorldScaleInverse(), r) || r.type[0] < 0) {
				return 0.0f;
			}
			const float half = 0.5f * std::min(r.size[0][0], r.size[0][1]);
			return half > 1.0f && half < 200.0f ? half * float(kGmodPerSkyrim) : 0.0f;
		}

		// Test 137 ("I see through certain walls"): Skyrim's camera near distance (Display INI).
		std::string NearDistanceText()
		{
			auto* ini = RE::INISettingCollection::GetSingleton();
			auto* s = ini ? ini->GetSetting("fNearDistance:Display") : nullptr;
			return s ? fmt::format("{:.1f} (Garry's Mod units {:.1f})", s->GetFloat(), s->GetFloat() * kGmodPerSkyrim) : std::string("not found");
		}

		std::string ControllerText(RE::PlayerCharacter* a_player)
		{
			auto* c = a_player->GetCharController();
			if (!c) {
				return "no character controller";
			}
			const float k = RE::bhkWorld::GetWorldScaleInverse();
			ControllerRead r;
			if (!ReadController(c, k, r)) {
				return "controller read FAULTED";
			}
			std::string shapes;
			for (int i = 0; i < 2; ++i) {
				shapes += fmt::format("; shape {}: type {} box {:.1f} x {:.1f} x {:.1f} (Garry's Mod units {:.1f} x {:.1f} x {:.1f})", i, r.type[i],
					r.size[i][0], r.size[i][1], r.size[i][2], r.size[i][0] * kGmodPerSkyrim, r.size[i][1] * kGmodPerSkyrim, r.size[i][2] * kGmodPerSkyrim);
			}
			return fmt::format("radius {:.3f} height {:.3f} actorHeight {:.3f} scale {:.3f} center {:.3f} (raw fields; Havok x{:.2f} = Skyrim units) | collision bound centre ({:.1f}, {:.1f}, {:.1f}) extents ({:.1f}, {:.1f}, {:.1f}) | bumper bound centre ({:.1f}, {:.1f}, {:.1f}) extents ({:.1f}, {:.1f}, {:.1f}){}",
				r.radius, r.height, r.actorHeight, r.scale, r.center, k, r.bound[0], r.bound[1], r.bound[2], r.bound[3], r.bound[4], r.bound[5],
				r.bumper[0], r.bumper[1], r.bumper[2], r.bumper[3], r.bumper[4], r.bumper[5], shapes) +
				fmt::format(" | radius sent to Garry's Mod {:.2f} | camera near distance {}", RadiusGmod(a_player), NearDistanceText());
		}

		void PublishOrigin(RE::PlayerCharacter* a_player, const RE::NiPoint3& a_pos)
		{
			auto* ws = a_player->GetWorldspace();
			// Continue from the epoch already in shared memory: Garry's Mod may still be anchored
			// to an epoch published by an earlier Skyrim session, and must see a new number.
			GmskyOrigin cur{};
			uint32_t last = g_origin.epoch;
			if (SlotRead(Link::Get().Shared().origin, cur)) {
				last = std::max(last, cur.epoch);
			}
			GmskyOrigin o{};
			o.epoch = last + 1;
			o.worldspace_form_id = ws ? ws->GetFormID() : 0;
			o.skyrim_origin[0] = a_pos.x;
			o.skyrim_origin[1] = a_pos.y;
			o.skyrim_origin[2] = a_pos.z;
			SlotWrite(Link::Get().Shared().origin, o);
			g_origin = o;
			logger::info("published origin epoch {} at ({:.1f}, {:.1f}, {:.1f}) worldspace {:08X}",
				o.epoch, a_pos.x, a_pos.y, a_pos.z, o.worldspace_form_id);
		}

		void LogSecond(RE::PlayerCharacter* a_player, const RE::NiPoint3& a_pos, float a_seconds)
		{
			auto& link = Link::Get();
			const double heading = WrapRad(a_player->data.angle.z) * kRadToDeg;
			const double pitch = a_player->data.angle.x * kRadToDeg;
			const float height = a_player->GetHeight();
			float eye = 0.0f;
			if (auto* cam = RE::PlayerCamera::GetSingleton(); cam && cam->cameraRoot) {
				eye = cam->cameraRoot->world.translate.z - a_pos.z;
			}
			auto* cell = a_player->GetParentCell();
			auto* ws = a_player->GetWorldspace();

			std::string move = "standing";
			if (g_haveLastLogPos) {
				const double dx = a_pos.x - g_lastLogPos.x, dy = a_pos.y - g_lastLogPos.y;
				const double speed = std::sqrt(dx * dx + dy * dy) / a_seconds;
				if (speed > 30.0) {
					const double mv = SkyrimMoveHeadingDeg(dx, dy);
					move = fmt::format("moving heading={:.1f} speed={:.0f} facing-minus-moving={:.1f}",
						mv, speed, AngleDiffDeg(heading, mv));
				}
			}
			g_lastLogPos = a_pos;
			g_haveLastLogPos = true;

			logger::info("SKYRIM frame={} pos=({:.1f}, {:.1f}, {:.1f}) heading={:.1f} pitch={:.1f} height={:.1f} camera-minus-feet={:.1f} cell={:08X} ws={:08X} | {}",
				g_frame, a_pos.x, a_pos.y, a_pos.z, heading, pitch, height, eye,
				cell ? cell->GetFormID() : 0, ws ? ws->GetFormID() : 0, move);
			// Phase 4a (world audit): where the player is, and water there (Garry's Mod knows nothing of
			// Skyrim's water yet).
			if (cell) {
				// Step 4b: the height sent to Garry's Mod (the player's water height) next to the
				// character controller's and the cell's, to compare.
				const auto valid = [](float a_z) { return a_z > -1.0e6f && a_z < 1.0e6f; };
				const auto show = [&](float a_z) { return valid(a_z) ? fmt::format("{:.1f}", a_z) : std::string("none"); };
				const float water = a_player->GetWaterHeight();
				float cellWater = 0.0f;
				if (!cell->GetWaterHeight(a_pos, cellWater)) {
					cellWater = -1.0e9f;
				}
				auto* controller = a_player->GetCharController();
				const auto* state = a_player->AsActorState();
				logger::info("WORLD {} '{}' in '{}' | water sent {} ({}) | controller's {} cell's {} | swimming {} | puppet {}",
					cell->IsInteriorCell() ? "interior" : "exterior", cell->GetFullName(), ws ? ws->GetFullName() : "-", show(water),
					valid(water) ? fmt::format("feet {:.1f} {} it", std::abs(a_pos.z - water), a_pos.z < water ? "below" : "above") : std::string("-"),
					controller ? show(controller->waterHeight) : std::string("-"), show(cellWater),
					state && state->IsSwimming() ? "yes" : "no", Puppet::IsActive() ? "on" : "off");
			}
			logger::info("  player collision (size check after test 136): {}", ControllerText(a_player));
			if (height > 0.0f) {
				logger::info("  scale check: gmod standing eye 64 / skyrim camera-minus-feet {:.1f} = {:.4f}; gmod hull 72 / skyrim height {:.1f} = {:.4f}; current guess {:.4f}",
					eye, eye > 0.0f ? 64.0 / eye : 0.0, height, 72.0 / height, kGmodPerSkyrim);
			}

			GmskyPlayerState g{};
			uint32_t seq = 0;
			if (!SlotRead(link.Shared().player_state, g, &seq)) {
				logger::info("GMOD  no player state yet (alive={})", link.PeerAlive());
				return;
			}
			const double gHeading = GmodYawToSkyrimHeading(g.yaw_deg) * kRadToDeg;
			std::string gmove = "standing";
			const double gspeed = std::sqrt(double(g.vel[0]) * g.vel[0] + double(g.vel[1]) * g.vel[1]);
			if (gspeed > 30.0) {
				// Garry's Mod convention (assumed): yaw 0 = +X, counter-clockwise, so moving along
				// (vx, vy) has yaw atan2(vy, vx).
				const double gMoveYaw = std::atan2(double(g.vel[1]), double(g.vel[0])) * kRadToDeg;
				gmove = fmt::format("moving yaw={:.1f} speed={:.0f} facing-minus-moving={:.1f}",
					gMoveYaw, gspeed, AngleDiffDeg(g.yaw_deg, gMoveYaw));
			}
			logger::info("GMOD  alive={} tick={} epoch={} pos=({:.1f}, {:.1f}, {:.1f}) yaw={:.1f} pitch={:.1f} eye={:.1f} flags={:#x} | {}",
				link.PeerAlive(), g.tick, g.origin_epoch, g.pos[0], g.pos[1], g.pos[2], g.yaw_deg, g.pitch_deg,
				g.eye_height, g.flags, gmove);
			// Garry's Mod's anchor belongs to g.origin_epoch. Converting with any other origin
			// (it hasn't seen our newest one yet, or none at all) gives a wrong position.
			if (g.origin_epoch == 0 || g.origin_epoch != g_origin.epoch) {
				logger::info("  as skyrim: not anchored to the current origin yet (gmod epoch {}, current {})",
					g.origin_epoch, g_origin.epoch);
				return;
			}
			const auto sk = GmodToSkyrim(g.pos, g.anchor, g_origin.skyrim_origin);
			logger::info("  as skyrim: pos=({:.1f}, {:.1f}, {:.1f}) heading={:.1f} pitch={:.1f}",
				sk.x, sk.y, sk.z, gHeading, GmodPitchToSkyrimPitch(g.pitch_deg) * kRadToDeg);
		}
	}

	void OnGameLoaded()
	{
		g_needOrigin = true;
		g_haveLastLogPos = false;
		Puppet::Reset("game loaded");
		Terrain::Reset();
		Collision::Reset();
		Arms::Reset("game loaded");
		logger::info("game loaded: will publish a new origin on the next frame");
	}

	float PlayerHalfWidth() { return g_halfWidth; }

	void PerFrame(RE::PlayerCharacter* a_player, float a_delta)
	{
		auto& link = Link::Get();
		if (!a_player) {
			return;
		}
		if (!link.IsOpen()) {
			const auto now = GetTickCount64();
			if (now - g_lastOpenTry < 2000) {
				return;
			}
			g_lastOpenTry = now;
			if (!link.Open()) {
				return;
			}
		}
		if (!link.OwnsSide()) {
			if (!g_lostOwnershipLogged) {
				logger::error("another Skyrim-side process (pid {}) is writing the link; not writing until it exits",
					link.Shared().header.skyrim_pid);
				g_lostOwnershipLogged = true;
			}
			return;
		}
		g_lostOwnershipLogged = false;
		++g_frame;
		link.Heartbeat();

		const auto pos = a_player->GetPosition();
		auto* cell = a_player->GetParentCell();

		if (cell) {
			auto* ws = a_player->GetWorldspace();
			const uint32_t place = cell->IsInteriorCell() ? cell->GetFormID() : (ws ? ws->GetFormID() : 0);
			if (g_placeKey != 0 && place != g_placeKey) {
				logger::info("player changed location ({:08X} -> {:08X}): new origin", g_placeKey, place);
				g_needOrigin = true;
				Puppet::Reset("location changed, waiting for the new origin");
			}
			g_placeKey = place;
		}
		if (Puppet::SkyrimMovedPlayer(pos)) {
			logger::info("Skyrim moved the player itself (to ({:.1f}, {:.1f}, {:.1f})): new origin", pos.x, pos.y, pos.z);
			g_needOrigin = true;
			Puppet::Reset("Skyrim moved the player, waiting for the new origin");
		}

		GmskySkyrimState s{};
		s.frame = g_frame;
		s.pos[0] = pos.x;
		s.pos[1] = pos.y;
		s.pos[2] = pos.z;
		s.heading_rad = a_player->data.angle.z;
		s.pitch_rad = a_player->data.angle.x;
		s.height = a_player->GetHeight();
		s.radius = RadiusGmod(a_player);
		g_halfWidth = float(s.radius / kGmodPerSkyrim);
		s.cell_form_id = cell ? cell->GetFormID() : 0;
		s.flags = Combat::PlayerInCombat() ? GMSKY_SF_IN_COMBAT : 0u;
		// Step 4b: the water surface where the player is (Skyrim's own "relevant water height" for
		// the player, else the cell's water level), for Garry's Mod's swimming.
		if (const float water = a_player->GetWaterHeight(); water > -1.0e6f && water < 1.0e6f && g_origin.epoch != 0) {
			s.flags |= GMSKY_SF_WATER;
			s.water_z = float((double(water) - g_origin.skyrim_origin[2]) * kGmodPerSkyrim);
		}
		SlotWrite(link.Shared().skyrim_state, s);

		if (g_needOrigin && cell) {
			PublishOrigin(a_player, pos);
			g_needOrigin = false;
		}

		Puppet::Update(a_player, g_origin, g_frame);
		Input::PerFrame(g_frame);
		Camera::AfterPlayerUpdate();
		Terrain::PerFrame(a_player, g_origin, g_frame);
		Collision::PerFrame(a_player, g_origin);
		Loose::PerFrame(a_player, g_origin, a_delta);
		Overlay::PerFrame();
		Arms::PerFrame(a_player);
		Hud::PerFrame(a_player, Arms::Live());
		Actors::PerFrame(a_player, g_origin, g_frame);
		Hitboxes::PerFrame(a_player, g_origin, g_frame);
		Combat::PerFrame(a_player, g_origin, Puppet::IsActive(), a_delta);

		g_logTimer += a_delta;
		if (g_logTimer >= 1.0f) {
			LogSecond(a_player, pos, g_logTimer);
			Puppet::LogSecond();
			Input::LogSecond();
			Camera::LogSecond();
			Terrain::LogSecond(a_player);
			Collision::LogSecond();
			Loose::LogSecond(a_player);
			Loose::LogSending();
			Body::LogSecond();
			Overlay::LogSecond();
			Draw::LogSecond();
			World::LogSecond();
			Render::LogSecond();
			Arms::LogSecond();
			Hud::LogSecond();
			Actors::LogSecond();
			Hitboxes::LogSecond();
			Combat::LogSecond();
			g_logTimer = 0.0f;
		}
	}

	namespace
	{
		struct PlayerUpdateHook
		{
			static void thunk(RE::PlayerCharacter* a_this, float a_delta)
			{
				func(a_this, a_delta);
				try {
					PerFrame(a_this, a_delta);
				} catch (const std::exception& e) {
					logger::error("per-frame update: {}", e.what());
				}
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};
	}

	void Install()
	{
		// PlayerCharacter::Update(float delta) is entry 0xAD of the player's function table
		// (same slot SkyCraft hooks on 1.7.104).
		REL::Relocation<std::uintptr_t> vtbl{ RE::VTABLE_PlayerCharacter[0] };
		PlayerUpdateHook::func = vtbl.write_vfunc(0xAD, PlayerUpdateHook::thunk);
		logger::info("hooked PlayerCharacter::Update");
	}
}
