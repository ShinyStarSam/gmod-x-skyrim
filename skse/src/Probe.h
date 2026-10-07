#pragma once

namespace gmsky::Probe
{
	// Hooks the player's per-frame update (which then also runs the puppet, Puppet.h).
	void Install();
	// A save was loaded or a new game started: publish a fresh origin next frame.
	void OnGameLoaded();
	void PerFrame(RE::PlayerCharacter* a_player, float a_delta);
	// S2j: the player's half width in Skyrim units, as sent to Garry's Mod last frame (Garry's
	// Mod's box is this wide); 0 if not known.
	float PlayerHalfWidth();
}
