#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Actors
{
	// Phase 3 step 3b: Skyrim's actors near the player, nearest first, into the GmskyActors slot
	// every frame (adapted from SkyCraft's WriteActorTable, MIT). Game thread only.
	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame);
	// Once a second: how many, the nearest few, write time.
	void LogSecond();
}
