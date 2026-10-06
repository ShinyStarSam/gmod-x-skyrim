#pragma once

namespace gmsky::Draw
{
	// Step 2c-1 (DESIGN.md "Step 2c in detail"): Garry's Mod's picture drawn over Skyrim's frame,
	// in a hook on Skyrim's swap chain Present (as SkyCraft's Overlay.cpp, MIT).

	// At kDataLoaded: hooks Present (the swap chain exists by then).
	void Install();

	// Once a second (game thread): presents, drawn / not drawn and why, upload ms, picture age.
	void LogSecond();
}
