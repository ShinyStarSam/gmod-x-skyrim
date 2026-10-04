#pragma once

namespace gmsky::Probe
{
	// Hooks the player's per-frame update.
	void Install();
	// A save was loaded or a new game started: publish a fresh origin next frame.
	void OnGameLoaded();
	void PerFrame(RE::PlayerCharacter* a_player, float a_delta);
}
