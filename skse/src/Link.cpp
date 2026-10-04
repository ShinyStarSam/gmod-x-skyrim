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

	namespace
	{
		uint8_t* RingBase(GmskyShared* a_shared) { return reinterpret_cast<uint8_t*>(a_shared) + GMSKY_RING_OFFSET; }
		std::atomic_ref<uint64_t> RingCounter(GmskyShared* a_shared, uint32_t a_off)
		{
			return std::atomic_ref<uint64_t>(*reinterpret_cast<uint64_t*>(RingBase(a_shared) + a_off));
		}
	}

	bool Link::RingWrite(uint32_t a_type, const void* a_payload, uint32_t a_bytes)
	{
		if (!shared_) {
			return false;
		}
		constexpr uint64_t size = GMSKY_RING_DATA_BYTES;
		const uint64_t msg = (sizeof(GmskyColMsgHeader) + uint64_t(a_bytes) + 7) & ~7ull;
		if (msg > size / 2) {
			logger::error("collision message too large ({} bytes)", msg);
			return false;
		}
		auto headRef = RingCounter(shared_, GMSKY_RING_HEAD_OFF);
		uint64_t head = headRef.load(std::memory_order_relaxed);
		const uint64_t tail = RingCounter(shared_, GMSKY_RING_TAIL_OFF).load(std::memory_order_acquire);
		uint64_t pos = head % size;
		const uint64_t pad = pos + msg > size ? size - pos : 0;
		if (size - (head - tail) < msg + pad) {
			return false;
		}
		uint8_t* data = RingBase(shared_) + GMSKY_RING_DATA_OFF;
		if (pad) {
			const GmskyColMsgHeader h{ GMSKY_COL_PAD, 0 };
			std::memcpy(data + pos, &h, sizeof(h));
			head += pad;
			pos = 0;
		}
		const GmskyColMsgHeader h{ a_type, a_bytes };
		std::memcpy(data + pos, &h, sizeof(h));
		if (a_bytes) {
			std::memcpy(data + pos + sizeof(h), a_payload, a_bytes);
		}
		headRef.store(head + msg, std::memory_order_release);
		return true;
	}

	uint64_t Link::RingPending() const
	{
		if (!shared_) {
			return 0;
		}
		const uint64_t head = RingCounter(shared_, GMSKY_RING_HEAD_OFF).load(std::memory_order_acquire);
		const uint64_t tail = RingCounter(shared_, GMSKY_RING_TAIL_OFF).load(std::memory_order_acquire);
		return head > tail ? head - tail : 0;
	}
}
