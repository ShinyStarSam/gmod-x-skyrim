#pragma once

#include "gmsky_protocol.h"

#include <string>
#include <vector>

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

	// S2k: bodies on this layer with this motion are sent as part of the static ground (moving
	// Props bodies aren't since K1: they are loose objects).
	bool SentAsGround(RE::COL_LAYER a_layer, RE::hkpMotion::MotionType a_motion);

	// S2k-K1: the collision stream's epoch while streaming, else 0.
	uint32_t StreamEpoch();
	// S2k-K1: queue a ready ring message behind the worker's other messages (so it never comes
	// before its stream's "clear"). Dropped if the stream has moved on from a_epoch.
	void Post(uint32_t a_epoch, uint32_t a_type, std::vector<uint8_t> a_payload);
	// S2k-K1: the posted messages since the last call, for the log.
	std::string RawText();

	inline constexpr float kRegionSize = 512.0f;  // Skyrim units per region edge
}
