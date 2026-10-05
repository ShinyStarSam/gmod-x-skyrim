// Step 2c-2: Skyrim's first-person arms and weapon hidden while Garry's Mod's overlay is live
// (Garry's Mod draws the hands and weapon). Adapted from SkyCraft (MIT), Game.cpp
// HideFirstPersonMeshes: only the meshes are marked "culled" (not drawn), never the nodes, which
// Skyrim's camera and animation use; exactly the meshes hidden here are shown again afterwards.
// The first-person model is walked every frame while hidden: Skyrim rebuilds it (weapon drawn or
// changed), and new meshes must be hidden too.
#include "Arms.h"

#include "Overlay.h"
#include "Puppet.h"

namespace gmsky::Arms
{
	namespace
	{
		constexpr double kLiveMs = 500.0;
		// Test 46: Skyrim's arms flashed for single frames around Skyrim menus (cause only partly
		// known; see Overlay::NewestAgeMs). The arms come back only after this many frames in a row
		// say the overlay isn't live; shorter blips are counted, not acted on.
		constexpr int kShowAfterFrames = 3;
		int g_notLiveFrames = 0;
		uint64_t g_blips = 0;

		std::vector<RE::NiPointer<RE::BSGeometry>> g_hidden;
		bool g_hiding = false;
		// Since the last LogSecond.
		uint64_t g_newlyHidden = 0, g_shownAgain = 0, g_changes = 0;
		// The oldest "newest picture" age seen while live (test 46: should stay far below kLiveMs).
		double g_maxLiveAge = 0.0;
		std::string g_lastReason = "start";

		void Show(const char* a_reason)
		{
			uint64_t n = 0;
			for (auto& mesh : g_hidden) {
				// Only if still culled: Skyrim may have shown or culled it itself meanwhile.
				if (mesh && mesh->GetAppCulled()) {
					mesh->SetAppCulled(false);
					++n;
				}
			}
			g_hidden.clear();
			g_shownAgain += n;
			if (g_hiding) {
				++g_changes;
				g_lastReason = a_reason;
				logger::info("arms: Skyrim's first-person meshes shown again ({}; {} meshes)", a_reason, n);
			}
			g_hiding = false;
		}

		void Hide(RE::PlayerCharacter* a_player)
		{
			auto* root = a_player->Get3D(true);  // the first-person model
			if (!root) {
				return;
			}
			uint64_t n = 0;
			RE::BSVisit::TraverseScenegraphGeometries(root, [&](RE::BSGeometry* a_mesh) {
				if (!a_mesh->GetAppCulled()) {
					a_mesh->SetAppCulled(true);
					g_hidden.emplace_back(a_mesh);
					++n;
				}
				return RE::BSVisit::BSVisitControl::kContinue;
			});
			g_newlyHidden += n;
			if (!g_hiding) {
				++g_changes;
				g_lastReason = "overlay live";
				logger::info("arms: Skyrim's first-person meshes hidden (overlay live; {} meshes)", n);
			}
			g_hiding = true;
		}
	}

	void PerFrame(RE::PlayerCharacter* a_player)
	{
		if (!a_player) {
			return;
		}
		const double age = Overlay::NewestAgeMs();
		const bool live = Puppet::IsActive() && age >= 0.0 && age < kLiveMs;
		if (live) {
			if (g_notLiveFrames > 0) {
				++g_blips;  // a short blip: the arms stayed hidden
				logger::info("arms: overlay looked stopped for {} frame(s), arms kept hidden", g_notLiveFrames);
			}
			g_notLiveFrames = 0;
			g_maxLiveAge = std::max(g_maxLiveAge, age);
			Hide(a_player);
		} else if (g_hiding && ++g_notLiveFrames >= kShowAfterFrames) {
			g_notLiveFrames = 0;
			const auto why = !Puppet::IsActive() ? std::string("puppet off") : age < 0.0 ? std::string("no picture") :
													   fmt::format("Garry's Mod's pictures stopped: newest {:.0f} ms old", age);
			Show(why.c_str());
		}
	}

	bool Live() { return g_hiding; }

	void Reset(const char* a_reason)
	{
		g_notLiveFrames = 0;
		Show(a_reason);
	}

	void LogSecond()
	{
		logger::info("arms: {} ({} meshes hidden now; {} newly hidden, {} shown again, {} changes this second; last change: {}; newest picture at most {:.1f} ms old while live; {} blips kept hidden in all)",
			g_hiding ? "HIDDEN" : "shown", g_hidden.size(), g_newlyHidden, g_shownAgain, g_changes, g_lastReason, g_maxLiveAge, g_blips);
		g_newlyHidden = g_shownAgain = g_changes = 0;
		g_maxLiveAge = 0.0;
	}
}
