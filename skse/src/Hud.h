#pragma once

namespace gmsky::Hud
{
	// Step 2d-1 (DESIGN.md "Step 2d in detail"): parts of Skyrim's HUD hidden while Garry's Mod's
	// overlay is live (crosshair, health / magicka / stamina, compass, charge meters, arrows).
	// Game thread only.

	// Every frame from the per-frame driver; `a_live`: the overlay is live (Arms::Live()).
	void PerFrame(RE::PlayerCharacter* a_player, bool a_live);
	// Once a second: hidden or shown, changes.
	void LogSecond();
}
