#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Hitboxes
{
	// Phase 3 step 3c (DESIGN.md "Hitboxes: how bullets hit Skyrim's NPCs"): reads the body shapes
	// of the NPCs near the player (the per-bone bodies that become the ragdoll), sends the nearest
	// ones to Garry's Mod (GmskyHitboxes slot, 3c-2) and logs them (3c-1). Game thread only.
	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame);
	// Once a second: the per-frame "do the parts follow the bones" numbers.
	void LogSecond();
}
