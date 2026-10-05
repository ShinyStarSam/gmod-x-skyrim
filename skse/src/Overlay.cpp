// Step 2b-4: Skyrim's side of the overlay picture block. Opens Local\GmodSkyrim_overlay_v1 (same
// rules as protocol/src/overlay.rs: whoever arrives first fills in the header), and every frame
// writes Skyrim's back buffer size, read from its swap chain (the same size SkyCraft's Present
// hook reads, Overlay.cpp DrawOverlay; ours comes in step 2c). Garry's Mod logs whether its picture
// has that size. Doesn't attach as the reader yet: taking pictures is step 2c.
#include "Overlay.h"

#include "gmsky_protocol.h"

namespace gmsky::Overlay
{
	namespace
	{
		HANDLE g_mapping = nullptr;
		uint8_t* g_base = nullptr;
		uint64_t g_lastOpenTry = 0;
		std::string g_lastError;
		uint32_t g_w = 0, g_h = 0;
		uint64_t g_sizeChanges = 0;
		std::string g_sizeError;

		GmskyOverlayCtl& Ctl() { return *reinterpret_cast<GmskyOverlayCtl*>(g_base); }

		GmskyOverlaySlotHdr SlotHdr(uint32_t a_slot)
		{
			GmskyOverlaySlotHdr h{};
			std::memcpy(&h, g_base + GMSKY_OV_SLOT_HDR_OFF + a_slot * sizeof(GmskyOverlaySlotHdr), sizeof(h));
			return h;
		}

		bool Fail(std::string a_msg)
		{
			if (a_msg != g_lastError) {
				logger::error("overlay block: {} (retrying every 2 s)", a_msg);
				g_lastError = std::move(a_msg);
			}
			return false;
		}

		bool Open()
		{
			// 101 MB; Windows only uses memory for the pages written.
			g_mapping = CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE, 0, GMSKY_OV_MAP_SIZE, GMSKY_OV_SHM_NAME);
			if (!g_mapping) {
				return Fail(fmt::format("CreateFileMappingW failed, error {}", GetLastError()));
			}
			auto* view = static_cast<uint8_t*>(MapViewOfFile(g_mapping, FILE_MAP_ALL_ACCESS, 0, 0, GMSKY_OV_MAP_SIZE));
			if (!view) {
				const auto err = GetLastError();
				CloseHandle(g_mapping);
				g_mapping = nullptr;
				return Fail(fmt::format("MapViewOfFile failed, error {}", err));
			}
			auto& c = *reinterpret_cast<GmskyOverlayCtl*>(view);
			std::atomic_ref<uint32_t> magic(c.magic);
			uint32_t expected = 0;
			bool created = false;
			if (magic.compare_exchange_strong(expected, GMSKY_MAGIC_INIT)) {
				c.version = GMSKY_OV_VERSION;
				c.map_size = GMSKY_OV_MAP_SIZE;
				magic.store(GMSKY_OV_MAGIC, std::memory_order_release);
				created = true;
			} else {
				const auto start = GetTickCount64();
				while (magic.load(std::memory_order_acquire) != GMSKY_OV_MAGIC) {
					if (GetTickCount64() - start > 1000) {
						const auto m = magic.load();
						UnmapViewOfFile(view);
						CloseHandle(g_mapping);
						g_mapping = nullptr;
						return Fail(fmt::format("never initialized (magic {:#x})", m));
					}
					Sleep(0);
				}
			}
			if (c.version != GMSKY_OV_VERSION || c.map_size != GMSKY_OV_MAP_SIZE) {
				const auto v = c.version;
				const auto s = c.map_size;
				UnmapViewOfFile(view);
				CloseHandle(g_mapping);
				g_mapping = nullptr;
				return Fail(fmt::format("version {} / size {} in the block, this plugin {} / {}", v, s, GMSKY_OV_VERSION, GMSKY_OV_MAP_SIZE));
			}
			g_base = view;
			g_lastError.clear();
			logger::info("overlay block {} ({} bytes)", created ? "created" : "opened (already created by Garry's Mod)", GMSKY_OV_MAP_SIZE);
			return true;
		}

		// Skyrim's back buffer size, from its swap chain. False (and why, logged once a second) if
		// it can't be read yet (before the renderer exists).
		bool BackBufferSize(uint32_t& a_w, uint32_t& a_h)
		{
			auto* window = RE::BSGraphics::Renderer::GetCurrentRenderWindow();
			if (!window || !window->swapChain) {
				g_sizeError = "no render window / swap chain yet";
				return false;
			}
			REX::W32::DXGI_SWAP_CHAIN_DESC desc{};
			if (FAILED(window->swapChain->GetDesc(&desc))) {
				g_sizeError = "swap chain GetDesc failed";
				return false;
			}
			g_sizeError.clear();
			a_w = desc.bufferDesc.width;
			a_h = desc.bufferDesc.height;
			return true;
		}
	}

	void PerFrame()
	{
		if (!g_base) {
			const auto now = GetTickCount64();
			if (now - g_lastOpenTry < 2000) {
				return;
			}
			g_lastOpenTry = now;
			if (!Open()) {
				return;
			}
		}
		uint32_t w = 0, h = 0;
		if (!BackBufferSize(w, h)) {
			return;
		}
		if (w != g_w || h != g_h) {
			logger::info("overlay: Skyrim's back buffer is {}x{} (was {}x{})", w, h, g_w, g_h);
			g_w = w;
			g_h = h;
			++g_sizeChanges;
		}
		auto& c = Ctl();
		std::atomic_ref<uint32_t>(c.skyrim_width).store(w, std::memory_order_relaxed);
		std::atomic_ref<uint32_t>(c.skyrim_height).store(h, std::memory_order_relaxed);
		LARGE_INTEGER q{};
		QueryPerformanceCounter(&q);
		std::atomic_ref<int64_t>(c.skyrim_size_qpc).store(q.QuadPart, std::memory_order_release);
	}

	void LogSecond()
	{
		if (!g_base) {
			logger::info("overlay: block not open{}", g_lastError.empty() ? "" : " (" + g_lastError + ")");
			return;
		}
		const auto ss = RE::BSGraphics::Renderer::GetScreenSize();
		auto& c = Ctl();
		const uint32_t s = std::atomic_ref<uint32_t>(c.state).load(std::memory_order_acquire);
		const uint64_t published = std::atomic_ref<uint64_t>(c.frames_published).load(std::memory_order_relaxed);
		// Garry's Mod's newest picture: the middle slot's header (read only, for the log; the
		// picture itself isn't touched until step 2c attaches as the reader).
		const auto hdr = SlotHdr(s & GMSKY_OVS_MIDDLE_MASK);
		LARGE_INTEGER q{}, f{};
		QueryPerformanceCounter(&q);
		QueryPerformanceFrequency(&f);
		const double age = hdr.capture_qpc ? double(q.QuadPart - hdr.capture_qpc) * 1000.0 / double(f.QuadPart) : -1.0;
		logger::info("overlay: Skyrim back buffer {}x{} ({} changes{}), Skyrim's screen setting {}x{}; Garry's Mod: {} pictures published, "
			"newest frame {} {}x{} {:.1f} ms old, writer {}, slots {:#x}",
			g_w, g_h, g_sizeChanges, g_sizeError.empty() ? "" : ", now: " + g_sizeError, ss.width, ss.height, published,
			hdr.frame, hdr.width, hdr.height, age, (s & GMSKY_OVS_WRITER_ON) ? "attached" : "not attached", s);
	}
}
