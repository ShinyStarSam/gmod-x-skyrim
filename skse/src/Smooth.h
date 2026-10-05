#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Smooth
{
	// Phase 1 step 2. Called every frame with Garry's Mod's newest player state; returns the
	// position to draw this frame (Garry's Mod units, relative to the anchor): blended between the ticks around "now minus
	// a small delay", or the newest tick as is when Garry's Mod turned smoothing off.
	std::array<double, 3> Position(const GmskyPlayerState& a_g);

	// The eye height (Garry's Mod units) blended the same way, for the frame Position last drew
	// (step 4: the camera follows crouching smoothly).
	double EyeHeight();

	// Forget the tick history (game load, new origin, Garry's Mod restarted).
	void Reset();

	// Once a second: delay, late frames and how even the per-frame steps were.
	void LogSecond();
}
