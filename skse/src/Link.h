#pragma once

#include "gmsky_protocol.h"

namespace gmsky
{
	// The Skyrim side of the shared memory (protocol/gmsky_protocol.h).
	// Same open/initialize rules as protocol/src/shm.rs: whoever arrives first fills the header.
	class Link
	{
	public:
		static Link& Get();

		// Creates or opens the shared memory and registers Skyrim's process id. Logs why on failure.
		bool Open();
		bool IsOpen() const { return shared_ != nullptr; }
		GmskyShared& Shared() { return *shared_; }

		// Each slot has one writer process. True if this process is Skyrim's registered writer,
		// re-claiming the side if its previous owner exited (pid 0). Same rule as shm.rs owns_side.
		bool OwnsSide();

		void Heartbeat();
		// Garry's Mod wrote a heartbeat within GMSKY_HEARTBEAT_TIMEOUT_MS.
		bool PeerAlive() const;

	private:
		// Logs an open failure, but only when it differs from the previous one (Open is retried).
		bool Fail(std::string a_msg);

		std::string lastError_;
		HANDLE mapping_{ nullptr };
		GmskyShared* shared_{ nullptr };
	};
}
