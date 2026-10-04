// Phase 1 step 3f: Skyrim's player body while Garry's Mod drives it (DESIGN.md).
//
// Adapted from SkyCraft (MIT, https://github.com/chasmlol/SkyCraft, skse/src/DigPhysics.cpp):
//  - Skyrim's character body collects contact points with the world and pushes itself out of
//    them. A contact above the feet (a wall at the head, a rock Garry's Mod put the player in)
//    would only push the body away from where Garry's Mod put it, so it is dropped; only what's
//    under the feet holds the body. In noclip every contact is dropped.
//  - Every actor's movement update asks for the land's height under it and, if that is more than
//    30 units above the actor, puts the actor back on top (Skyrim's rescue for characters that
//    fell through the ground). For our player Garry's Mod decides the height, so it is skipped.
#include "Body.h"

namespace gmsky::Body
{
	namespace
	{
		constexpr float kFeetSlack = 0.3f;    // Havok units (~21 Skyrim units) above the feet still "under" them
		constexpr float kRescueNear = 64.0f;  // Skyrim units: an actor this close to our player is our player

		std::atomic<const RE::hkpCollidable*> g_body{ nullptr };
		std::atomic<float> g_feetHavokZ{ 0.0f };
		std::atomic<float> g_x{ 0.0f }, g_y{ 0.0f };
		std::atomic<bool> g_noclip{ false };
		bool g_rescueHooked = false;

		// Per second (hooks may run off the main thread).
		std::atomic<uint32_t> g_droppedAbove{ 0 }, g_droppedNoclip{ 0 }, g_rescuesSkipped{ 0 };

		const RE::hkpCollidable* Root(const RE::hkpCdBody* a_body)
		{
			while (a_body && a_body->parent) {
				a_body = a_body->parent;
			}
			return static_cast<const RE::hkpCollidable*>(a_body);
		}

		struct AddCdPointHook
		{
			static void thunk(RE::bhkCharacterPointCollector* a_this, const RE::hkpCdPoint& a_point)
			{
				const auto* body = g_body.load(std::memory_order_relaxed);
				if (body && (Root(a_point.cdBodyA) == body || Root(a_point.cdBodyB) == body)) {
					if (g_noclip.load(std::memory_order_relaxed)) {
						g_droppedNoclip.fetch_add(1, std::memory_order_relaxed);
						return;
					}
					alignas(16) float p[4];
					_mm_store_ps(p, a_point.contact.position.quad);
					if (p[2] > g_feetHavokZ.load(std::memory_order_relaxed) + kFeetSlack) {
						g_droppedAbove.fetch_add(1, std::memory_order_relaxed);
						return;
					}
				}
				func(a_this, a_point);
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};

		struct LandRescueHook
		{
			static bool thunk(RE::TES* a_tes, const RE::NiPoint3& a_pos, float& a_height)
			{
				const bool found = func(a_tes, a_pos, a_height);
				if (found && a_height > a_pos.z && g_body.load(std::memory_order_relaxed) &&
					std::hypot(a_pos.x - g_x.load(std::memory_order_relaxed), a_pos.y - g_y.load(std::memory_order_relaxed)) < kRescueNear) {
					a_height = a_pos.z;  // "the land is at your feet": no rescue
					g_rescuesSkipped.fetch_add(1, std::memory_order_relaxed);
				}
				return found;
			}
			static inline REL::Relocation<decltype(thunk)> func;
		};
	}

	void Install()
	{
		REL::Relocation<std::uintptr_t> collector{ RE::VTABLE_bhkCharacterPointCollector[0] };
		AddCdPointHook::func = collector.write_vfunc(0x1, AddCdPointHook::thunk);
		// The land-height call in the actor movement update (AE 37507 + 0xCEA, the spot SkyCraft
		// found on 1.7.104). Only patched if it really is a 5-byte call there.
		REL::Relocation<std::uintptr_t> site{ REL::ID(37507), 0xCEA };
		if (REL::Module::IsAE() && *reinterpret_cast<const std::uint8_t*>(site.address()) == 0xE8) {
			LandRescueHook::func = SKSE::GetTrampoline().write_call<5>(site.address(), LandRescueHook::thunk);
			g_rescueHooked = true;
			logger::info("body hooks installed (contacts, land rescue)");
		} else {
			logger::warn("body: the land-rescue call isn't where expected; a noclip player under the ground may be lifted out");
		}
	}

	void Set(const RE::hkpCollidable* a_body, const RE::NiPoint3& a_feet, bool a_noclip)
	{
		g_feetHavokZ.store(a_feet.z * RE::bhkWorld::GetWorldScale(), std::memory_order_relaxed);
		g_x.store(a_feet.x, std::memory_order_relaxed);
		g_y.store(a_feet.y, std::memory_order_relaxed);
		g_noclip.store(a_noclip, std::memory_order_relaxed);
		g_body.store(a_body, std::memory_order_relaxed);
	}

	void Release()
	{
		g_body.store(nullptr, std::memory_order_relaxed);
		g_noclip.store(false, std::memory_order_relaxed);
	}

	void LogSecond()
	{
		logger::info("BODY {}: contacts dropped above the feet={} in noclip={}, land rescues skipped={}{}",
			g_body.load(std::memory_order_relaxed) ? "puppet" : "normal", g_droppedAbove.exchange(0), g_droppedNoclip.exchange(0),
			g_rescuesSkipped.exchange(0), g_rescueHooked ? "" : " (land-rescue hook not installed)");
	}
}
