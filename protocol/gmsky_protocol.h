// gmsky_protocol.h: the shared-memory layout between the Skyrim plugin and the Garry's Mod module.
//
// This file is the source of truth. protocol/src/lib.rs is a hand-written Rust copy; both files
// assert the same sizes and offsets, so a change to one without the other fails to compile.
// Any layout change must bump GMSKY_VERSION in both files.
//
// Units: Garry's Mod values are Source units and degrees. Skyrim values are Skyrim units and radians.
// All conversion between the two happens on the Skyrim side (see gmsky:: below), in one place.
#pragma once

#include <stddef.h>
#include <stdint.h>

#define GMSKY_SHM_NAME L"Local\\GmodSkyrim_v1"
#define GMSKY_MAP_SIZE 65536u
#define GMSKY_MAGIC 0x594B534Du      // "MSKY": layout initialized and ready
#define GMSKY_MAGIC_INIT 0x54494E49u // "INIT": one side is initializing right now
#define GMSKY_VERSION 1u
#define GMSKY_HEARTBEAT_TIMEOUT_MS 2000u

// GmskyPlayerState.flags
#define GMSKY_PF_ON_GROUND 0x1u
#define GMSKY_PF_CROUCHING 0x2u
#define GMSKY_PF_ALIVE 0x4u
#define GMSKY_PF_NOCLIP 0x8u

typedef struct GmskyHeader {
    uint32_t magic;            // GMSKY_MAGIC once initialized
    uint32_t version;          // GMSKY_VERSION
    uint32_t total_size;       // sizeof(GmskyShared)
    uint32_t reserved0;
    uint32_t skyrim_pid;       // process id, 0 if not connected
    uint32_t gmod_pid;
    uint64_t skyrim_heartbeat; // GetTickCount64() at the last write by that side
    uint64_t gmod_heartbeat;
} GmskyHeader;

// Skyrim -> Garry's Mod. Where Garry's Mod's local frame sits in the Skyrim world.
// A new epoch means "re-anchor": Garry's Mod records its current player position as the anchor.
typedef struct GmskyOrigin {
    uint32_t epoch;              // 0 = no origin yet
    uint32_t worldspace_form_id; // 0 = interior
    double skyrim_origin[3];     // Skyrim units
} GmskyOrigin;

// Skyrim -> Garry's Mod. Skyrim's own view of its player, for logging and checks.
typedef struct GmskySkyrimState {
    uint64_t frame;
    double pos[3];      // Skyrim units, feet
    float heading_rad;  // Skyrim angle.z
    float pitch_rad;    // Skyrim angle.x
    float height;       // player capsule / bound height, Skyrim units
    uint32_t cell_form_id;
    uint32_t flags;
    uint32_t reserved0;
} GmskySkyrimState;

// Garry's Mod -> Skyrim. The authoritative player, in Garry's Mod local units.
typedef struct GmskyPlayerState {
    uint64_t tick;          // Garry's Mod engine tick
    uint32_t origin_epoch;  // the GmskyOrigin.epoch that anchor[] belongs to (0 = none)
    uint32_t flags;         // GMSKY_PF_*
    double anchor[3];       // Garry's Mod position recorded when origin_epoch was first seen
    double pos[3];          // feet position
    float eye_height;       // eye position minus feet, Source units
    float pitch_deg;        // Source: positive looks down
    float yaw_deg;          // Source: 0 = +X, positive turns left (counter-clockwise from above)
    float roll_deg;
    float vel[3];           // Source units per second
    float fov_deg;
} GmskyPlayerState;

// A "latest value" slot guarded by a sequence lock (seqlock):
// the writer makes seq odd, writes, makes seq even. A reader retries if seq was odd or changed.
// seq == 0 means "never written".
#define GMSKY_DEFINE_SLOT(Name, T) \
    typedef struct Name {          \
        uint32_t seq;              \
        uint32_t reserved0;        \
        T data;                    \
    } Name

GMSKY_DEFINE_SLOT(GmskyOriginSlot, GmskyOrigin);
GMSKY_DEFINE_SLOT(GmskySkyrimStateSlot, GmskySkyrimState);
GMSKY_DEFINE_SLOT(GmskyPlayerStateSlot, GmskyPlayerState);

typedef struct GmskyShared {
    GmskyHeader header;
    GmskyOriginSlot origin;            // written by Skyrim
    GmskySkyrimStateSlot skyrim_state; // written by Skyrim
    GmskyPlayerStateSlot player_state; // written by Garry's Mod
} GmskyShared;

// ---- Layout checks. The same numbers are asserted in protocol/src/lib.rs. ----
#ifdef __cplusplus
#define GMSKY_ASSERT(c) static_assert(c, #c)
#else
#define GMSKY_ASSERT(c) _Static_assert(c, #c)
#endif
GMSKY_ASSERT(sizeof(GmskyHeader) == 40);
GMSKY_ASSERT(offsetof(GmskyHeader, skyrim_pid) == 16);
GMSKY_ASSERT(offsetof(GmskyHeader, skyrim_heartbeat) == 24);
GMSKY_ASSERT(offsetof(GmskyHeader, gmod_heartbeat) == 32);
GMSKY_ASSERT(sizeof(GmskyOrigin) == 32);
GMSKY_ASSERT(offsetof(GmskyOrigin, skyrim_origin) == 8);
GMSKY_ASSERT(sizeof(GmskySkyrimState) == 56);
GMSKY_ASSERT(offsetof(GmskySkyrimState, heading_rad) == 32);
GMSKY_ASSERT(offsetof(GmskySkyrimState, cell_form_id) == 44);
GMSKY_ASSERT(sizeof(GmskyPlayerState) == 96);
GMSKY_ASSERT(offsetof(GmskyPlayerState, anchor) == 16);
GMSKY_ASSERT(offsetof(GmskyPlayerState, pos) == 40);
GMSKY_ASSERT(offsetof(GmskyPlayerState, eye_height) == 64);
GMSKY_ASSERT(offsetof(GmskyPlayerState, vel) == 80);
GMSKY_ASSERT(offsetof(GmskyPlayerState, fov_deg) == 92);
GMSKY_ASSERT(offsetof(GmskyOriginSlot, data) == 8);
GMSKY_ASSERT(offsetof(GmskyShared, origin) == 40);
GMSKY_ASSERT(offsetof(GmskyShared, skyrim_state) == 80);
GMSKY_ASSERT(offsetof(GmskyShared, player_state) == 144);
GMSKY_ASSERT(sizeof(GmskyShared) == 248);
GMSKY_ASSERT(sizeof(GmskyShared) <= GMSKY_MAP_SIZE);

#ifdef __cplusplus
#include <atomic>
#include <cmath>

namespace gmsky {

// ---- Seqlock helpers (C++20 std::atomic_ref) ----

template <class Slot, class T>
inline void SlotWrite(Slot& slot, const T& value) {
    std::atomic_ref<uint32_t> seq(slot.seq);
    const uint32_t s = seq.load(std::memory_order_relaxed);
    seq.store(s + 1, std::memory_order_relaxed);
    std::atomic_thread_fence(std::memory_order_release);
    volatile unsigned char* dst = reinterpret_cast<volatile unsigned char*>(&slot.data);
    const unsigned char* src = reinterpret_cast<const unsigned char*>(&value);
    for (size_t i = 0; i < sizeof(T); ++i) dst[i] = src[i];
    seq.store(s + 2, std::memory_order_release);
}

// Returns false if never written or if the writer kept it busy for all attempts.
template <class Slot, class T>
inline bool SlotRead(Slot& slot, T& out, uint32_t* seq_out = nullptr) {
    std::atomic_ref<uint32_t> seq(slot.seq);
    for (int attempt = 0; attempt < 64; ++attempt) {
        const uint32_t s1 = seq.load(std::memory_order_acquire);
        if (s1 == 0) return false;
        if (s1 & 1u) continue;
        T copy;
        const volatile T* src = &slot.data;
        for (size_t i = 0; i < sizeof(T); ++i)
            reinterpret_cast<unsigned char*>(&copy)[i] = reinterpret_cast<const volatile unsigned char*>(src)[i];
        std::atomic_thread_fence(std::memory_order_acquire);
        if (seq.load(std::memory_order_relaxed) == s1) {
            out = copy;
            if (seq_out) *seq_out = s1;
            return true;
        }
    }
    return false;
}

// ---- Coordinate conversion (the only place it happens) ----
//
// Both games are Z-up. Garry's Mod works in a local frame:
//   skyrim = origin + (gmod - anchor) / scale
// kGmodPerSkyrim matches eye heights, so the view sits at the same height in both games:
// Garry's Mod standing eye 64 / Skyrim first-person camera above the feet 120.5 (measured in game
// 2026-10-04, standing, logs/skyrim.log; that character's height was 131.8). Other races/heights
// will differ slightly; making this per-character is a later job.
constexpr double kGmodPerSkyrim = 64.0 / 120.5;
constexpr double kPi = 3.14159265358979323846;

struct Vec3 {
    double x, y, z;
};

inline Vec3 GmodToSkyrim(const double g[3], const double anchor[3], const double origin[3],
                         double scale = kGmodPerSkyrim) {
    return {origin[0] + (g[0] - anchor[0]) / scale, origin[1] + (g[1] - anchor[1]) / scale,
            origin[2] + (g[2] - anchor[2]) / scale};
}

inline Vec3 SkyrimToGmod(const double s[3], const double anchor[3], const double origin[3],
                         double scale = kGmodPerSkyrim) {
    return {anchor[0] + (s[0] - origin[0]) * scale, anchor[1] + (s[1] - origin[1]) * scale,
            anchor[2] + (s[2] - origin[2]) * scale};
}

// Wrap radians into [0, 2*pi).
inline double WrapRad(double r) {
    r = std::fmod(r, 2.0 * kPi);
    return r < 0 ? r + 2.0 * kPi : r;
}

// Skyrim heading: 0 = north (+Y), clockwise seen from above. Source yaw: 0 = +X (east),
// counter-clockwise. So heading = 90deg - yaw.
// Skyrim side VERIFIED in game 2026-10-04: walking straight, heading minus movement direction
// atan2(dx, dy) was 0.0-0.2 deg. Garry's Mod side supported by the first in-game test (straight
// stretches: moving -18.4 / facing -18.6, moving -90.6 / facing -90.9) but not cleanly isolated yet.
inline double GmodYawToSkyrimHeading(double yaw_deg) { return WrapRad((90.0 - yaw_deg) * kPi / 180.0); }

// Both games: positive pitch looks down. VERIFIED in game 2026-10-04: looking straight down gave
// Skyrim 90.0 deg and Garry's Mod 89.0 deg.
inline double GmodPitchToSkyrimPitch(double pitch_deg) { return pitch_deg * kPi / 180.0; }

}  // namespace gmsky
#endif
