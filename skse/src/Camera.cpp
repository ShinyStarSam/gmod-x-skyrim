// Phase 1 step 1c: turn Skyrim's first-person camera straight to the view.
//
// Two things delay what Skyrim shows behind Garry's Mod: Skyrim's camera update runs before the
// player update in a frame (an angle set in the player update is shown a frame later), and its
// first-person camera eases its rotation (~50 ms). So, as SkyCraft (MIT) does: after Skyrim's
// camera update, and again after the player update, we set the camera root's rotation ourselves.
// Skyrim's axis order for the camera root is learned at run time first; nothing is overridden
// until it is certain.
#include "Camera.h"

#include "Input.h"
#include "Puppet.h"

namespace gmsky::Camera
{
	namespace
	{
		constexpr float kRadToDegF = float(180.0 / kPi);
		constexpr int kLearnFrames = 240;
		constexpr float kVoteToleranceDeg = 6.0f;
		constexpr const char* kAxisNames[6] = { "+forward", "-forward", "+up", "-up", "+right", "-right" };

		// Axis learning: for each column of Skyrim's camera-root matrix, votes for which of
		// +/- forward/up/right it is.
		std::array<std::array<int, 6>, 3> g_votes{};
		int g_samples = 0;
		std::array<int, 3> g_axisMap{ 0, 1, 2 };
		bool g_learned = false, g_rejected = false;

		// Per second.
		uint32_t g_cameraCalls = 0, g_turnedAfterCamera = 0, g_turnedAfterPlayer = 0, g_devCount = 0;
		float g_devSum = 0.0f, g_devMax = 0.0f;
		std::string g_why = "not started";

		RE::NiPoint3 Col(const RE::NiMatrix3& a_m, int a_c)
		{
			return { a_m.entry[0][a_c], a_m.entry[1][a_c], a_m.entry[2][a_c] };
		}

		float AngleDeg(const RE::NiPoint3& a_a, const RE::NiPoint3& a_b)
		{
			const float la = a_a.Length(), lb = a_b.Length();
			if (la < 1e-6f || lb < 1e-6f) {
				return 180.0f;
			}
			return std::acos(std::clamp(a_a.Dot(a_b) / (la * lb), -1.0f, 1.0f)) * kRadToDegF;
		}

		// +forward, -forward, +up, -up, +right, -right of a look direction (no roll).
		// Skyrim: X east, Y north, Z up; heading clockwise from north; positive pitch looks down.
		std::array<RE::NiPoint3, 6> Axes(double a_heading, double a_pitch)
		{
			const float sh = float(std::sin(a_heading)), ch = float(std::cos(a_heading));
			const float sp = float(std::sin(a_pitch)), cp = float(std::cos(a_pitch));
			const RE::NiPoint3 f{ sh * cp, ch * cp, -sp };
			const RE::NiPoint3 r{ ch, -sh, 0.0f };
			const RE::NiPoint3 u = r.Cross(f);
			return { f, f * -1.0f, u, u * -1.0f, r, r * -1.0f };
		}

		// Whether we turn the camera now, and to where. Sets g_why when not.
		bool Wanted(RE::PlayerCamera* a_camera, double& a_heading, double& a_pitch)
		{
			bool drive = false;
			if (!a_camera || !a_camera->cameraRoot) {
				g_why = "no camera";
			} else if (!Puppet::ViewNow(a_heading, a_pitch, drive)) {
				g_why = "puppet off";
			} else if (!drive) {
				g_why = "off (gmsky_camera 0)";
			} else if (!a_camera->IsInFirstPerson()) {
				g_why = "not in first person";
			} else if (Input::SkyrimMenuOpen()) {
				g_why = "a Skyrim menu is open";
			} else {
				return true;
			}
			return false;
		}

		void Turn(RE::NiNode* a_root, double a_heading, double a_pitch)
		{
			const auto axes = Axes(a_heading, a_pitch);
			RE::NiMatrix3 m;
			for (int c = 0; c < 3; ++c) {
				const auto& v = axes[g_axisMap[c]];
				m.entry[0][c] = v.x;
				m.entry[1][c] = v.y;
				m.entry[2][c] = v.z;
			}
			a_root->local.rotate = m;
			a_root->world.rotate = m;
			// Pass the new rotation down to the render camera under the root.
			RE::NiUpdateData update{};
			a_root->UpdateDownwardPass(update, 0);
		}

		// Learns the axis order from Skyrim's own camera while it agrees with the player's angles.
		void Learn(const RE::NiMatrix3& a_skyrim, RE::PlayerCharacter* a_player)
		{
			const auto axes = Axes(a_player->data.angle.z, a_player->data.angle.x);
			for (int c = 0; c < 3; ++c) {
				int best = -1;
				float bestAngle = 1e9f;
				for (int k = 0; k < 6; ++k) {
					const float a = AngleDeg(Col(a_skyrim, c), axes[k]);
					if (a < bestAngle) {
						bestAngle = a;
						best = k;
					}
				}
				if (bestAngle < kVoteToleranceDeg) {
					++g_votes[c][best];
				}
			}
			if (++g_samples < kLearnFrames) {
				return;
			}
			bool ok = true;
			std::array<bool, 3> used{};
			for (int c = 0; c < 3; ++c) {
				const auto it = std::ranges::max_element(g_votes[c]);
				const int k = int(it - g_votes[c].begin());
				ok &= *it > g_samples * 6 / 10 && !used[k / 2];
				used[k / 2] = true;
				g_axisMap[c] = k;
			}
			g_learned = ok;
			g_rejected = !ok;
			logger::info("CAMERA axes learned from {} frames: col0={} ({}) col1={} ({}) col2={} ({}) -> {}", g_samples,
				kAxisNames[g_axisMap[0]], *std::ranges::max_element(g_votes[0]), kAxisNames[g_axisMap[1]],
				*std::ranges::max_element(g_votes[1]), kAxisNames[g_axisMap[2]], *std::ranges::max_element(g_votes[2]),
				ok ? "we turn the camera from now on" : "INCONSISTENT, leaving the camera to Skyrim");
		}

		void AfterCameraUpdate(RE::PlayerCamera* a_camera)
		{
			++g_cameraCalls;
			double heading = 0.0, pitch = 0.0;
			auto* player = RE::PlayerCharacter::GetSingleton();
			if (!player || !Wanted(a_camera, heading, pitch) || g_rejected) {
				return;
			}
			auto* root = a_camera->cameraRoot.get();
			const auto skyrim = root->world.rotate;
			if (!g_learned) {
				g_why = "learning Skyrim's camera axes";
				Learn(skyrim, player);
				return;
			}
			// How far Skyrim's own camera is from the view we show: its easing and frame delay.
			const auto axes = Axes(heading, pitch);
			float dev = 0.0f;
			for (int c = 0; c < 3; ++c) {
				dev = std::max(dev, AngleDeg(Col(skyrim, c), axes[g_axisMap[c]]));
			}
			g_devSum += dev;
			g_devMax = std::max(g_devMax, dev);
			++g_devCount;
			Turn(root, heading, pitch);
			++g_turnedAfterCamera;
			g_why.clear();
		}

		struct CameraUpdateHook
		{
			static void thunk(RE::PlayerCamera* a_this)
			{
				func(a_this);
				try {
					AfterCameraUpdate(a_this);
				} catch (const std::exception& e) {
					logger::error("camera update: {}", e.what());
				}
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};
	}

	void Install()
	{
		// PlayerCamera::Update is called directly, not through the function table, so (as
		// SkyCraft) scan the game's code for `call PlayerCamera::Update` and redirect each one.
		const auto target = REL::Relocation<std::uintptr_t>{ RELOCATION_ID(49852, 50784) }.address();
		const auto text = REL::Module::get().segment(REL::Segment::textx);
		const auto base = text.address();
		const auto* code = reinterpret_cast<const std::uint8_t*>(base);
		std::vector<std::uintptr_t> sites;
		for (std::size_t i = 0; i + 5 <= text.size(); ++i) {
			if (code[i] != 0xE8) {
				continue;
			}
			std::int32_t rel;
			std::memcpy(&rel, code + i + 1, 4);
			if (base + i + 5 + static_cast<std::intptr_t>(rel) == target) {
				sites.push_back(base + i);
			}
		}
		auto& trampoline = SKSE::GetTrampoline();
		for (const auto site : sites) {
			CameraUpdateHook::func = trampoline.write_call<5>(site, CameraUpdateHook::thunk);
		}
		logger::info("PlayerCamera::Update: hooked {} call site(s)", sites.size());
	}

	void AfterPlayerUpdate()
	{
		auto* camera = RE::PlayerCamera::GetSingleton();
		double heading = 0.0, pitch = 0.0;
		if (!g_learned || !Wanted(camera, heading, pitch)) {
			return;
		}
		Turn(camera->cameraRoot.get(), heading, pitch);
		++g_turnedAfterPlayer;
	}

	void LogSecond()
	{
		const std::string learning = g_learned ? "" : g_rejected ? " axes INCONSISTENT" : fmt::format(" learning {}/{}", g_samples, kLearnFrames);
		logger::info("CAMERA updates={} turned by us after camera update={} after player update={} | Skyrim's own camera off our view avg={:.2f} max={:.2f} deg{}{}",
			g_cameraCalls, g_turnedAfterCamera, g_turnedAfterPlayer, g_devCount ? g_devSum / g_devCount : 0.0f, g_devMax, learning,
			g_why.empty() ? "" : " | not turning: " + g_why);
		g_cameraCalls = g_turnedAfterCamera = g_turnedAfterPlayer = g_devCount = 0;
		g_devSum = g_devMax = 0.0f;
	}
}
