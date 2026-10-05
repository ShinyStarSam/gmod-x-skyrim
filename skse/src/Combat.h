#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Combat
{
	// Phase 3 step 3d (DESIGN.md "3d in detail"): Garry's Mod's hits on Skyrim's NPCs (the
	// GmskyHitEvents queue) applied through Skyrim's own hit function. Adapted from SkyCraft (MIT),
	// Combat.cpp ApplyHit. Game thread only.

	// At kDataLoaded: finds Skyrim's hit function (checked against the engine's own melee code).
	void Install();
	// Every frame from the per-frame driver.
	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, bool a_puppeting);
	// Once a second: hits applied, lost, by which path.
	void LogSecond();
}
