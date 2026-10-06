#pragma once

namespace gmsky::World
{
	// Geometry route (DESIGN.md §12, "Garry's Mod's things drawn inside Skyrim"): Garry's Mod's
	// objects drawn by Skyrim itself, inside its own frame, as SkyCraft's WorldRender.cpp (MIT).
	// Step G1: only a fixed test cube, to prove the hook, Skyrim's depth and camera on our setup.

	// At kDataLoaded: hooks the call to Main::RenderWorld (checked before patching).
	void Install();

	// A save was loaded or a new game started: the test cube is placed again.
	void OnGameLoaded();

	// Once a second (game thread): frames drawn / skipped and why, ms, where the cube is on screen.
	void LogSecond();
}
