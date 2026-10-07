#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Collision
{
	// Re-plan step D (DESIGN.md). Adapted from SkyCraft (MIT), skse/src/Collision.cpp: streams
	// Skyrim's Havok collision around the player to Garry's Mod as triangles, region by region
	// (cubes of kRegionSize Skyrim units), nearest to where the player is going first, through
	// the collision ring. The main thread copies the shapes out of the Havok world under its read
	// lock; a worker thread turns them into triangles and writes the ring.
	//
	// Every frame while the puppet is on and Garry's Mod asks for shapes (GMSKY_PF_SHAPES).
	// Otherwise it stops; when it starts again it sends "clear" with a new collision epoch.
	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin);

	// Stop streaming (game load, location change). The next PerFrame starts again from "clear".
	void Reset();

	// Once a second: regions, triangles, timings, ring use.
	void LogSecond();

	// S2k-K0: bodies on this layer are sent as part of the static ground.
	bool SentAsGround(RE::COL_LAYER a_layer);

	inline constexpr float kRegionSize = 512.0f;  // Skyrim units per region edge
}
