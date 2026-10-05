// Phase 3 step 3b: Skyrim's actors near the player go to Garry's Mod (the GmskyActors slot), so it
// can give each an invisible stand-in to hit (step 3c). Adapted from SkyCraft (MIT), Combat.cpp
// WriteActorTable: Skyrim's "high process" actors (the ones near the player that Skyrim fully
// runs), loaded, enabled, not the player; here sorted nearest first and capped at
// GMSKY_MAX_ACTORS. Positions use the same conversion as the ground grids (relative to the anchor).
#include "Actors.h"

#include "Link.h"

#include <algorithm>
#include <vector>

namespace gmsky::Actors
{
	namespace
	{
		constexpr float kRange = 6000.0f;  // Skyrim units (~3,200 Garry's Mod units): beyond a bandit's bow
		constexpr double kNoAnchor[3] = { 0.0, 0.0, 0.0 };

		struct Near
		{
			RE::Actor* actor;
			float dist;
		};

		GmskyActors g_out{};
		uint32_t g_writes = 0;
		uint32_t g_maxCount = 0;
		double g_ms = 0.0, g_msMax = 0.0;
		bool g_wasEmpty = true;

		double NowMs()
		{
			LARGE_INTEGER t, f;
			QueryPerformanceCounter(&t);
			QueryPerformanceFrequency(&f);
			return double(t.QuadPart) * 1000.0 / double(f.QuadPart);
		}

		void Fill(GmskyActor& a_r, RE::Actor* a_actor, RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin)
		{
			a_r = GmskyActor{};
			a_r.form_id = a_actor->GetFormID();
			a_r.flags = (a_actor->IsHostileToActor(a_player) ? GMSKY_ACT_HOSTILE : 0u) | (a_actor->IsDead() ? GMSKY_ACT_DEAD : 0u) |
			            (a_actor->IsEssential() ? GMSKY_ACT_ESSENTIAL : 0u) | (a_actor->IsInCombat() ? GMSKY_ACT_IN_COMBAT : 0u);
			const auto p = a_actor->GetPosition();
			const double s[3] = { p.x, p.y, p.z };
			const auto g = SkyrimToGmod(s, kNoAnchor, a_origin.skyrim_origin);
			a_r.pos[0] = float(g.x);
			a_r.pos[1] = float(g.y);
			a_r.pos[2] = float(g.z);
			a_r.yaw_deg = float(SkyrimHeadingToGmodYaw(a_actor->GetAngleZ()));
			a_r.radius = float(std::clamp(double(a_actor->GetBoundRadius()) * kGmodPerSkyrim, 4.0, 400.0));
			a_r.height = float(std::clamp(double(a_actor->GetHeight()) * kGmodPerSkyrim, 8.0, 800.0));
			const float maxHealth = a_actor->GetActorValueMax(RE::ActorValue::kHealth);
			a_r.health_frac = maxHealth > 0.0f ? std::clamp(a_actor->AsActorValueOwner()->GetActorValue(RE::ActorValue::kHealth) / maxHealth, 0.0f, 1.0f) : 0.0f;
			a_r.level = static_cast<uint16_t>(std::min<uint32_t>(a_actor->GetLevel(), 0xFFFF));
			if (const char* name = a_actor->GetDisplayFullName()) {
				strncpy_s(a_r.name, name, _TRUNCATE);
			}
		}

		const char* FlagText(uint32_t a_flags)
		{
			static char buf[64];
			buf[0] = 0;
			if (a_flags & GMSKY_ACT_HOSTILE) strcat_s(buf, " hostile");
			if (a_flags & GMSKY_ACT_IN_COMBAT) strcat_s(buf, " fighting");
			if (a_flags & GMSKY_ACT_ESSENTIAL) strcat_s(buf, " essential");
			if (a_flags & GMSKY_ACT_DEAD) strcat_s(buf, " dead");
			return buf;
		}
	}

	void PerFrame(RE::PlayerCharacter* a_player, const GmskyOrigin& a_origin, uint64_t a_frame)
	{
		auto& link = Link::Get();
		if (!link.IsOpen() || !link.OwnsSide()) {
			return;
		}
		const double t0 = NowMs();
		static std::vector<Near> nearby;
		nearby.clear();
		auto* lists = RE::ProcessLists::GetSingleton();
		if (lists && a_origin.epoch != 0) {
			const auto playerPos = a_player->GetPosition();
			for (auto& handle : lists->highActorHandles) {
				auto ptr = handle.get();
				auto* actor = ptr.get();
				if (!actor || actor == a_player || actor->IsDisabled() || !actor->Is3DLoaded()) {
					continue;
				}
				const float d = actor->GetPosition().GetDistance(playerPos);
				if (d <= kRange) {
					nearby.push_back({ actor, d });
				}
			}
		}
		std::sort(nearby.begin(), nearby.end(), [](const Near& a, const Near& b) { return a.dist < b.dist; });
		const uint32_t count = uint32_t(std::min<std::size_t>(nearby.size(), GMSKY_MAX_ACTORS));
		g_out.count = count;
		g_out.origin_epoch = a_origin.epoch;
		g_out.skyrim_frame = a_frame;
		for (uint32_t i = 0; i < count; ++i) {
			Fill(g_out.actors[i], nearby[i].actor, a_player, a_origin);
		}
		SlotWrite(link.Shared().actors, g_out);
		++g_writes;
		g_maxCount = std::max(g_maxCount, count);
		const double ms = NowMs() - t0;
		g_ms += ms;
		g_msMax = std::max(g_msMax, ms);
		if ((count == 0) != g_wasEmpty) {
			g_wasEmpty = count == 0;
			logger::info("actors: {}", g_wasEmpty ? "none near the player any more" : "actors near the player, sent to Garry's Mod");
		}
	}

	void LogSecond()
	{
		if (g_writes == 0) {
			return;
		}
		std::string nearest;
		auto* player = RE::PlayerCharacter::GetSingleton();
		for (uint32_t i = 0; i < g_out.count && i < 3 && player; ++i) {
			const auto& a = g_out.actors[i];
			auto* actor = RE::TESForm::LookupByID<RE::Actor>(a.form_id);
			const float d = actor ? actor->GetPosition().GetDistance(player->GetPosition()) : -1.0f;
			nearest += fmt::format(" | {} {:08X} lvl {} at {:.0f} ({:.0f} Garry's Mod) hp {:.0f}%{}", a.name, a.form_id, a.level, d,
				d * kGmodPerSkyrim, a.health_frac * 100.0f, FlagText(a.flags));
		}
		if (g_out.count > 0 || g_maxCount > 0) {
			logger::info("ACTORS {} now (max {} this second), {} writes, {:.3f} ms avg ({:.3f} max){}", g_out.count, g_maxCount, g_writes,
				g_ms / g_writes, g_msMax, nearest);
		}
		g_writes = 0;
		g_maxCount = 0;
		g_ms = g_msMax = 0.0;
	}
}
