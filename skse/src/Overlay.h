#pragma once

#include "gmsky_protocol.h"

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

	// Step 2c-1, the reader (any thread, but one at a time: the Present hook).
	bool IsOpen();
	// If Garry's Mod published a picture since the last take, makes it ours: *a_slot, true. The
	// first call attaches Skyrim as the block's reader.
	bool Take(uint32_t* a_slot);
	GmskyOverlaySlotHdr Header(uint32_t a_slot);
	const uint8_t* Pixels(uint32_t a_slot);
	// Garry's Mod's snapshot request counter (gmsky_overlay_shot); 0 if the block isn't open.
	uint32_t ShotRequest();
}
