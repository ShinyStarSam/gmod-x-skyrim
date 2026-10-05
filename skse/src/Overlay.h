#pragma once

namespace gmsky::Overlay
{
	// Step 2b-4 (DESIGN.md "Step 2b in detail"): the Skyrim side of the overlay picture block
	// (gmsky_protocol.h "Overlay picture block"). For now it only publishes Skyrim's back buffer
	// size and looks at what Garry's Mod publishes (logged); it doesn't take pictures yet (2c).

	// Every frame (from the per-frame driver): opens the block if needed (retried every 2 s),
	// writes the back buffer size.
	void PerFrame();
	// Once a second: Skyrim's size, Garry's Mod's newest picture.
	void LogSecond();
}
