// Phase 1 step 1: input bridge. Skyrim keeps keyboard focus. While the puppet is on, Skyrim's
// window is in front and no Skyrim menu has the input, keyboard and mouse go to Garry's Mod
// (GmskyInput slot) instead of Skyrim's own player. Skyrim keeps Esc, ~, J, M, F9, and E for
// activating doors, people and containers. Approach and hooks adapted from SkyCraft (MIT).
#include "Input.h"

#include "Link.h"
#include "Puppet.h"

namespace gmsky::Input
{
	namespace
	{
		// DirectInput scan codes (what Skyrim reports for the keyboard).
		constexpr uint32_t kDikEscape = 0x01;
		constexpr uint32_t kDikConsole = 0x29;  // ~
		constexpr uint32_t kDikJ = 0x24;        // journal
		constexpr uint32_t kDikM = 0x32;        // map
		constexpr uint32_t kDikF9 = 0x43;       // quickload
		constexpr uint32_t kDikE = 0x12;        // Skyrim activate

		bool IsSkyrimMenuKey(uint32_t a_code)
		{
			return a_code == kDikEscape || a_code == kDikConsole || a_code == kDikJ || a_code == kDikM || a_code == kDikF9;
		}

		// Garry's Mod's default keys.
		uint32_t ActionForKey(uint32_t a_dik)
		{
			switch (a_dik) {
			case 0x11: return GMSKY_IN_FORWARD;  // W
			case 0x1F: return GMSKY_IN_BACK;     // S
			case 0x1E: return GMSKY_IN_LEFT;     // A
			case 0x20: return GMSKY_IN_RIGHT;    // D
			case 0x39: return GMSKY_IN_JUMP;     // Space
			case 0x1D: return GMSKY_IN_DUCK;     // Left Ctrl
			case 0x2A: return GMSKY_IN_SPEED;    // Left Shift (sprint)
			case 0x38: return GMSKY_IN_WALK;     // Left Alt
			case 0x13: return GMSKY_IN_RELOAD;   // R
			default: return 0;
			}
		}

		bool g_routing = false;
		uint32_t g_held = 0;
		int64_t g_mouseX = 0, g_mouseY = 0;
		int32_t g_wheel = 0;
		uint64_t g_frame = 0;
		std::string g_reason = "not started";

		// Per-second statistics.
		uint32_t g_events = 0, g_writes = 0, g_activations = 0;
		int64_t g_secMouseX = 0, g_secMouseY = 0;

		bool SkyrimHasFocus()
		{
			DWORD pid = 0;
			if (HWND fg = GetForegroundWindow()) {
				GetWindowThreadProcessId(fg, &pid);
			}
			return pid == GetCurrentProcessId();
		}

		bool BlockingMenuOpen()
		{
			auto* ui = RE::UI::GetSingleton();
			if (!ui) {
				return true;
			}
			for (const auto& menu : ui->menuStack) {
				if (menu && menu->menuFlags.any(RE::UI_MENU_FLAGS::kPausesGame, RE::UI_MENU_FLAGS::kUsesCursor)) {
					return true;
				}
			}
			return false;
		}

		void Write()
		{
			auto& link = Link::Get();
			if (!link.IsOpen() || !link.OwnsSide()) {
				return;
			}
			GmskyInput in{};
			in.frame = g_frame;
			in.active = g_routing ? 1u : 0u;
			in.buttons = g_routing ? g_held : 0u;
			in.mouse_x_total = g_mouseX;
			in.mouse_y_total = g_mouseY;
			in.wheel_total = g_wheel;
			SlotWrite(link.Shared().input, in);
			++g_writes;
		}

		// Recomputes whether input goes to Garry's Mod. On a change, releases held keys and
		// publishes at once (a Skyrim menu pauses the per-frame update, so don't wait for it).
		void UpdateRouting()
		{
			std::string reason;
			if (!Puppet::IsActive()) {
				reason = "puppet off";
			} else if (!SkyrimHasFocus()) {
				reason = "Skyrim window not in front";
			} else if (BlockingMenuOpen()) {
				reason = "a Skyrim menu is open";
			}
			const bool route = reason.empty();
			if (route != g_routing || (!route && reason != g_reason)) {
				logger::info("INPUT -> {}{}", route ? "Garry's Mod" : "Skyrim: ", reason);
			}
			if (route != g_routing) {
				g_routing = route;
				g_held = 0;
				Write();
			}
			g_reason = reason;
		}

		// E on something Skyrim can activate (door, person, container, item): activate it in
		// Skyrim, using Skyrim's own crosshair target. (SkyCraft's ActivateSkyrimTarget.)
		void ActivateSkyrimTarget()
		{
			auto* pick = RE::CrosshairPickData::GetSingleton();
			auto* player = RE::PlayerCharacter::GetSingleton();
			if (!pick || !player) {
				return;
			}
			auto target = pick->GetActiveTarget().get();
			if (!target || target.get() == player) {
				logger::info("E: nothing to activate under the crosshair");
				return;
			}
			target->ActivateRef(player, 0, nullptr, 0, false);
			++g_activations;
			logger::info("E: activated {:08X} ({})", target->GetFormID(), target->GetDisplayFullName());
		}

		class Sink final : public RE::BSTEventSink<RE::InputEvent*>
		{
		public:
			static Sink* Get()
			{
				static Sink sink;
				return &sink;
			}

			RE::BSEventNotifyControl ProcessEvent(RE::InputEvent* const* a_event, RE::BSTEventSource<RE::InputEvent*>*) override
			{
				UpdateRouting();
				if (!a_event || !g_routing) {
					return RE::BSEventNotifyControl::kContinue;
				}
				bool changed = false;
				for (auto* e = *a_event; e; e = e->next) {
					++g_events;
					if (e->GetEventType() == RE::INPUT_EVENT_TYPE::kMouseMove) {
						auto* mm = e->AsMouseMoveEvent();
						g_mouseX += mm->mouseInputX;
						g_mouseY += mm->mouseInputY;
						g_secMouseX += mm->mouseInputX;
						g_secMouseY += mm->mouseInputY;
						changed = true;
						continue;
					}
					if (e->GetEventType() != RE::INPUT_EVENT_TYPE::kButton) {
						continue;
					}
					auto* b = e->AsButtonEvent();
					const bool down = b->IsDown();
					const bool up = b->IsUp();
					if (!down && !up) {
						continue;  // held, already counted
					}
					const auto code = b->GetIDCode();
					uint32_t action = 0;
					if (b->GetDevice() == RE::INPUT_DEVICE::kKeyboard) {
						if (code == kDikE) {
							if (down) {
								ActivateSkyrimTarget();
							}
							continue;
						}
						action = ActionForKey(code);
					} else if (b->GetDevice() == RE::INPUT_DEVICE::kMouse) {
						if (code == 8 || code == 9) {  // wheel up / down
							if (down) {
								g_wheel += code == 8 ? 1 : -1;
								changed = true;
							}
							continue;
						}
						action = code == 0 ? GMSKY_IN_ATTACK : code == 1 ? GMSKY_IN_ATTACK2 : 0;
					}
					if (action) {
						g_held = down ? (g_held | action) : (g_held & ~action);
						changed = true;
					}
				}
				if (changed) {
					Write();
				}
				return RE::BSEventNotifyControl::kContinue;
			}
		};

		// PlayerControls turns input into Skyrim's own player actions (move, look, attack, jump).
		// While input goes to Garry's Mod it gets none. (Turning Skyrim's player controls off
		// instead would also stop NPCs fighting the player, as SkyCraft found.)
		struct PlayerControlsHook
		{
			static RE::BSEventNotifyControl thunk(RE::PlayerControls* a_this, RE::InputEvent* const* a_event, RE::BSTEventSource<RE::InputEvent*>* a_source)
			{
				UpdateRouting();
				if (!g_routing) {
					return func(a_this, a_event, a_source);
				}
				return RE::BSEventNotifyControl::kContinue;
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};

		// MenuControls opens Skyrim's menus. While input goes to Garry's Mod, only Esc, ~, J, M
		// and F9 reach it. (Same re-linking trick as SkyCraft: pass it a list of just those events.)
		struct MenuControlsHook
		{
			static RE::BSEventNotifyControl thunk(RE::MenuControls* a_this, RE::InputEvent* const* a_event, RE::BSTEventSource<RE::InputEvent*>* a_source)
			{
				UpdateRouting();
				if (!a_event || !*a_event || !g_routing) {
					return func(a_this, a_event, a_source);
				}
				std::vector<RE::InputEvent*> all;
				std::vector<RE::InputEvent*> keep;
				for (auto* e = *a_event; e; e = e->next) {
					all.push_back(e);
					if (e->GetEventType() == RE::INPUT_EVENT_TYPE::kButton && e->GetDevice() == RE::INPUT_DEVICE::kKeyboard &&
						IsSkyrimMenuKey(e->AsButtonEvent()->GetIDCode())) {
						keep.push_back(e);
					}
				}
				if (keep.empty()) {
					return RE::BSEventNotifyControl::kContinue;
				}
				std::vector<RE::InputEvent*> savedNext;
				for (auto* e : all) {
					savedNext.push_back(e->next);
				}
				for (std::size_t i = 0; i < keep.size(); ++i) {
					keep[i]->next = i + 1 < keep.size() ? keep[i + 1] : nullptr;
				}
				RE::InputEvent* head = keep.front();
				const auto result = func(a_this, &head, a_source);
				for (std::size_t i = 0; i < all.size(); ++i) {
					all[i]->next = savedNext[i];
				}
				return result;
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};
	}

	void Install()
	{
		if (auto* devices = RE::BSInputDeviceManager::GetSingleton()) {
			devices->AddEventSink(Sink::Get());
		}
		REL::Relocation<std::uintptr_t> menuVtbl{ RE::VTABLE_MenuControls[0] };
		MenuControlsHook::func = menuVtbl.write_vfunc(0x1, MenuControlsHook::thunk);
		REL::Relocation<std::uintptr_t> playerVtbl{ RE::VTABLE_PlayerControls[0] };
		PlayerControlsHook::func = playerVtbl.write_vfunc(0x1, PlayerControlsHook::thunk);
		logger::info("input hooks installed");
	}

	void PerFrame(uint64_t a_frame)
	{
		g_frame = a_frame;
		UpdateRouting();
		Write();
	}

	void LogSecond()
	{
		logger::info("INPUT {}: held={:#x} mouse this second=({}, {}) totals=({}, {}) wheel={} events={} writes={} activations={}",
			g_routing ? "to Garry's Mod" : "to Skyrim", g_held, g_secMouseX, g_secMouseY, g_mouseX, g_mouseY, g_wheel,
			g_events, g_writes, g_activations);
		g_events = g_writes = g_activations = 0;
		g_secMouseX = g_secMouseY = 0;
	}
}
