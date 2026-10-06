#pragma once

#include "gmsky_protocol.h"

namespace gmsky::Render
{
	// Geometry route, step G2 (DESIGN.md §12): Skyrim's side of the render block
	// (Local\GmodSkyrim_render_v1). Reads Garry's Mod's models, textures and frames from the ring
	// (as SkyCraft's WorldRender.cpp DrainMessages, MIT), checks every message and keeps them on the
	// CPU. Drawing them is G3.

	struct Model
	{
		uint64_t serial = 0;  // new for every MODEL message (the drawing side re-uploads on change)
		GmskyRndModel header{};
		std::vector<GmskyRndPart> parts;
		std::vector<GmskyRndVertex> vertices;
		std::vector<uint32_t> indices;
	};

	struct Texture
	{
		uint64_t serial = 0;
		GmskyRndTexture header{};
		std::vector<uint8_t> pixels;
	};

	struct Frame
	{
		bool valid = false;
		GmskyRndFrame header{};
		std::vector<GmskyRndObject> objects;
		std::vector<GmskyRndMatrix> matrices;
	};

	// Render thread, once per Skyrim frame (from the in-frame hook): opens the block if needed
	// (every 2 s until it works), then reads what Garry's Mod wrote since.
	void Drain();

	// Render thread: what has arrived so far (valid until the next Drain).
	const std::unordered_map<uint32_t, Model>& Models();
	const std::unordered_map<uint32_t, Texture>& Textures();
	const Frame& NewestFrame();

	// Once a second (game thread): messages by type, bytes, resets, bad messages, what is held,
	// the newest frame's objects and age.
	void LogSecond();
}
