// The per-frame driver. Every frame: publish Skyrim's player state, manage the origin (a new one
// on game load, location change, or when Skyrim moves the player itself), and run the puppet
// (Puppet.cpp). Once a second, log Skyrim's numbers next to Garry's Mod's.
#include "Probe.h"

#include "Arms.h"
#include "Body.h"
#include "Camera.h"
#include "Collision.h"
#include "Draw.h"
#include "Hud.h"
#include "Input.h"
#include "Link.h"
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
		s.cell_form_id = cell ? cell->GetFormID() : 0;
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
		Overlay::PerFrame();
		Arms::PerFrame(a_player);
		Hud::PerFrame(a_player, Arms::Live());

		g_logTimer += a_delta;
		if (g_logTimer >= 1.0f) {
			LogSecond(a_player, pos, g_logTimer);
			Puppet::LogSecond();
			Input::LogSecond();
			Camera::LogSecond();
			Terrain::LogSecond(a_player);
			Collision::LogSecond();
			Body::LogSecond();
			Overlay::LogSecond();
			Draw::LogSecond();
			Arms::LogSecond();
			Hud::LogSecond();
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
