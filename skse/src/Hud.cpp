// Step 2d-1: parts of Skyrim's HUD hidden while Garry's Mod's overlay is live (the user's choice,
// DESIGN.md "Step 2d in detail"): Garry's Mod's HUD (HOLOHUD) shows its own crosshair, health and
// compass. Adapted from SkyCraft (MIT), Game.cpp HideHud / HideCrosshair: the HUD movie's elements
// get `_visible` (the crosshair also `_alpha`) set every frame while live, because Skyrim's HUD
// shows the crosshair again whenever it updates it. The sneak eye still shows while sneaking.
// Everything hidden is shown again when the overlay stops. Kept: activation prompt, enemy health,
// notifications, subtitles.
#include "Hud.h"

namespace gmsky::Hud
{
	namespace
	{
		constexpr const char* kBase = "_root.HUDMovieBaseInstance.";
		// Hidden while live. The compass holder also holds the shout meter.
		constexpr const char* kElements[] = {
			"Health",
			"Magica",
			"Stamina",
			"LeftChargeMeter",
			"RightChargeMeter",
			"ArrowInfoInstance",
			"CompassShoutMeterHolder",
		};
		constexpr const char* kCrosshair[] = { "CrosshairInstance", "CrosshairAlert" };
		constexpr const char* kStealth = "StealthMeterInstance";

		bool g_hidden = false;
		bool g_pathsLogged = false;
		uint64_t g_changes = 0;

		std::string Path(const char* a_element, const char* a_field = nullptr)
		{
			std::string p = std::string(kBase) + a_element;
			if (a_field) {
				p += ".";
				p += a_field;
			}
			return p;
		}

		// Once: which of our element paths this Skyrim's HUD movie has (they are names in Skyrim's
		// own HUD; a missing one means that element isn't hidden).
		void LogPaths(RE::GFxMovieView* a_movie)
		{
			std::string found, missing;
			auto check = [&](const char* a_element) {
				RE::GFxValue v;
				const bool ok = a_movie->GetVariable(&v, Path(a_element).c_str()) && !v.IsUndefined();
				std::string& list = ok ? found : missing;
				if (!list.empty()) {
					list += ", ";
				}
				list += a_element;
			};
			for (const auto* e : kElements) check(e);
			for (const auto* e : kCrosshair) check(e);
			check(kStealth);
			logger::info("hud: Skyrim's HUD elements found: {}; NOT found: {}", found, missing.empty() ? "none" : missing);
		}

		void Apply(RE::GFxMovieView* a_movie, bool a_hide, bool a_sneaking)
		{
			const RE::GFxValue visible(!a_hide);
			for (const auto* e : kElements) {
				a_movie->SetVariable(Path(e, "_visible").c_str(), visible);
			}
			const RE::GFxValue alpha(a_hide ? 0.0 : 100.0);
			for (const auto* e : kCrosshair) {
				a_movie->SetVariable(Path(e, "_visible").c_str(), visible);
				a_movie->SetVariable(Path(e, "_alpha").c_str(), alpha);
			}
			a_movie->SetVariable(Path(kStealth, "_visible").c_str(), RE::GFxValue(!a_hide || a_sneaking));
		}
	}

	void PerFrame(RE::PlayerCharacter* a_player, bool a_live)
	{
		if (!a_live && !g_hidden) {
			return;
		}
		auto* ui = RE::UI::GetSingleton();
		auto hud = ui ? ui->GetMenu(RE::HUDMenu::MENU_NAME) : nullptr;
		if (!hud || !hud->uiMovie) {
			return;
		}
		if (!g_pathsLogged) {
			g_pathsLogged = true;
			LogPaths(hud->uiMovie.get());
		}
		Apply(hud->uiMovie.get(), a_live, a_player && a_player->IsSneaking());
		if (a_live != g_hidden) {
			g_hidden = a_live;
			++g_changes;
			logger::info("hud: Skyrim's crosshair, bars, compass, charge meters and arrows {}", a_live ? "hidden (overlay live)" : "shown again");
		}
	}

	void LogSecond()
	{
		logger::info("hud: Skyrim's HUD parts {} ({} changes in all)", g_hidden ? "HIDDEN" : "shown", g_changes);
	}
}
