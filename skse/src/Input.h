#pragma once

namespace gmsky::Input
{
	// Installs the input listener and the PlayerControls / MenuControls hooks.
	void Install();

	// Every frame: decide whether input goes to Garry's Mod, and publish it (GmskyInput slot).
	void PerFrame(uint64_t a_frame);

	// Once a second: input statistics to logs/skyrim.log.
	void LogSecond();

	// Running mouse totals published so far. Returns false while input goes to Skyrim.
	bool MouseTotals(int64_t& a_x, int64_t& a_y);

	// A Skyrim menu that pauses the game or shows a cursor is open.
	bool SkyrimMenuOpen();
}
