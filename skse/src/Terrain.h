#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Terrain
{
	// Phase 1 step 3b. Every frame while the puppet is on: casts a share of the downward rays for
	// the grid around the player (a time budget per frame) and publishes each finished grid to
	// Garry's Mod (GmskyTerrain slot), in Garry's Mod units for the current origin.
	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame);

	// Drop the grid in progress (game load).
	void Reset();

	// Once a second: grids published, ray timings, and the grid's height under the player
	// compared with where the puppet stands.
	void LogSecond(RE::PlayerCharacter* a_player);
}
