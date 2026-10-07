// Phase 1 step 1c: turn Skyrim's first-person camera straight to the view.
//
// Two things delay what Skyrim shows behind Garry's Mod: Skyrim's camera update runs before the
// player update in a frame (an angle set in the player update is shown a frame later), and its
// first-person camera eases its rotation (~50 ms). So, as SkyCraft (MIT) does: after Skyrim's
// camera update, and again after the player update, we set the camera root's rotation ourselves.
// Skyrim's axis order for the camera root is learned at run time first; nothing is overridden
// until it is certain.
//
// Phase 1 step 4 (adapted from SkyCraft's PinCameraAndSky / ApplyMcFov, MIT): in the same two
// spots the camera (and the sky) are also put at Garry's Mod's eye (feet + eye height: crouching
// lowers it), Garry's Mod's view punch is added (pitch, yaw, roll), and Skyrim's FOV is set to
// Garry's Mod's; Skyrim's own FOV comes back when we stop driving the camera.
//
// After test 97 (the physgun halo smaller than the prop): the two FOVs mean different things.
// Garry's Mod's (Source's) is the width of a 4:3 view, widened for a wider screen: fov_desired 100
// on 16:9 has a half-view tangent of 1.589 (about 115 degrees across, logged in tests 71-74).
// Skyrim's worldFOV is the width of its actual screen: at 100 its frustum's right was 1.1918 =
// tan(50 degrees) on 16:9. So the FOV is converted with Skyrim's screen shape. (SkyCraft's comment
// says Skyrim's is a 4:3 width too; the frustum we logged says otherwise.)
#include "Camera.h"

#include "Input.h"
#include "Probe.h"
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
		// Step 4, per second: how far we moved Skyrim's own camera, our camera's height above the
		// feet, the eye height, the biggest view punch.
		uint32_t g_placed = 0;
		float g_moveSum = 0.0f, g_moveMax = 0.0f, g_heightMin = 1e9f, g_heightMax = -1e9f;
		float g_eyeMin = 1e9f, g_eyeMax = -1e9f, g_punchMax = 0.0f;
		// Skyrim's FOV, saved while Garry's Mod's is in use.
		bool g_fovSaved = false;
		float g_savedWorldFov = 0.0f, g_fovInUse = 0.0f, g_fovGmod = 0.0f, g_aspect = 0.0f;
		// S2j after test 138: the camera's near distance we set (0 = not set), what we found there
		// before setting it each frame (ours again = it stays; Skyrim's = it is reset every frame),
		// and the reach of the near rectangle's corners.
		bool g_nearCustomSaved = false, g_savedUseCustomNear = false;
		float g_savedCustomNear = 0.0f;
		float g_nearSet = 0.0f, g_nearFoundMin = 1e9f, g_nearFoundMax = 0.0f, g_nearReach = 0.0f, g_nearOwn = 0.0f;
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

		// Skyrim's own camera near distance (fNearDistance:Display), 15 by default.
		float OwnNear()
		{
			auto* ini = RE::INISettingCollection::GetSingleton();
			auto* s = ini ? ini->GetSetting("fNearDistance:Display") : nullptr;
			return s && s->GetFloat() > 0.0f ? s->GetFloat() : 15.0f;
		}

		// S2j after test 138 ("the camera still goes through walls"): the camera can't draw closer
		// than a flat rectangle at its near distance, and with a wide view (115.6 degrees) that
		// rectangle's corners reach about twice the near distance from the eye: 30.5 Skyrim units,
		// past the player's 20-unit half width, so a wall the box keeps 20 away was cut. The near
		// distance now shrinks so the corners stay within 90% of the half width (Skyrim's own when
		// that is smaller). Set on the world camera's frustum while we drive the camera.
		void ApplyNear()
		{
			auto* cam = RE::Main::WorldRootCamera();
			const float half = Probe::PlayerHalfWidth();
			if (!cam || half <= 0.0f) {
				return;
			}
			auto& f = cam->GetRuntimeData2().viewFrustum;
			if (f.bOrtho) {
				return;
			}
			g_nearFoundMin = std::min(g_nearFoundMin, f.fNear);
			g_nearFoundMax = std::max(g_nearFoundMax, f.fNear);
			const float r = std::max(std::fabs(f.fLeft), std::fabs(f.fRight));
			const float t = std::max(std::fabs(f.fTop), std::fabs(f.fBottom));
			const float k = std::sqrt(1.0f + r * r + t * t);  // corner distance per unit of near distance
			g_nearOwn = OwnNear();
			const float nearDist = std::clamp(0.9f * half / k, 1.0f, g_nearOwn);
			// Test 139: written only here, Skyrim put its own 15 back now and then (found
			// 8.92..15.00), so the near distance jumped between frames and far shadows flickered.
			// The world scene graph's own custom near distance is what Skyrim rebuilds it from.
			if (auto* sg = RE::Main::WorldRootNode()) {
				auto& d = sg->GetRuntimeData();
				if (!g_nearCustomSaved) {
					g_nearCustomSaved = true;
					g_savedUseCustomNear = d.useCustomNear;
					g_savedCustomNear = d.customNearDistance;
				}
				d.useCustomNear = true;
				d.customNearDistance = nearDist;
			}
			f.fNear = nearDist;
			g_nearSet = nearDist;
			g_nearReach = nearDist * k;
		}

		void RestoreNear()
		{
			if (g_nearCustomSaved) {
				if (auto* sg = RE::Main::WorldRootNode()) {
					auto& d = sg->GetRuntimeData();
					d.useCustomNear = g_savedUseCustomNear;
					d.customNearDistance = g_savedCustomNear;
				}
				g_nearCustomSaved = false;
			}
			if (g_nearSet > 0.0f) {
				if (auto* cam = RE::Main::WorldRootCamera()) {
					cam->GetRuntimeData2().viewFrustum.fNear = OwnNear();
				}
				logger::info("CAMERA near distance back to Skyrim's own {:.1f}", OwnNear());
				g_nearSet = 0.0f;
			}
		}

		void RestoreFov(RE::PlayerCamera* a_camera)
		{
			RestoreNear();
			if (g_fovSaved && a_camera) {
				a_camera->GetRuntimeData2().worldFOV = g_savedWorldFov;
				logger::info("CAMERA FOV back to Skyrim's own {:.1f}", g_savedWorldFov);
			}
			g_fovSaved = false;
			g_fovInUse = g_fovGmod = 0.0f;
		}

		// Skyrim's screen shape (width / height) from its camera's frustum (right / top: both are
		// tangents, so their ratio doesn't depend on the FOV); 0 if not known.
		float ScreenAspect()
		{
			auto* cam = RE::Main::WorldRootCamera();
			if (!cam) {
				return 0.0f;
			}
			const auto& f = cam->GetRuntimeData2().viewFrustum;
			return f.fTop > 1e-4f && f.fRight > 1e-4f ? f.fRight / f.fTop : 0.0f;
		}

		float FrustumRight()
		{
			auto* cam = RE::Main::WorldRootCamera();
			return cam ? cam->GetRuntimeData2().viewFrustum.fRight : 0.0f;
		}

		// Garry's Mod's FOV (the width of a 4:3 view) -> Skyrim's (the width of its screen).
		void ApplyFov(RE::PlayerCamera* a_camera, float a_fov)
		{
			if (a_fov < 10.0f || a_fov > 170.0f) {
				return;
			}
			const float aspect = ScreenAspect();
			if (aspect <= 0.0f) {
				return;  // (the first frames, before Skyrim's camera exists: next frame)
			}
			const double half = std::tan(double(a_fov) * 0.5 * kPi / 180.0) * double(aspect) / (4.0 / 3.0);
			const float skyrimFov = float(2.0 * std::atan(half) * 180.0 / kPi);
			auto& data = a_camera->GetRuntimeData2();
			if (!g_fovSaved) {
				g_savedWorldFov = data.worldFOV;
				g_fovSaved = true;
			}
			if (a_fov != g_fovGmod || std::fabs(aspect - g_aspect) > 1e-3f) {
				logger::info("CAMERA FOV: Garry's Mod's {:.1f} (4:3 width) on a {:.4f} screen -> Skyrim's {:.2f} (screen width; half-view tangent {:.4f}); Skyrim's own was {:.1f}",
					a_fov, aspect, skyrimFov, half, g_savedWorldFov);
				g_fovGmod = a_fov;
				g_aspect = aspect;
			}
			data.worldFOV = skyrimFov;
			g_fovInUse = skyrimFov;
		}

		// Whether we drive the camera now, and how. Sets g_why when not (and gives Skyrim its FOV
		// back when we stop driving for good: puppet off, gmsky_camera 0).
		bool Wanted(RE::PlayerCamera* a_camera, Puppet::CameraView& a_view)
		{
			if (!a_camera || !a_camera->cameraRoot) {
				g_why = "no camera";
			} else if (!Puppet::CameraNow(a_view)) {
				g_why = "puppet off";
				RestoreFov(a_camera);
			} else if (!a_view.drive) {
				g_why = "off (gmsky_camera 0)";
				RestoreFov(a_camera);
			} else if (!a_camera->IsInFirstPerson()) {
				g_why = "not in first person";
			} else if (Input::SkyrimMenuOpen()) {
				g_why = "a Skyrim menu is open";
			} else {
				return true;
			}
			return false;
		}

		// Turns the camera root to the view (with the view punch's roll: right and up turned
		// around forward) and puts it, and the sky, at the eye.
		void Drive(RE::PlayerCamera* a_camera, const Puppet::CameraView& a_view)
		{
			auto* root = a_camera->cameraRoot.get();
			auto axes = Axes(a_view.headingRad, a_view.pitchRad);
			if (a_view.rollRad != 0.0) {
				const float c = float(std::cos(a_view.rollRad)), s = float(std::sin(a_view.rollRad));
				const RE::NiPoint3 r = axes[4], u = axes[2];
				axes[4] = r * c + u * s;
				axes[5] = axes[4] * -1.0f;
				axes[2] = u * c - r * s;
				axes[3] = axes[2] * -1.0f;
			}
			RE::NiMatrix3 m;
			for (int c = 0; c < 3; ++c) {
				const auto& v = axes[g_axisMap[c]];
				m.entry[0][c] = v.x;
				m.entry[1][c] = v.y;
				m.entry[2][c] = v.z;
			}
			root->local.rotate = m;
			root->world.rotate = m;
			// Position (as SkyCraft's PinCameraAndSky): the root's own translate only when its
			// parent adds nothing, the camera's position, and the sky (it is drawn around the camera).
			const bool identityParent = !root->parent || root->parent->world.translate.Length() < 0.001f;
			if (identityParent) {
				root->local.translate = a_view.eye;
			}
			root->world.translate = a_view.eye;
			a_camera->GetRuntimeData2().pos = a_view.eye;
			if (auto* sky = RE::Sky::GetSingleton(); sky && sky->root) {
				sky->root->local.translate = a_view.eye;
				sky->root->world.translate = a_view.eye;
			}
			ApplyFov(a_camera, a_view.fovDeg);
			ApplyNear();
			// Pass the new rotation and position down to the render camera under the root.
			RE::NiUpdateData update{};
			root->UpdateDownwardPass(update, 0);
		}

		void CountPlace(const RE::NiPoint3& a_skyrimCamera, const Puppet::CameraView& a_view)
		{
			const float move = a_skyrimCamera.GetDistance(a_view.eye);
			const float height = float(a_view.eyeGmod / kGmodPerSkyrim);
			++g_placed;
			g_moveSum += move;
			g_moveMax = std::max(g_moveMax, move);
			g_heightMin = std::min(g_heightMin, height);
			g_heightMax = std::max(g_heightMax, height);
			g_eyeMin = std::min(g_eyeMin, a_view.eyeGmod);
			g_eyeMax = std::max(g_eyeMax, a_view.eyeGmod);
			for (float p : a_view.punchDeg) {
				g_punchMax = std::max(g_punchMax, std::fabs(p));
			}
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
			Puppet::CameraView view;
			auto* player = RE::PlayerCharacter::GetSingleton();
			if (!player || !Wanted(a_camera, view) || g_rejected) {
				return;
			}
			const double heading = view.headingRad, pitch = view.pitchRad;
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
			const auto skyrimCamera = root->world.translate;  // where Skyrim's own update put it
			Drive(a_camera, view);
			CountPlace(skyrimCamera, view);
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
		Puppet::CameraView view;
		if (!g_learned || !Wanted(camera, view)) {
			return;
		}
		Drive(camera, view);
		++g_turnedAfterPlayer;
	}

	void LogSecond()
	{
		const std::string learning = g_learned ? "" : g_rejected ? " axes INCONSISTENT" : fmt::format(" learning {}/{}", g_samples, kLearnFrames);
		logger::info("CAMERA updates={} turned by us after camera update={} after player update={} | Skyrim's own camera off our view avg={:.2f} max={:.2f} deg{}{}",
			g_cameraCalls, g_turnedAfterCamera, g_turnedAfterPlayer, g_devCount ? g_devSum / g_devCount : 0.0f, g_devMax, learning,
			g_why.empty() ? "" : " | not turning: " + g_why);
		if (g_placed) {
			logger::info("CAMERA eye (step 4): placed {} times; Garry's Mod eye height {:.1f}..{:.1f} -> camera {:.1f}..{:.1f} Skyrim units above the feet; moved Skyrim's own camera by {:.1f} avg {:.1f} max; view punch max {:.2f} deg; FOV {:.1f} (Skyrim's own {:.1f}); "
				"Skyrim's frustum right {:.4f}, Garry's Mod's half-view tangent {:.4f} (equal = same view); near distance set {:.2f} (Skyrim's own {:.1f}; its corners reach {:.1f}, the player's half width {:.1f}), found before setting {:.2f}..{:.2f} (equal to the set value = it stays)",
				g_placed, g_eyeMin, g_eyeMax, g_heightMin, g_heightMax, g_moveSum / g_placed, g_moveMax, g_punchMax, g_fovInUse,
				g_fovSaved ? g_savedWorldFov : 0.0f, FrustumRight(),
				g_fovGmod > 0.0f && g_aspect > 0.0f ? std::tan(double(g_fovGmod) * 0.5 * kPi / 180.0) * g_aspect / (4.0 / 3.0) : 0.0,
				g_nearSet, g_nearOwn, g_nearReach, Probe::PlayerHalfWidth(), g_nearFoundMin < 1e8f ? g_nearFoundMin : 0.0f, g_nearFoundMax);
			g_nearFoundMin = 1e9f;
			g_nearFoundMax = 0.0f;
		}
		g_cameraCalls = g_turnedAfterCamera = g_turnedAfterPlayer = g_devCount = g_placed = 0;
		g_devSum = g_devMax = g_moveSum = g_moveMax = g_punchMax = 0.0f;
		g_heightMin = g_eyeMin = 1e9f;
		g_heightMax = g_eyeMax = -1e9f;
	}
}
