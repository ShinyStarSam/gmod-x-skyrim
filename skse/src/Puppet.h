#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Puppet
{
	// Phase 0 step 6: Skyrim's player follows Garry's Mod. Called every frame after Skyrim's own
	// player update. Moves the player only when it's safe (Garry's Mod alive and anchored to
	// `a_origin`, player loaded); otherwise Skyrim keeps normal control. Returns true if it moved
	// the player this frame.
	bool Update(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin);

	// True if Skyrim itself moved the player far from where the puppet last put it (load door,
	// fast travel, console). The caller then publishes a new origin.
	bool SkyrimMovedPlayer(const RE::NiPoint3& a_pos);

	// Stop puppeting until conditions are met again (game load, new origin).
	void Reset(const char* a_reason);

	// Once a second: what the puppet did, to logs/skyrim.log.
	void LogSecond();
}
