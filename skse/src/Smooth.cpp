// Phase 1 step 2: smooth position. Garry's Mod moves the player 66 times a second and Skyrim
// draws 60 times a second, not in step, so the newest tick each frame gives uneven steps. As
// SkyCraft (MIT) does with Minecraft's ticks: keep the last few ticks with their times locked to
// the exact tick rhythm, and draw the player slightly in the past, blending between the two ticks
// around that moment. The delay follows how late ticks really arrive, so the next tick has
// normally arrived when it's needed. Only the position: the view has no added delay.
// Positions are kept relative to each tick's own anchor: when Garry's Mod re-centres (moves the
// player and the anchor together, DESIGN.md re-plan step C) the relative position doesn't jump.
#include "Smooth.h"

namespace gmsky::Smooth
{
	namespace
	{
		struct Tick
		{
			std::array<double, 3> pos;  // relative to the tick's anchor
			double eye;                 // eye height (step 4: the camera), Garry's Mod units
			uint64_t tick;  // Garry's Mod tick number
			int64_t at;     // when it happened (QPC), on the locked rhythm
		};

		const double kQpcPerMs = [] {
			LARGE_INTEGER f;
			QueryPerformanceFrequency(&f);
			return double(f.QuadPart) / 1000.0;
		}();

		std::deque<Tick> g_ticks;
		int g_stampOutliers = 0;
		int64_t g_lastFrameQpc = 0;
		double g_delayMs = 20.0;
		std::array<double, 132> g_tickDue{};  // last 2 s of ticks (66/s): how long each was already due when we first saw it
		std::size_t g_tickDueNext = 0;
		bool g_tickDueInit = false;
		std::array<double, 3> g_lastDrawn{};
		double g_drawnEye = 64.0;  // the eye height that goes with the last drawn position
		bool g_haveLastDrawn = false;

		// Per second.
		uint32_t g_frames = 0, g_newTicks = 0, g_lateFrames = 0, g_stampSamples = 0, g_offFrames = 0;
		double g_stampErrMs = 0.0;
		// Per-frame steps while moving: how even they are.
		uint32_t g_steps = 0, g_stalls = 0;
		double g_stepSum = 0.0, g_stepSqSum = 0.0, g_stepMax = 0.0;

		double Dist(const std::array<double, 3>& a_a, const std::array<double, 3>& a_b)
		{
			const double dx = a_a[0] - a_b[0], dy = a_a[1] - a_b[1], dz = a_a[2] - a_b[2];
			return std::sqrt(dx * dx + dy * dy + dz * dz);
		}

		std::array<double, 3> Relative(const GmskyPlayerState& a_g)
		{
			return { a_g.pos[0] - a_g.anchor[0], a_g.pos[1] - a_g.anchor[1], a_g.pos[2] - a_g.anchor[2] };
		}

		// Locks a new tick's time to the rhythm: Garry's Mod ticks exactly every tick_ms, but stamps
		// a tick when its frame gets to it (a few ms of noise at 300 fps).
		void AddTick(const GmskyPlayerState& a_g, int64_t a_period, double a_tickMs)
		{
			if (!g_ticks.empty() && a_g.tick < g_ticks.back().tick) {
				g_ticks.clear();  // Garry's Mod restarted or changed map
			}
			Tick t{ Relative(a_g), a_g.eye_height, a_g.tick, a_g.tick_qpc };
			if (!g_ticks.empty()) {
				const auto& last = g_ticks.back();
				const int64_t n = int64_t(a_g.tick - last.tick);
				const int64_t err = a_g.tick_qpc - (last.at + n * a_period);
				if (n >= 1 && n <= 10 && std::abs(err) < a_period * 3 / 10) {
					t.at = last.at + n * a_period + err / 16;  // the rhythm is exact; the stamps are noisy
					g_stampOutliers = 0;
					g_stampErrMs += std::abs(double(err)) / kQpcPerMs;
					++g_stampSamples;
				} else if (n >= 1 && n <= 10 && ++g_stampOutliers < 3) {
					t.at = last.at + n * a_period;  // one odd stamp (a hitch): keep the rhythm
				} else {
					g_stampOutliers = 0;  // lost the rhythm (a pause, a new tick rate): start from this stamp
				}
			}
			// Was it already needed on our last frame? Then drawing had to wait for it. Each tick
			// carries one position (where the player is at its time), so blending toward it starts
			// one tick *before* its time: it is needed from t.at - period. (SkyCraft's ticks carry
			// a start and an end position, so there it is needed from t.at. Test 8: without the
			// extra tick the delay shrank to 4 ms and 3 of 4 frames waited for a tick.)
			if (g_lastFrameQpc != 0) {
				if (!g_tickDueInit) {
					g_tickDue.fill(g_delayMs - 1.0);
					g_tickDueInit = true;
				}
				const double dueMs = double(g_lastFrameQpc - (t.at - a_period)) / kQpcPerMs;
				if (dueMs < 30.0 + a_tickMs) {  // later than that is a hitch, not a pattern to wait for
					g_tickDue[g_tickDueNext++ % g_tickDue.size()] = dueMs;
				}
			}
			g_ticks.push_back(t);
			if (g_ticks.size() > 8) {
				g_ticks.pop_front();
			}
			++g_newTicks;
		}

		void CountStep(const std::array<double, 3>& a_drawn, const GmskyPlayerState& a_g)
		{
			++g_frames;
			if (g_haveLastDrawn && std::hypot(a_g.vel[0], a_g.vel[1]) > 30.0) {
				const double step = Dist(a_drawn, g_lastDrawn);
				++g_steps;
				g_stepSum += step;
				g_stepSqSum += step * step;
				g_stepMax = std::max(g_stepMax, step);
				if (step < 0.01) {
					++g_stalls;
				}
			}
			g_lastDrawn = a_drawn;
			g_haveLastDrawn = true;
		}
	}

	std::array<double, 3> Position(const GmskyPlayerState& a_g)
	{
		const std::array<double, 3> newest = Relative(a_g);
		if (!(a_g.flags & GMSKY_PF_SMOOTH) || a_g.tick_qpc == 0 || a_g.tick_ms <= 0.0f) {
			++g_offFrames;
			g_lastFrameQpc = 0;
			g_ticks.clear();
			g_drawnEye = a_g.eye_height;
			CountStep(newest, a_g);
			return newest;
		}
		const int64_t period = std::max<int64_t>(1, std::llround(double(a_g.tick_ms) * kQpcPerMs));
		LARGE_INTEGER now;
		QueryPerformanceCounter(&now);
		if (g_ticks.empty() || a_g.tick != g_ticks.back().tick) {
			AddTick(a_g, period, a_g.tick_ms);
		}

		// The delay follows how late ticks have been lately: it grows 2% slower than real time and
		// shrinks 0.2% faster, too little to see either way.
		const double frameMs = g_lastFrameQpc != 0 ? double(now.QuadPart - g_lastFrameQpc) / kQpcPerMs : 0.0;
		g_lastFrameQpc = now.QuadPart;
		if (g_tickDueInit) {
			const double target = std::clamp(*std::ranges::max_element(g_tickDue) + 1.0, 4.0, 30.0 + a_g.tick_ms);
			const double dt = std::min(frameMs, 100.0) / 1000.0;
			g_delayMs = target > g_delayMs ? std::min(target, g_delayMs + 20.0 * dt) : std::max(target, g_delayMs - 2.0 * dt);
		}
		const int64_t drawAt = now.QuadPart - std::llround(g_delayMs * kQpcPerMs);

		// The newest tick at or before drawAt, and the one after it.
		std::size_t i = 0;
		for (std::size_t k = g_ticks.size(); k-- > 0;) {
			if (g_ticks[k].at <= drawAt) {
				i = k;
				break;
			}
		}
		const Tick& a = g_ticks[i];
		std::array<double, 3> drawn = a.pos;
		g_drawnEye = a.eye;
		if (i + 1 < g_ticks.size()) {
			const Tick& b = g_ticks[i + 1];
			const double span = double(b.at - a.at);
			const double u = span > 0.0 ? std::clamp(double(drawAt - a.at) / span, 0.0, 1.0) : 1.0;
			for (int c = 0; c < 3; ++c) {
				drawn[c] = a.pos[c] + (b.pos[c] - a.pos[c]) * u;
			}
			g_drawnEye = a.eye + (b.eye - a.eye) * u;
		} else if (drawAt > a.at) {
			++g_lateFrames;  // the next tick hasn't arrived: the player holds still this frame
		}
		CountStep(drawn, a_g);
		return drawn;
	}

	double EyeHeight()
	{
		return g_drawnEye;
	}

	void Reset()
	{
		g_ticks.clear();
		g_lastFrameQpc = 0;
		g_haveLastDrawn = false;
	}

	void LogSecond()
	{
		if (g_frames == 0) {
			return;
		}
		const double avg = g_steps ? g_stepSum / g_steps : 0.0;
		const double sd = g_steps ? std::sqrt(std::max(0.0, g_stepSqSum / g_steps - avg * avg)) : 0.0;
		const std::string mode = g_offFrames ? fmt::format("OFF (gmsky_smooth 0) {} frames", g_offFrames)
		                                     : fmt::format("on: delay={:.1f} ms late frames={} new ticks={} stamp noise={:.2f} ms",
		                                           g_delayMs, g_lateFrames, g_newTicks, g_stampSamples ? g_stampErrMs / g_stampSamples : 0.0);
		logger::info("SMOOTH {} | steps while moving: {} frames avg={:.2f} spread={:.0f}% max={:.2f} stalls={} (Garry's Mod units)",
			mode, g_steps, avg, avg > 0.0 ? 100.0 * sd / avg : 0.0, g_stepMax, g_stalls);
		g_frames = g_newTicks = g_lateFrames = g_stampSamples = g_offFrames = g_steps = g_stalls = 0;
		g_stampErrMs = g_stepSum = g_stepSqSum = g_stepMax = 0.0;
	}
}
