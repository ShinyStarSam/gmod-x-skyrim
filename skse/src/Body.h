#pragma once

namespace gmsky::Body
{
	// Phase 1 step 3f: Skyrim's player body while Garry's Mod drives it. Hooks Skyrim's character
	// contact collector and the "land rescue" in the actor movement update. Call once at data load.
	void Install();

	// Every frame the puppet moved the player: its body (the character proxy's collidable, null if
	// none), feet position (Skyrim units) and whether Garry's Mod is in noclip.
	void Set(const RE::hkpCollidable* a_body, const RE::NiPoint3& a_feet, bool a_noclip);

	// The puppet is off: Skyrim's body behaves normally again.
	void Release();

	// Once a second: contacts dropped and land rescues skipped, to logs/skyrim.log.
	void LogSecond();
}
