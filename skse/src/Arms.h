#pragma once

namespace gmsky::Arms
{
	// Step 2c-2 (DESIGN.md "Step 2c in detail"): Skyrim's own first-person arms and weapon are
	// hidden while Garry's Mod's overlay is live (Garry's Mod draws the hands). Game thread only.

	// Every frame from the per-frame driver.
	void PerFrame(RE::PlayerCharacter* a_player);
	// Shows everything hidden again and forgets it (game load).
	void Reset(const char* a_reason);
	// Once a second: how many meshes are hidden, and changes.
	void LogSecond();
	// The overlay counts as live (the same decision that hides the arms; step 2d-1 uses it too).
	bool Live();
}
