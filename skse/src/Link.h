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

		void Heartbeat();
		// Garry's Mod wrote a heartbeat within GMSKY_HEARTBEAT_TIMEOUT_MS.
		bool PeerAlive() const;

	private:
		HANDLE mapping_{ nullptr };
		GmskyShared* shared_{ nullptr };
	};
}
