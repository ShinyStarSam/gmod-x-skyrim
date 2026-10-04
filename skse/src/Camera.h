#pragma once

namespace gmsky::Camera
{
	// Phase 1 step 1c: hooks Skyrim's camera update (its call sites, it isn't called through the
	// function table).
	void Install();

	// Every frame, after the puppet: turn the camera again with the newest view.
	void AfterPlayerUpdate();

	// Once a second: camera statistics to logs/skyrim.log.
	void LogSecond();
}
