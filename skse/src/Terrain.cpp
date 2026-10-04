// Phase 1 step 3b: Skyrim's ground around the player, for Garry's Mod's collision.
//
// SkyCraft's "stage A": a grid of rays straight down through Skyrim's physics world (terrain, rocks,
// buildings' roofs), 65 x 65 points 48 Skyrim units apart, centred on the player. The rays are
// spread over frames (a time budget per frame); the grid is published only when complete.
// Grid centres sit on a fixed lattice of 768 units (one Garry's Mod piece of 16 x 16 squares), so
// the pieces of different grids line up exactly and Garry's Mod reuses them instead of building
// overlapping copies (test 13: inserting a mesh over an existing one crashed its physics). A new
// grid starts when the lattice point nearest where the player will be changes, the origin changes,
// or the player's height moved far from the last grid's (noclip flying, step 3e). "Where the
// player will be": its position plus its velocity times the recent cast time (test 20: a grid
// centred where a fast player was when casting started arrived behind it).
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
		constexpr float kLattice = 16 * kSpacing;  // grid centres are multiples of this (768)
		constexpr float kRayAbove = 4000.0f;   // rays run from this far above the player's feet...
		constexpr float kRayBelow = 20000.0f;  // ...to this far below (they stop at the first surface;
		                                       // long so a player flying high in noclip still finds ground)
		constexpr float kRecastHeight = 2000.0f;  // new grid when the feet moved this far up or down
		constexpr double kBudgetMs = 1.5;      // ray casting time per frame
		constexpr double kBudgetFastMs = 4.0;  // ... while the player moves faster than kFastSpeed
		constexpr float kFastSpeed = 1000.0f;  // Skyrim units per second
		constexpr double kLeadExtraMs = 100.0;  // lead time beyond the cast time (reaching Garry's Mod)

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
			uint32_t misses = 0, actorHits = 0, actorLeft = 0, frames = 0;
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
			float refZ = 0.0f;
			std::array<float, kN * kN> z{};
		};
		Done g_done;

		GmskyTerrain g_out{};  // what goes to Garry's Mod (17 KB, kept off the stack)
		uint32_t g_nextId = 1;
		double g_castMsAvg = 400.0;  // start-to-publish time of recent grids (test 20: 320-517 ms)
		double g_leadUnits = 0.0;    // how far ahead of the player the running grid is centred

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

		void Publish(const GmskyOrigin& a_origin, uint64_t a_frame)
		{
			g_out.id = g_nextId++;
			g_out.origin_epoch = g_job.epoch;
			g_out.n = kN;
			g_out.misses = g_job.misses;
			// Relative to the anchor (protocol 7): Garry's Mod adds its own current anchor, so a
			// re-centre while this grid is on its way can't put it in the wrong place.
			constexpr double kNoAnchor[3] = { 0.0, 0.0, 0.0 };
			const double c[3] = { g_job.cx, g_job.cy, 0.0 };
			const auto gc = SkyrimToGmod(c, kNoAnchor, a_origin.skyrim_origin);
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
				g_out.height[k] = float(SkyrimToGmod(p, kNoAnchor, a_origin.skyrim_origin).z);
				zMin = std::min(zMin, z);
				zMax = std::max(zMax, z);
			}
			SlotWrite(Link::Get().Shared().terrain, g_out);

			g_done.valid = true;
			g_done.epoch = g_job.epoch;
			g_done.cx = g_job.cx;
			g_done.cy = g_job.cy;
			g_done.refZ = g_job.refZ;
			g_done.z = g_job.z;
			++g_published;
			const int rays = kN * kN;
			const double tookMs = NowMs() - g_job.startMs;
			g_castMsAvg = 0.7 * g_castMsAvg + 0.3 * tookMs;
			g_lastGrid = fmt::format("grid {} at ({:.0f}, {:.0f}) epoch {}, centred {:.0f} units ahead of the player: {} rays in {:.1f} ms casting ({:.1f} us each) over {} frames, {:.0f} ms from start; misses={} actor hits={} (still on an actor after re-casting: {}); ground z {:.1f}..{:.1f}",
				g_out.id, g_job.cx, g_job.cy, g_job.epoch, g_leadUnits, rays, g_job.castMs, 1000.0 * g_job.castMs / rays, g_job.frames,
				tookMs, g_job.misses, g_job.actorHits, g_job.actorLeft, zMin, zMax);
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
		if (g.flags & GMSKY_PF_SHAPES) {
			g_job.running = false;  // Garry's Mod uses Skyrim's collision shapes instead (Collision.cpp)
			return;
		}
		const auto pos = a_player->GetPosition();
		if (g_job.running && g_job.epoch != a_origin.epoch) {
			g_job.running = false;  // the origin changed under the grid: start again
		}
		// Garry's Mod's velocity, in Skyrim units per second (horizontal).
		const double vx = g.vel[0] / kGmodPerSkyrim, vy = g.vel[1] / kGmodPerSkyrim;
		const double speed = std::hypot(vx, vy);
		if (!g_job.running) {
			const double lead = (g_castMsAvg + kLeadExtraMs) / 1000.0;
			const double ax = pos.x + vx * lead, ay = pos.y + vy * lead;
			const double lx = std::round(ax / kLattice) * kLattice, ly = std::round(ay / kLattice) * kLattice;
			const bool moved = !g_done.valid || g_done.epoch != a_origin.epoch || lx != g_done.cx || ly != g_done.cy;
			const bool height = !moved && std::abs(pos.z - g_done.refZ) > kRecastHeight;
			if (!moved && !height) {
				return;
			}
			if (height) {
				logger::info("TERRAIN player height {:.0f} is {:.0f} from the last grid's {:.0f}: casting a new grid", pos.z,
					pos.z - g_done.refZ, g_done.refZ);
			}
			g_job = Job{};
			g_job.running = true;
			g_job.epoch = a_origin.epoch;
			g_job.cx = lx;
			g_job.cy = ly;
			g_job.refZ = pos.z;
			g_job.startMs = NowMs();
			g_leadUnits = speed * lead;
		}

		const double t0 = NowMs();
		const double half = (kN - 1) * 0.5;
		int cast = 0;
		const double budget = speed > kFastSpeed ? kBudgetFastMs : kBudgetMs;
		while (g_job.next < kN * kN && (cast < 16 || NowMs() - t0 < budget)) {
			const int i = g_job.next / kN, j = g_job.next % kN;
			const float x = float(g_job.cx + (i - half) * kSpacing), y = float(g_job.cy + (j - half) * kSpacing);
			float z = 0.0f;
			bool actor = false;
			bool hit = CastDown(a_player, x, y, g_job.refZ + kRayAbove, g_job.refZ - kRayBelow, z, actor);
			// An NPC or animal isn't ground (test 11: up to 8 rays per grid hit one). Cast again from
			// just inside it: a ray starting inside a shape passes out of it, down to what's below.
			for (int retry = 0; hit && actor && retry < 3; ++retry) {
				++g_job.actorHits;
				hit = CastDown(a_player, x, y, z - 1.0f, g_job.refZ - kRayBelow, z, actor);
			}
			if (hit) {
				g_job.z[g_job.next] = z;
				g_job.actorLeft += actor ? 1 : 0;
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
			Publish(a_origin, a_frame);
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
