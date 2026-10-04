#include "Link.h"

namespace gmsky
{
	Link& Link::Get()
	{
		static Link link;
		return link;
	}

	bool Link::Open()
	{
		if (shared_) {
			return true;
		}
		mapping_ = CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE, 0, GMSKY_MAP_SIZE, GMSKY_SHM_NAME);
		if (!mapping_) {
			return Fail(fmt::format("CreateFileMappingW failed, error {}", GetLastError()));
		}
		auto* view = static_cast<GmskyShared*>(MapViewOfFile(mapping_, FILE_MAP_ALL_ACCESS, 0, 0, GMSKY_MAP_SIZE));
		if (!view) {
			const auto err = GetLastError();
			CloseHandle(mapping_);
			mapping_ = nullptr;
			return Fail(fmt::format("MapViewOfFile failed, error {}", err));
		}

		auto& h = view->header;
		std::atomic_ref<uint32_t> magic(h.magic);
		uint32_t expected = 0;
		if (magic.compare_exchange_strong(expected, GMSKY_MAGIC_INIT)) {
			h.version = GMSKY_VERSION;
			h.total_size = sizeof(GmskyShared);
			magic.store(GMSKY_MAGIC, std::memory_order_release);
			logger::info("shared memory created and initialized ({} bytes)", sizeof(GmskyShared));
		} else {
			const auto start = GetTickCount64();
			while (magic.load(std::memory_order_acquire) != GMSKY_MAGIC) {
				if (GetTickCount64() - start > 1000) {
					const auto m = magic.load();
					UnmapViewOfFile(view);
					CloseHandle(mapping_);
					mapping_ = nullptr;
					return Fail(fmt::format("shared memory never initialized (magic {:#x})", m));
				}
				Sleep(0);
			}
			logger::info("shared memory opened (already created by Garry's Mod)");
		}
		if (h.version != GMSKY_VERSION) {
			const auto v = h.version;
			UnmapViewOfFile(view);
			CloseHandle(mapping_);
			mapping_ = nullptr;
			return Fail(fmt::format("protocol version mismatch: shared memory has {}, this plugin is {}", v, GMSKY_VERSION));
		}
		std::atomic_ref<uint32_t>(h.skyrim_pid).store(GetCurrentProcessId(), std::memory_order_release);
		shared_ = view;
		lastError_.clear();
		Heartbeat();
		return true;
	}

	bool Link::Fail(std::string a_msg)
	{
		if (a_msg != lastError_) {
			logger::error("shared memory: {} (retrying every 2 s)", a_msg);
			lastError_ = std::move(a_msg);
		}
		return false;
	}

	bool Link::OwnsSide()
	{
		if (!shared_) {
			return false;
		}
		const uint32_t pid = GetCurrentProcessId();
		std::atomic_ref<uint32_t> owner(shared_->header.skyrim_pid);
		uint32_t cur = owner.load(std::memory_order_acquire);
		return cur == pid || (cur == 0 && owner.compare_exchange_strong(cur, pid));
	}

	void Link::Heartbeat()
	{
		if (shared_) {
			std::atomic_ref<uint64_t>(shared_->header.skyrim_heartbeat).store(GetTickCount64(), std::memory_order_release);
		}
	}

	bool Link::PeerAlive() const
	{
		if (!shared_) {
			return false;
		}
		const auto last = std::atomic_ref<uint64_t>(shared_->header.gmod_heartbeat).load(std::memory_order_acquire);
		return last != 0 && GetTickCount64() - last < GMSKY_HEARTBEAT_TIMEOUT_MS;
	}
}
