#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Puppet
{
	// Phase 0 step 6: Skyrim's player follows Garry's Mod. Called every frame after Skyrim's own
	// player update. Moves the player only when it's safe (Garry's Mod alive and anchored to
	// `a_origin`, player loaded); otherwise Skyrim keeps normal control. Returns true if it moved
	// the player this frame. `a_frame` is Skyrim's frame counter (for measuring the view's lag).
	bool Update(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame);

	// True if Skyrim itself moved the player far from where the puppet last put it (load door,
	// fast travel, console). The caller then publishes a new origin.
	bool SkyrimMovedPlayer(const RE::NiPoint3& a_pos);

	// The puppet moved the player on the latest frame.
	bool IsActive();

	// The view to show right now: Garry's Mod's newest view plus the mouse movement it hasn't
	// applied yet (Skyrim radians). False while the puppet is off. a_driveCamera: Garry's Mod
	// allows turning Skyrim's camera directly (gmsky_camera).
	bool ViewNow(double& a_headingRad, double& a_pitchRad, bool& a_driveCamera);

	// Step 4, the camera: ViewNow's view plus Garry's Mod's view punch, where its eye is (the
	// puppet's feet on the latest frame + Garry's Mod's eye height, both blended between ticks;
	// Skyrim units) and its FOV. False while the puppet is off.
	struct CameraView
	{
		double headingRad = 0.0, pitchRad = 0.0, rollRad = 0.0;
		RE::NiPoint3 eye{};
		float eyeGmod = 0.0f;   // eye height, Garry's Mod units
		float fovDeg = 0.0f;    // horizontal FOV of a 4:3 view (Source and Skyrim alike); 0 = unknown
		float punchDeg[3]{};    // pitch, yaw, roll
		bool drive = false;     // gmsky_camera 1
	};
	bool CameraNow(CameraView& a_out);

	// Stop puppeting until conditions are met again (game load, new origin).
	void Reset(const char* a_reason);

	// Once a second: what the puppet did, to logs/skyrim.log.
	void LogSecond();
}
