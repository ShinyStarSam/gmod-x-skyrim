// Phase 0 step 6: Skyrim's player follows Garry's Mod's player.
//
// Garry's Mod decides left/right/forward position and where the player looks. Height comes from
// Skyrim's own ground under that spot (a downward ray cast), because Garry's Mod still stands on a
// flat test floor; Phase 1 replaces that with Skyrim's real shape inside Garry's Mod.
// Approach as in SkyCraft: set the position after Skyrim's own update every frame, and zero the
// character controller's own velocity and fall tracking so Skyrim adds no momentum or fall damage.
#include "Puppet.h"

#include "Input.h"
#include "Link.h"

namespace gmsky::Puppet
{
	namespace
	{
		constexpr float kRayUp = 100.0f;    // start the ground ray this far above the current feet
		constexpr float kRayDown = 600.0f;  // and end it this far below
		constexpr float kTeleportThreshold = 300.0f;
		constexpr double kRadToDeg = 180.0 / kPi;

		bool g_active = false;
		std::string g_offReason = "not started";
		RE::NiPoint3 g_lastSet{};
		bool g_haveLastSet = false;

		// Per-second statistics for the log.
		uint32_t g_sets = 0, g_groundRay = 0, g_groundLand = 0, g_groundKeep = 0, g_bigSteps = 0;
		float g_maxStep = 0.0f, g_maxDrift = 0.0f;
		RE::NiPoint3 g_lastTargetRaw{};
		double g_lastHeadingDeg = 0.0, g_lastPitchDeg = 0.0;
		// View lag and prediction (Phase 1 step 1b), per second.
		uint32_t g_lagSamples = 0, g_lagMax = 0, g_predicted = 0;
		uint64_t g_lagSum = 0;
		double g_aheadMax = 0.0, g_aheadLast = 0.0;
		float g_degPerCount = 0.0f;

		// Garry's Mod's view plus the mouse movement Skyrim already sent but Garry's Mod hasn't
		// applied yet, with Garry's Mod's own formula and pitch limit. Garry's Mod still decides
		// the view: once it has applied everything, the extra is zero.
		// a_stats = false: same result, without counting it in the per-second statistics.
		void PredictView(const GmskyPlayerState& a_g, uint64_t a_frame, double& a_yaw, double& a_pitch, bool a_stats = true)
		{
			a_yaw = a_g.yaw_deg;
			a_pitch = a_g.pitch_deg;
			if (a_stats) {
				g_aheadLast = 0.0;
			}
			if (!(a_g.flags & GMSKY_PF_INPUT_APPLIED)) {
				return;
			}
			if (a_stats) {
				// How many Skyrim frames old the input in Garry's Mod's view is (1 = the newest possible).
				const uint32_t lag = a_frame > a_g.input_frame ? uint32_t(a_frame - a_g.input_frame) : 0;
				++g_lagSamples;
				g_lagSum += lag;
				g_lagMax = std::max(g_lagMax, lag);
				g_degPerCount = a_g.deg_per_count;
			}

			int64_t mx = 0, my = 0;
			if (!Input::MouseTotals(mx, my) || a_g.deg_per_count <= 0.0f) {
				return;
			}
			const int64_t dx = mx - a_g.mouse_x_applied, dy = my - a_g.mouse_y_applied;
			if (std::abs(dx) > 100000 || std::abs(dy) > 100000) {
				return;  // totals don't belong together (restart): show Garry's Mod's view as is
			}
			const double k = a_g.deg_per_count;
			a_yaw -= double(dx) * k;
			a_pitch = std::clamp(a_pitch + double(dy) * k, -89.0, 89.0);
			if (a_stats) {
				g_aheadLast = std::max(std::abs(double(dx) * k), std::abs(a_pitch - a_g.pitch_deg));
				g_aheadMax = std::max(g_aheadMax, g_aheadLast);
				++g_predicted;
			}
		}

		bool Off(const std::string& a_reason)
		{
			if (g_active || a_reason != g_offReason) {
				logger::info("PUPPET OFF: {}", a_reason);
			}
			g_active = false;
			g_offReason = a_reason;
			return false;
		}

		enum class Ground
		{
			kRay,
			kLand,
			kKeep
		};

		// Skyrim's ground height under (x, y): a ray straight down through Skyrim's physics world
		// (rocks, roads, buildings, terrain). The ray uses the player's own collision group so it
		// doesn't hit the player's own capsule. Falls back to the terrain height, then to a_refZ.
		float GroundZ(RE::PlayerCharacter* a_player, float a_x, float a_y, float a_refZ, Ground& a_how)
		{
			auto* tes = RE::TES::GetSingleton();
			if (tes) {
				const float s = RE::bhkWorld::GetWorldScale();
				RE::bhkPickData pick;
				pick.rayInput.from = RE::hkVector4(a_x * s, a_y * s, (a_refZ + kRayUp) * s, 0.0f);
				pick.rayInput.to = RE::hkVector4(a_x * s, a_y * s, (a_refZ - kRayDown) * s, 0.0f);
				RE::CFilter filter;
				a_player->GetCollisionFilterInfo(filter);
				filter.SetCollisionLayer(RE::COL_LAYER::kLOS);
				pick.rayInput.filterInfo = filter;
				tes->Pick(pick);
				if (pick.rayOutput.HasHit()) {
					a_how = Ground::kRay;
					return (a_refZ + kRayUp) - (kRayUp + kRayDown) * pick.rayOutput.hitFraction;
				}
				float land = 0.0f;
				if (tes->GetLandHeight(RE::NiPoint3{ a_x, a_y, a_refZ }, land)) {
					a_how = Ground::kLand;
					return land;
				}
			}
			a_how = Ground::kKeep;
			return a_refZ;
		}
	}

	bool SkyrimMovedPlayer(const RE::NiPoint3& a_pos)
	{
		return g_active && g_haveLastSet && a_pos.GetDistance(g_lastSet) > kTeleportThreshold;
	}

	bool IsActive()
	{
		return g_active;
	}

	void Reset(const char* a_reason)
	{
		Off(a_reason);
		g_haveLastSet = false;
	}

	bool Update(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame)
	{
		auto& link = Link::Get();
		if (!link.PeerAlive()) {
			return Off("Garry's Mod not connected (no heartbeat)");
		}
		GmskyPlayerState g{};
		if (!SlotRead(link.Shared().player_state, g)) {
			return Off("no Garry's Mod player state");
		}
		if (a_origin.epoch == 0 || g.origin_epoch != a_origin.epoch) {
			return Off(fmt::format("waiting for Garry's Mod to anchor to origin epoch {} (it has {})", a_origin.epoch, g.origin_epoch));
		}
		if (!(g.flags & GMSKY_PF_ALIVE)) {
			return Off("Garry's Mod player is dead");
		}
		if (!a_player->GetParentCell() || !a_player->Is3DLoaded()) {
			return Off("Skyrim player not loaded");
		}
		if (!g_active) {
			logger::info("PUPPET ON: Skyrim's player follows Garry's Mod (origin epoch {})", a_origin.epoch);
			g_active = true;
			g_offReason.clear();
		}

		const auto cur = a_player->GetPosition();
		if (g_haveLastSet) {
			// How far Skyrim itself moved the player since our last set (gravity, collision push).
			const float dx = cur.x - g_lastSet.x, dy = cur.y - g_lastSet.y;
			g_maxDrift = std::max(g_maxDrift, std::sqrt(dx * dx + dy * dy));
		}

		const auto t = GmodToSkyrim(g.pos, g.anchor, a_origin.skyrim_origin);
		g_lastTargetRaw = RE::NiPoint3{ float(t.x), float(t.y), float(t.z) };
		Ground how;
		const float z = GroundZ(a_player, float(t.x), float(t.y), cur.z, how);
		(how == Ground::kRay ? g_groundRay : how == Ground::kLand ? g_groundLand : g_groundKeep)++;
		const RE::NiPoint3 pos{ float(t.x), float(t.y), z };

		if (g_haveLastSet) {
			const float step = pos.GetDistance(g_lastSet);
			g_maxStep = std::max(g_maxStep, step);
			if (step > kTeleportThreshold) {
				++g_bigSteps;
				logger::info("PUPPET big step {:.0f} units: ({:.1f}, {:.1f}, {:.1f}) -> ({:.1f}, {:.1f}, {:.1f})",
					step, g_lastSet.x, g_lastSet.y, g_lastSet.z, pos.x, pos.y, pos.z);
			}
		}

		a_player->SetPosition(pos, true);
		double yaw = 0.0, gpitch = 0.0;
		PredictView(g, a_frame, yaw, gpitch);
		const double heading = GmodYawToSkyrimHeading(yaw);
		const double pitch = GmodPitchToSkyrimPitch(gpitch);
		a_player->data.angle.z = float(heading);
		a_player->data.angle.x = float(pitch);
		if (auto* controller = a_player->GetCharController()) {
			// Garry's Mod moves the player; Skyrim keeps no momentum or fall damage of its own.
			controller->SetLinearVelocityImpl(RE::hkVector4(0.0f, 0.0f, 0.0f, 0.0f));
			controller->fallStartHeight = pos.z;
			controller->fallTime = 0.0f;
		}

		g_lastSet = pos;
		g_haveLastSet = true;
		g_lastHeadingDeg = heading * kRadToDeg;
		g_lastPitchDeg = pitch * kRadToDeg;
		++g_sets;
		return true;
	}

	bool ViewNow(double& a_headingRad, double& a_pitchRad, bool& a_driveCamera)
	{
		if (!g_active) {
			return false;
		}
		GmskyPlayerState g{};
		if (!SlotRead(Link::Get().Shared().player_state, g)) {
			return false;
		}
		double yaw = 0.0, pitch = 0.0;
		PredictView(g, 0, yaw, pitch, false);
		a_headingRad = GmodYawToSkyrimHeading(yaw);
		a_pitchRad = GmodPitchToSkyrimPitch(pitch);
		a_driveCamera = (g.flags & GMSKY_PF_DRIVE_CAMERA) != 0;
		return true;
	}

	void LogSecond()
	{
		if (!g_active) {
			logger::info("PUPPET off: {}", g_offReason);
		} else {
			logger::info("PUPPET on: sets={} ground ray/land/keep={}/{}/{} set=({:.1f}, {:.1f}, {:.1f}) gmod-as-skyrim z={:.1f} heading={:.1f} pitch={:.1f} max-step={:.1f} max-drift={:.1f} big-steps={}",
				g_sets, g_groundRay, g_groundLand, g_groundKeep, g_lastSet.x, g_lastSet.y, g_lastSet.z, g_lastTargetRaw.z,
				g_lastHeadingDeg, g_lastPitchDeg, g_maxStep, g_maxDrift, g_bigSteps);
			if (g_lagSamples) {
				logger::info("VIEW lag (Skyrim frames, 1 = newest) avg={:.2f} max={} | prediction {}: frames={} ahead max={:.2f} deg, last frame {:.2f} deg",
					double(g_lagSum) / g_lagSamples, g_lagMax, g_degPerCount > 0.0f ? "on" : "off (gmsky_predict 0)",
					g_predicted, g_aheadMax, g_aheadLast);
			}
		}
		g_lagSamples = g_lagMax = g_predicted = 0;
		g_lagSum = 0;
		g_aheadMax = 0.0;
		g_sets = g_groundRay = g_groundLand = g_groundKeep = g_bigSteps = 0;
		g_maxStep = g_maxDrift = 0.0f;
	}
}
