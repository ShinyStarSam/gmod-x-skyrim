// Geometry route, step G2 (DESIGN.md §12, "G2 in detail"): Skyrim reads the render block
// Local\GmodSkyrim_render_v1. Opened like the overlay block (whoever arrives first fills in the
// header); on opening, Skyrim skips whatever is unread and adds 1 to reader_epoch, so Garry's Mod
// sends RESET and everything again. Drained once per frame from the in-frame hook, at most 48 MB
// per frame (as SkyCraft's DrainMessages, MIT). Every message is checked (gmsky::rnd::Parse*);
// broken ones are counted, the first few logged with the reason, and skipped.
#include "Render.h"

#include <mutex>

namespace gmsky::Render
{
	namespace
	{
		constexpr uint64_t kMaxBytesPerFrame = 48ull << 20;

		HANDLE g_mapping = nullptr;
		uint8_t* g_base = nullptr;
		uint64_t g_lastOpenTry = 0;
		std::string g_lastError;
		uint32_t g_readerEpoch = 0;
		uint64_t g_serial = 0;

		std::unordered_map<uint32_t, Model> g_models;
		std::unordered_map<uint32_t, Texture> g_textures;
		Frame g_frame;

		// Since the last LogSecond, under g_lock (render thread writes, game thread reads).
		struct Counts
		{
			uint64_t messages[6]{};  // by GMSKY_RND_* type (0 = unknown type)
			uint64_t bytes = 0;
			uint64_t resets = 0, forgets = 0;
			uint64_t badRing = 0, badMessages = 0;
			uint64_t drains = 0;
			double drainMsMax = 0;
		};
		std::mutex g_lock;
		Counts g_counts;
		// Snapshot for the log line (under g_lock).
		size_t g_heldModels = 0, g_heldTextures = 0;
		uint64_t g_heldBytes = 0;
		Frame g_logFrame;
		uint32_t g_unknownModels = 0, g_writerSession = 0, g_lastResetSession = 0;
		int g_badLogged = 0;

		GmskyRenderCtl& Ctl() { return *reinterpret_cast<GmskyRenderCtl*>(g_base); }

		bool Fail(std::string a_msg)
		{
			if (a_msg != g_lastError) {
				logger::error("render block: {} (retrying every 2 s)", a_msg);
				g_lastError = std::move(a_msg);
			}
			return false;
		}

		bool Open()
		{
			// 64 MB; Windows only uses memory for the pages written.
			g_mapping = CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE, 0, GMSKY_RND_MAP_SIZE, GMSKY_RND_SHM_NAME);
			if (!g_mapping) {
				return Fail(fmt::format("CreateFileMappingW failed, error {}", GetLastError()));
			}
			auto* view = static_cast<uint8_t*>(MapViewOfFile(g_mapping, FILE_MAP_ALL_ACCESS, 0, 0, GMSKY_RND_MAP_SIZE));
			if (!view) {
				const auto err = GetLastError();
				CloseHandle(g_mapping);
				g_mapping = nullptr;
				return Fail(fmt::format("MapViewOfFile failed, error {}", err));
			}
			auto& c = *reinterpret_cast<GmskyRenderCtl*>(view);
			std::atomic_ref<uint32_t> magic(c.magic);
			uint32_t expected = 0;
			bool created = false;
			if (magic.compare_exchange_strong(expected, GMSKY_MAGIC_INIT)) {
				c.version = GMSKY_RND_VERSION;
				c.map_size = GMSKY_RND_MAP_SIZE;
				magic.store(GMSKY_RND_MAGIC, std::memory_order_release);
				created = true;
			} else {
				const auto start = GetTickCount64();
				while (magic.load(std::memory_order_acquire) != GMSKY_RND_MAGIC) {
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
			if (c.version != GMSKY_RND_VERSION || c.map_size != GMSKY_RND_MAP_SIZE) {
				const auto v = c.version;
				const auto s = c.map_size;
				UnmapViewOfFile(view);
				CloseHandle(g_mapping);
				g_mapping = nullptr;
				return Fail(fmt::format("version {} / size {} in the block, this plugin {} / {}", v, s, GMSKY_RND_VERSION, GMSKY_RND_MAP_SIZE));
			}
			g_base = view;
			g_lastError.clear();
			// Attach as the reader: skip the unread half stream, ask Garry's Mod to start again.
			const auto head = rnd::Counter(g_base, GMSKY_RND_HEAD_OFF).load(std::memory_order_acquire);
			const auto skipped = head - rnd::Counter(g_base, GMSKY_RND_TAIL_OFF).load(std::memory_order_acquire);
			rnd::Counter(g_base, GMSKY_RND_TAIL_OFF).store(head, std::memory_order_release);
			g_readerEpoch = std::atomic_ref<uint32_t>(c.reader_epoch).fetch_add(1, std::memory_order_acq_rel) + 1;
			logger::info("render block {} ({} bytes); reader epoch {}, {} unread bytes skipped, Garry's Mod session {}",
				created ? "created" : "opened (already created by Garry's Mod)", GMSKY_RND_MAP_SIZE, g_readerEpoch, skipped,
				std::atomic_ref<uint32_t>(c.writer_session).load());
			return true;
		}

		void Bad(const char* a_what, uint32_t a_type, uint32_t a_bytes)
		{
			std::lock_guard lock(g_lock);
			++g_counts.badMessages;
			if (g_badLogged < 20) {
				++g_badLogged;
				logger::warn("render block: message type {} ({} bytes) skipped: {}", a_type, a_bytes, a_what);
			}
		}

		void OnMessage(uint32_t a_type, const uint8_t* a_p, uint32_t a_bytes)
		{
			switch (a_type) {
			case GMSKY_RND_RESET:
				{
					GmskyRndReset r{};
					if (a_bytes != sizeof(r)) {
						return Bad("reset of the wrong size", a_type, a_bytes);
					}
					std::memcpy(&r, a_p, sizeof(r));
					g_models.clear();
					g_textures.clear();
					g_frame = Frame{};
					std::lock_guard lock(g_lock);
					++g_counts.resets;
					g_lastResetSession = r.session;
					logger::info("render block: RESET from Garry's Mod session {} (answering reader epoch {}; ours {}): models and textures dropped",
						r.session, r.reader_epoch, g_readerEpoch);
					return;
				}
			case GMSKY_RND_TEXTURE:
				{
					Texture t;
					const uint8_t* px = nullptr;
					if (const char* err = rnd::ParseTexture(a_p, a_bytes, t.header, px)) {
						return Bad(err, a_type, a_bytes);
					}
					t.pixels.assign(px, px + t.header.bytes);
					t.serial = ++g_serial;
					g_textures[t.header.id] = std::move(t);
					return;
				}
			case GMSKY_RND_MODEL:
				{
					rnd::ModelView mv{};
					if (const char* err = rnd::ParseModel(a_p, a_bytes, mv)) {
						return Bad(err, a_type, a_bytes);
					}
					Model m;
					m.header = mv.model;
					m.parts.assign(mv.parts, mv.parts + mv.model.part_count);
					m.vertices.assign(mv.vertices, mv.vertices + mv.model.vertex_count);
					m.indices.assign(mv.indices, mv.indices + mv.model.index_count);
					m.serial = ++g_serial;
					g_models[m.header.id] = std::move(m);
					return;
				}
			case GMSKY_RND_FORGET:
				{
					GmskyRndForget f{};
					if (a_bytes != sizeof(f)) {
						return Bad("forget of the wrong size", a_type, a_bytes);
					}
					std::memcpy(&f, a_p, sizeof(f));
					if (f.kind == GMSKY_RND_FORGET_MODEL) {
						g_models.erase(f.id);
					} else if (f.kind == GMSKY_RND_FORGET_TEXTURE) {
						g_textures.erase(f.id);
					}
					std::lock_guard lock(g_lock);
					++g_counts.forgets;
					return;
				}
			case GMSKY_RND_FRAME:
				{
					rnd::FrameView fv{};
					if (const char* err = rnd::ParseFrame(a_p, a_bytes, fv)) {
						return Bad(err, a_type, a_bytes);
					}
					// Only the newest frame matters; older ones in the same drain are overwritten.
					g_frame.valid = true;
					g_frame.header = fv.frame;
					g_frame.objects.assign(fv.objects, fv.objects + fv.frame.object_count);
					g_frame.matrices.assign(fv.matrices, fv.matrices + fv.frame.matrix_count);
					return;
				}
			default:
				return Bad("unknown type", a_type, a_bytes);
			}
		}

		double QpcMs(int64_t a_ticks)
		{
			static const double freq = [] { LARGE_INTEGER f{}; QueryPerformanceFrequency(&f); return double(f.QuadPart); }();
			return double(a_ticks) * 1000.0 / freq;
		}

		int64_t Qpc()
		{
			LARGE_INTEGER q{};
			QueryPerformanceCounter(&q);
			return q.QuadPart;
		}
	}

	void Drain()
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
		const auto start = Qpc();
		uint64_t messages[6]{};
		bool bad = false;
		const auto bytes = rnd::Drain(
			g_base, [&](uint32_t a_type, const uint8_t* a_p, uint32_t a_bytes) {
				++messages[a_type < 6 ? a_type : 0];
				OnMessage(a_type, a_p, a_bytes);
			},
			kMaxBytesPerFrame, bad);
		const double ms = QpcMs(Qpc() - start);

		uint32_t unknown = 0;
		for (const auto& o : g_frame.objects) {
			unknown += g_models.contains(o.model_id) ? 0 : 1;
		}
		uint64_t held = 0;
		for (const auto& [id, m] : g_models) {
			held += m.vertices.size() * sizeof(GmskyRndVertex) + m.indices.size() * 4;
		}
		for (const auto& [id, t] : g_textures) {
			held += t.pixels.size();
		}
		std::atomic_ref<uint64_t>(Ctl().messages_read).fetch_add(messages[1] + messages[2] + messages[3] + messages[4] + messages[5] + messages[0], std::memory_order_relaxed);

		std::lock_guard lock(g_lock);
		for (int k = 0; k < 6; ++k) {
			g_counts.messages[k] += messages[k];
		}
		g_counts.bytes += bytes;
		g_counts.badRing += bad ? 1 : 0;
		++g_counts.drains;
		g_counts.drainMsMax = std::max(g_counts.drainMsMax, ms);
		if (bad && g_badLogged < 20) {
			++g_badLogged;
			logger::warn("render block: the ring held something that isn't a message; everything pending skipped");
		}
		g_heldModels = g_models.size();
		g_heldTextures = g_textures.size();
		g_heldBytes = held;
		g_unknownModels = unknown;
		g_writerSession = std::atomic_ref<uint32_t>(Ctl().writer_session).load(std::memory_order_relaxed);
		if (g_frame.valid && (!g_logFrame.valid || g_logFrame.header.frame != g_frame.header.frame)) {
			g_logFrame.valid = true;
			g_logFrame.header = g_frame.header;
			g_logFrame.objects.assign(g_frame.objects.begin(), g_frame.objects.begin() + std::min<size_t>(g_frame.objects.size(), 3));
			g_logFrame.matrices.assign(g_frame.matrices.begin(), g_frame.matrices.begin() + std::min<size_t>(g_frame.matrices.size(), 1));
		}
	}

	const std::unordered_map<uint32_t, Model>& Models() { return g_models; }
	const std::unordered_map<uint32_t, Texture>& Textures() { return g_textures; }
	const Frame& NewestFrame() { return g_frame; }

	void LogSecond()
	{
		Counts c;
		Frame f;
		size_t models = 0, textures = 0;
		uint64_t held = 0;
		uint32_t unknown = 0, session = 0;
		{
			std::lock_guard lock(g_lock);
			c = std::exchange(g_counts, Counts{});
			f = g_logFrame;
			models = g_heldModels;
			textures = g_heldTextures;
			held = g_heldBytes;
			unknown = g_unknownModels;
			session = g_writerSession;
		}
		if (!g_base) {
			return;
		}
		if (c.bytes == 0 && models == 0 && !f.valid && c.badMessages == 0) {
			static uint64_t quiet = 0;
			if (++quiet % 10 == 1) {
				logger::info("render block: nothing from Garry's Mod yet ({} drains this second; writer session {})", c.drains, session);
			}
			return;
		}
		std::string frame = "no frame yet";
		if (f.valid) {
			frame = fmt::format("newest frame {} ({:.1f} ms old, origin epoch {}): {} objects, {} matrices, {} with unknown models",
				f.header.frame, QpcMs(Qpc() - f.header.qpc), f.header.origin_epoch, f.header.object_count, f.header.matrix_count, unknown);
			if (!f.objects.empty() && !f.matrices.empty()) {
				const auto& o = f.objects[0];
				const auto& m = f.matrices[0].m;
				frame += fmt::format("; first: model {} x{} matrices, colour {:08X}, at ({:.1f}, {:.1f}, {:.1f})", o.model_id, o.matrix_count, o.color, m[0][3], m[1][3], m[2][3]);
			}
		}
		logger::info("render block: {} drains, {} bytes (max {:.3f} ms a drain); messages: {} reset, {} texture, {} model, {} forget, {} frame, {} unknown; {} bad messages, {} bad ring; holding {} models, {} textures ({} KB); {}",
			c.drains, c.bytes, c.drainMsMax, c.messages[1], c.messages[2], c.messages[3], c.messages[4], c.messages[5], c.messages[0], c.badMessages, c.badRing,
			models, textures, held / 1024, frame);
	}
}
