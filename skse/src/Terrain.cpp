// Phase 1 step 3b: Skyrim's ground around the player, for Garry's Mod's collision.
//
// SkyCraft's "stage A": a grid of rays straight down through Skyrim's physics world (terrain, rocks,
// buildings' roofs), 65 x 65 points 48 Skyrim units apart, centred on the player. The rays are
// spread over frames (a time budget per frame); the grid is published only when complete. A new
// grid starts when the player is more than 768 units from the last centre, or the origin changes.
// Only the top surface of each column is seen (no arches or interiors yet), as SkyCraft's stage A.
#include "Terrain.h"

#include "Link.h"
#include "Puppet.h"

namespace gmsky::Terrain
{
	namespace
	{
		constexpr int kN = GMSKY_TERRAIN_N;
		constexpr float kSpacing = 48.0f;    // Skyrim units between points
		constexpr float kRecenter = 768.0f;  // new grid when the player is this far from the last centre
		constexpr float kRayAbove = 4000.0f;  // rays run from this far above the player's feet...
		constexpr float kRayBelow = 4000.0f;  // ...to this far below
		constexpr double kBudgetMs = 1.5;    // ray casting time per frame

		const double kQpcPerMs = [] {
			LARGE_INTEGER f;
			QueryPerformanceFrequency(&f);
			return double(f.QuadPart) / 1000.0;
		}();

		double NowMs()
		{
			LARGE_INTEGER c;
			QueryPerformanceCounter(&c);
			return double(c.QuadPart) / kQpcPerMs;
		}

		struct Job
		{
			bool running = false;
			uint32_t epoch = 0;
			double cx = 0.0, cy = 0.0;  // Skyrim x, y of the centre point
			float refZ = 0.0f;          // the player's feet when the grid started
			int next = 0;               // next point to cast
			uint32_t misses = 0, actorHits = 0, frames = 0;
			double castMs = 0.0, startMs = 0.0;
			std::array<float, kN * kN> z{};  // Skyrim z, NaN = no hit
		};
		Job g_job;

		// The last published grid, in Skyrim units, for the comparison in the log.
		struct Done
		{
			bool valid = false;
			uint32_t epoch = 0;
			double cx = 0.0, cy = 0.0;
			std::array<float, kN * kN> z{};
		};
		Done g_done;

		GmskyTerrain g_out{};  // what goes to Garry's Mod (17 KB, kept off the stack)
		uint32_t g_nextId = 1;

		// Per second.
		uint32_t g_published = 0, g_secRays = 0;
		double g_secCastMs = 0.0;
		std::string g_lastGrid = "none yet";

		// One ray straight down. Uses the player's collision group so it never hits the player.
		bool CastDown(RE::PlayerCharacter* a_player, float a_x, float a_y, float a_zTop, float a_zBottom, float& a_z, bool& a_actor)
		{
			auto* tes = RE::TES::GetSingleton();
			if (!tes) {
				return false;
			}
			const float s = RE::bhkWorld::GetWorldScale();
			RE::bhkPickData pick;
			pick.rayInput.from = RE::hkVector4(a_x * s, a_y * s, a_zTop * s, 0.0f);
			pick.rayInput.to = RE::hkVector4(a_x * s, a_y * s, a_zBottom * s, 0.0f);
			RE::CFilter filter;
			a_player->GetCollisionFilterInfo(filter);
			filter.SetCollisionLayer(RE::COL_LAYER::kLOS);
			pick.rayInput.filterInfo = filter;
			tes->Pick(pick);
			if (!pick.rayOutput.HasHit()) {
				return false;
			}
			a_z = a_zTop - (a_zTop - a_zBottom) * pick.rayOutput.hitFraction;
			auto* ref = RE::TESHavokUtilities::FindCollidableRef(*pick.rayOutput.rootCollidable);
			a_actor = ref && ref->As<RE::Actor>();
			return true;
		}

		void Publish(const GmskyPlayerState& a_g, const GmskyOrigin& a_origin, uint64_t a_frame)
		{
			g_out.id = g_nextId++;
			g_out.origin_epoch = g_job.epoch;
			g_out.n = kN;
			g_out.misses = g_job.misses;
			const double c[3] = { g_job.cx, g_job.cy, 0.0 };
			const auto gc = SkyrimToGmod(c, a_g.anchor, a_origin.skyrim_origin);
			g_out.center[0] = gc.x;
			g_out.center[1] = gc.y;
			g_out.spacing = float(kSpacing * kGmodPerSkyrim);
			g_out.skyrim_frame = a_frame;
			float zMin = std::numeric_limits<float>::max(), zMax = std::numeric_limits<float>::lowest();
			for (int k = 0; k < kN * kN; ++k) {
				const float z = g_job.z[k];
				if (std::isnan(z)) {
					g_out.height[k] = std::numeric_limits<float>::quiet_NaN();
					continue;
				}
				const double p[3] = { 0.0, 0.0, z };
				g_out.height[k] = float(SkyrimToGmod(p, a_g.anchor, a_origin.skyrim_origin).z);
				zMin = std::min(zMin, z);
				zMax = std::max(zMax, z);
			}
			SlotWrite(Link::Get().Shared().terrain, g_out);

			g_done.valid = true;
			g_done.epoch = g_job.epoch;
			g_done.cx = g_job.cx;
			g_done.cy = g_job.cy;
			g_done.z = g_job.z;
			++g_published;
			const int rays = kN * kN;
			g_lastGrid = fmt::format("grid {} at ({:.0f}, {:.0f}) epoch {}: {} rays in {:.1f} ms casting ({:.1f} us each) over {} frames, {:.0f} ms from start; misses={} actor hits={}; ground z {:.1f}..{:.1f}",
				g_out.id, g_job.cx, g_job.cy, g_job.epoch, rays, g_job.castMs, 1000.0 * g_job.castMs / rays, g_job.frames,
				NowMs() - g_job.startMs, g_job.misses, g_job.actorHits, zMin, zMax);
			logger::info("TERRAIN published {}", g_lastGrid);
			g_job.running = false;
		}

		// The grid's height at (x, y), bilinear between its four points; NaN outside or on a miss.
		float GridZ(float a_x, float a_y)
		{
			const double half = (kN - 1) * 0.5;
			const double fi = (a_x - g_done.cx) / kSpacing + half, fj = (a_y - g_done.cy) / kSpacing + half;
			const int i = int(std::floor(fi)), j = int(std::floor(fj));
			if (i < 0 || j < 0 || i >= kN - 1 || j >= kN - 1) {
				return std::numeric_limits<float>::quiet_NaN();
			}
			const float u = float(fi - i), v = float(fj - j);
			const auto at = [](int a, int b) { return g_done.z[a * kN + b]; };
			return (1 - u) * (1 - v) * at(i, j) + u * (1 - v) * at(i + 1, j) + u * v * at(i + 1, j + 1) + (1 - u) * v * at(i, j + 1);
		}
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame)
	{
		if (!Puppet::IsActive() || !a_player) {
			g_job.running = false;
			return;
		}
		GmskyPlayerState g{};
		if (!SlotRead(Link::Get().Shared().player_state, g) || g.origin_epoch != a_origin.epoch) {
			return;
		}
		const auto pos = a_player->GetPosition();
		if (g_job.running && g_job.epoch != a_origin.epoch) {
			g_job.running = false;  // the origin changed under the grid: start again
		}
		if (!g_job.running) {
			const bool stale = !g_done.valid || g_done.epoch != a_origin.epoch ||
			                   std::hypot(pos.x - g_done.cx, pos.y - g_done.cy) > kRecenter;
			if (!stale) {
				return;
			}
			g_job = Job{};
			g_job.running = true;
			g_job.epoch = a_origin.epoch;
			// Centre on a multiple of the spacing, so grids overlap point for point.
			g_job.cx = std::round(pos.x / kSpacing) * kSpacing;
			g_job.cy = std::round(pos.y / kSpacing) * kSpacing;
			g_job.refZ = pos.z;
			g_job.startMs = NowMs();
		}

		const double t0 = NowMs();
		const double half = (kN - 1) * 0.5;
		int cast = 0;
		while (g_job.next < kN * kN && (cast < 16 || NowMs() - t0 < kBudgetMs)) {
			const int i = g_job.next / kN, j = g_job.next % kN;
			const float x = float(g_job.cx + (i - half) * kSpacing), y = float(g_job.cy + (j - half) * kSpacing);
			float z = 0.0f;
			bool actor = false;
			if (CastDown(a_player, x, y, g_job.refZ + kRayAbove, g_job.refZ - kRayBelow, z, actor)) {
				g_job.z[g_job.next] = z;
				g_job.actorHits += actor ? 1 : 0;
			} else {
				g_job.z[g_job.next] = std::numeric_limits<float>::quiet_NaN();
				++g_job.misses;
			}
			++g_job.next;
			++cast;
		}
		const double ms = NowMs() - t0;
		g_job.castMs += ms;
		++g_job.frames;
		g_secRays += cast;
		g_secCastMs += ms;
		if (g_job.next >= kN * kN) {
			Publish(g, a_origin, a_frame);
		}
	}

	void Reset()
	{
		g_job.running = false;
		g_done.valid = false;
	}

	void LogSecond(RE::PlayerCharacter* a_player)
	{
		if (!g_done.valid && !g_job.running && g_secRays == 0) {
			return;
		}
		std::string under = "-";
		if (g_done.valid && a_player) {
			const auto pos = a_player->GetPosition();
			const float gz = GridZ(pos.x, pos.y);
			under = std::isnan(gz) ? "outside the grid or a miss" :
			                         fmt::format("{:.1f} (feet {:.1f}, grid minus feet {:.1f}), {:.0f} units from its centre",
			                             gz, pos.z, gz - pos.z, std::hypot(pos.x - g_done.cx, pos.y - g_done.cy));
		}
		logger::info("TERRAIN this second: {} rays in {:.1f} ms, {} grid(s) published{} | grid height under the player: {} | last: {}",
			g_secRays, g_secCastMs, g_published, g_job.running ? fmt::format(", next grid {}/{} rays", g_job.next, kN * kN) : "",
			under, g_lastGrid);
		g_secRays = 0;
		g_secCastMs = 0.0;
		g_published = 0;
	}
}
