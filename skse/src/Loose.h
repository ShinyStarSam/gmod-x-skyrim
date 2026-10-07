#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Loose
{
	// S2k-K0 (DESIGN.md "S2k"): measurement only, changes nothing in the game. Once a second, every
	// Havok body near the player that isn't fixed (loose clutter, dropped weapons, doors, ...),
	// counted by collision layer and motion type; every 30 s (5 in K0) the nearest ones listed with their
	// reference, mass, speed and shape. Also counts moving bodies on layers Collision.cpp bakes
	// into the static ground. Game thread only.
	void LogSecond(RE::PlayerCharacter* a_player);

	// S2k-K1: ten times a second, the loose bodies (layers Clutter, Weapon, Props; not fixed)
	// within 1,500 Skyrim units, the 2,000 nearest, go to Garry's Mod through the collision ring:
	// new ones with their convex pieces, moved ones again without, gone ones as gone. Only while
	// the collision stream runs; a new stream sends everything again.
	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, float a_delta);
	// Once a second: what K1 sent.
	void LogSending();
}
