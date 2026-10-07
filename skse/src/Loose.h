#pragma once

namespace gmsky::Loose
{
	// S2k-K0 (DESIGN.md "S2k"): measurement only, changes nothing in the game. Once a second, every
	// Havok body near the player that isn't fixed (loose clutter, dropped weapons, doors, ...),
	// counted by collision layer and motion type; every 5 s the nearest ones listed with their
	// reference, mass, speed and shape. Also counts moving bodies on layers Collision.cpp bakes
	// into the static ground. Game thread only.
	void LogSecond(RE::PlayerCharacter* a_player);
}
