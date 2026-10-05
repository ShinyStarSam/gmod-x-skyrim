#include "Body.h"
#include "Camera.h"
#include "Combat.h"
#include "Draw.h"
#include "Input.h"
#include "Link.h"
#include "Probe.h"

namespace
{
	// Logs go to <project>/logs/skyrim.log (never into Documents/My Games: AGENTS.md rule 4).
	void SetupLog()
	{
		const std::filesystem::path dir = std::filesystem::path(GMSKY_PROJECT_ROOT) / "logs";
		std::error_code ec;
		std::filesystem::create_directories(dir, ec);
		auto sink = std::make_shared<spdlog::sinks::basic_file_sink_mt>((dir / "skyrim.log").string(), true);
		auto log = std::make_shared<spdlog::logger>("global", std::move(sink));
		log->set_level(spdlog::level::info);
		log->flush_on(spdlog::level::info);
		spdlog::set_default_logger(std::move(log));
		spdlog::set_pattern("[%H:%M:%S.%e] [%l] %v");
	}

	void OnMessage(SKSE::MessagingInterface::Message* a_msg)
	{
		switch (a_msg->type) {
		case SKSE::MessagingInterface::kDataLoaded:
			// If this fails, the per-frame probe retries every 2 s.
			gmsky::Link::Get().Open();
			gmsky::Probe::Install();
			gmsky::Input::Install();
			gmsky::Camera::Install();
			gmsky::Body::Install();
			gmsky::Draw::Install();
			gmsky::Combat::Install();
			break;
		case SKSE::MessagingInterface::kPostLoadGame:
		case SKSE::MessagingInterface::kNewGame:
			gmsky::Probe::OnGameLoaded();
			break;
		default:
			break;
		}
	}
}

SKSEPluginLoad(const SKSE::LoadInterface* a_skse)
{
	SKSE::Init(a_skse, { .trampoline = true, .trampolineSize = 512 });
	SetupLog();
	logger::info("gmsky 0.1.0 loading (runtime {}), project {}", a_skse->RuntimeVersion().string(), GMSKY_PROJECT_ROOT);
	SKSE::GetMessagingInterface()->RegisterListener(OnMessage);
	return true;
}
