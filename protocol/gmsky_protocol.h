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

// v2: the block grew (the collision ring), and an old process must not open the bigger block.
#define GMSKY_SHM_NAME L"Local\\GmodSkyrim_v2"
#define GMSKY_SLOTS_SIZE 65536u                // the latest-value slots (GmskyShared) live here
#define GMSKY_RING_OFFSET GMSKY_SLOTS_SIZE     // then the collision ring (see "Collision ring")
#define GMSKY_RING_BYTES (16u << 20)
#define GMSKY_MAP_SIZE (GMSKY_RING_OFFSET + GMSKY_RING_BYTES)
#define GMSKY_MAGIC 0x594B534Du      // "MSKY": layout initialized and ready
#define GMSKY_MAGIC_INIT 0x54494E49u // "INIT": one side is initializing right now
#define GMSKY_VERSION 9u  // 2: GmskyInput; 3: applied input in PlayerState; 4: tick stamps; 5: GmskyTerrain; 6: noclip_presses; 7: anchor-relative GmskyTerrain; 8: collision ring; 9: view punch
#define GMSKY_HEARTBEAT_TIMEOUT_MS 2000u

// GmskyPlayerState.flags
#define GMSKY_PF_ON_GROUND 0x1u
#define GMSKY_PF_CROUCHING 0x2u
#define GMSKY_PF_ALIVE 0x4u
#define GMSKY_PF_NOCLIP 0x8u
#define GMSKY_PF_INPUT_APPLIED 0x10u  // input_frame / mouse_*_applied are valid (input from Skyrim is on)
#define GMSKY_PF_DRIVE_CAMERA 0x20u   // Skyrim may turn its camera directly (gmsky_camera 1)
#define GMSKY_PF_SMOOTH 0x40u         // Skyrim may blend between ticks (gmsky_smooth 1)
#define GMSKY_PF_SHAPES 0x80u         // Garry's Mod wants Skyrim's collision shapes (the ring), not the ray grid

// GmskyInput.buttons: actions held in Skyrim's window (not raw keys; the key choice is Skyrim-side)
#define GMSKY_IN_FORWARD 0x1u
#define GMSKY_IN_BACK 0x2u
#define GMSKY_IN_LEFT 0x4u
#define GMSKY_IN_RIGHT 0x8u
#define GMSKY_IN_JUMP 0x10u
#define GMSKY_IN_DUCK 0x20u
#define GMSKY_IN_SPEED 0x40u   // sprint
#define GMSKY_IN_WALK 0x80u
#define GMSKY_IN_ATTACK 0x100u
#define GMSKY_IN_ATTACK2 0x200u
#define GMSKY_IN_RELOAD 0x400u

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
    // Input from Skyrim already turned into this state (valid with GMSKY_PF_INPUT_APPLIED).
    // Skyrim draws yaw/pitch plus the mouse movement not applied yet (DESIGN.md, Phase 1 step 1b).
    uint64_t input_frame;      // GmskyInput.frame of the last input applied
    int64_t mouse_x_applied;   // GmskyInput.mouse_x_total already in yaw_deg
    int64_t mouse_y_applied;   // GmskyInput.mouse_y_total already in pitch_deg
    float deg_per_count;       // degrees per mouse count; 0 = Skyrim must not predict
    uint32_t reserved0;
    // When this tick was written (QueryPerformanceCounter: the same clock in both processes) and
    // the tick length, so Skyrim can blend between ticks (DESIGN.md, Phase 1 step 2).
    int64_t tick_qpc;          // 0 = unknown
    float tick_ms;             // engine.TickInterval() in ms
    uint32_t reserved1;
    // View punch (Source's GetViewPunchAngles: hard landings, later weapon recoil), degrees, added
    // to the camera only (DESIGN.md, Phase 1 step 4). Same signs as pitch/yaw/roll above.
    float punch_deg[3];        // pitch, yaw, roll
    uint32_t reserved2;
} GmskyPlayerState;

// Skyrim -> Garry's Mod. Input typed into Skyrim's window while Garry's Mod drives the player.
// Mouse and wheel are running totals, so a reader that misses a write loses nothing: it applies
// the difference since the totals it saw last.
typedef struct GmskyInput {
    uint64_t frame;          // Skyrim frame of this write
    uint32_t active;         // 1 = route this input to Garry's Mod's player; 0 = ignore it
    uint32_t buttons;        // GMSKY_IN_* held right now
    int64_t mouse_x_total;   // mouse counts since Skyrim started, + = right
    int64_t mouse_y_total;   // + = down
    int32_t wheel_total;     // wheel notches, + = up
    uint32_t noclip_presses; // noclip key (V) presses since Skyrim started; each new one toggles noclip
} GmskyInput;

// Skyrim -> Garry's Mod. Skyrim's ground around the player (Phase 1 step 3): a square grid of
// heights from downward ray casts, in Garry's Mod units *relative to the anchor* for the given
// origin epoch: (skyrim - origin) * scale (all conversion stays on the Skyrim side). Garry's Mod
// adds its current anchor, so a grid stays right when Garry's Mod re-centres (moves the player and
// the anchor together, DESIGN.md re-plan step C) while the grid is on its way.
// Point (i, j) is at x = center[0] + (i - (n-1)/2) * spacing, y = center[1] + (j - (n-1)/2) * spacing;
// index i * n + j.
#define GMSKY_TERRAIN_N 65
typedef struct GmskyTerrain {
    uint32_t id;            // +1 per finished grid; 0 = none yet
    uint32_t origin_epoch;  // the GmskyOrigin.epoch / anchor these Garry's Mod coordinates belong to
    uint32_t n;             // points per side (GMSKY_TERRAIN_N)
    uint32_t misses;        // rays that hit nothing (their height is NaN)
    double center[2];       // Garry's Mod x, y of the middle point, relative to the anchor
    float spacing;          // Garry's Mod units between points
    float reserved0;
    uint64_t skyrim_frame;  // Skyrim frame when the grid was finished
    float height[GMSKY_TERRAIN_N * GMSKY_TERRAIN_N];  // Garry's Mod z of the ground relative to the anchor; NaN = no hit
    uint32_t reserved1;
} GmskyTerrain;

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
GMSKY_DEFINE_SLOT(GmskyInputSlot, GmskyInput);
GMSKY_DEFINE_SLOT(GmskyTerrainSlot, GmskyTerrain);

typedef struct GmskyShared {
    GmskyHeader header;
    GmskyOriginSlot origin;            // written by Skyrim
    GmskySkyrimStateSlot skyrim_state; // written by Skyrim
    GmskyPlayerStateSlot player_state; // written by Garry's Mod
    GmskyInputSlot input;              // written by Skyrim
    GmskyTerrainSlot terrain;          // written by Skyrim
} GmskyShared;

// ---- Collision ring (Skyrim -> Garry's Mod; DESIGN.md re-plan step D; adapted from SkyCraft, MIT) ----
// A byte ring at GMSKY_RING_OFFSET in the mapping: u64 head (total bytes written, by Skyrim) at
// +GMSKY_RING_HEAD_OFF, u64 tail (total bytes read, by Garry's Mod) at +GMSKY_RING_TAIL_OFF, data
// from +GMSKY_RING_DATA_OFF. Every message starts 8-byte aligned with a GmskyColMsgHeader; its
// size is rounded up to 8. GMSKY_COL_PAD means "the rest of the ring is unused: go to its start".
// One writer thread, one reader.
#define GMSKY_RING_HEAD_OFF 0x00u
#define GMSKY_RING_TAIL_OFF 0x40u
#define GMSKY_RING_DATA_OFF 0x80u
#define GMSKY_RING_DATA_BYTES (GMSKY_RING_BYTES - GMSKY_RING_DATA_OFF)

#define GMSKY_COL_PAD 0u     // no payload
#define GMSKY_COL_CLEAR 1u   // payload: GmskyColClear. Drop every region; newer messages are this epoch.
#define GMSKY_COL_REGION 2u  // payload: GmskyColRegion + GmskyColTri[count]. Replaces that region.

typedef struct GmskyColMsgHeader {
    uint32_t type;   // GMSKY_COL_*
    uint32_t bytes;  // payload bytes after this header
} GmskyColMsgHeader;

typedef struct GmskyColClear {
    uint32_t col_epoch;     // +1 on every restart of the stream
    uint32_t origin_epoch;  // the GmskyOrigin.epoch the triangles that follow belong to
} GmskyColClear;

// One cube of Skyrim's world, region_size Skyrim units a side, numbered in Skyrim's coordinates
// (region (rx, ry, rz) starts at Skyrim (rx, ry, rz) * size). lo/size: the same cube in Garry's
// Mod units relative to the anchor. Triangles touching the cube (and a little around it) are sent
// with it, so one triangle can be in several regions.
typedef struct GmskyColRegion {
    int32_t rx, ry, rz;
    uint32_t col_epoch;
    uint32_t count;     // GmskyColTri that follow
    uint32_t flags;     // reserved, 0
    float lo[3];        // Garry's Mod units, relative to the anchor
    float size;         // Garry's Mod units
} GmskyColRegion;

#define GMSKY_TRI_STAIR_HELPER 0x1u  // one of Skyrim's invisible ramps over stairs

typedef struct GmskyColTri {
    float v[9];       // three corners, Garry's Mod units relative to the anchor
    uint32_t flags;   // GMSKY_TRI_*
} GmskyColTri;

// ---- Overlay picture block (Garry's Mod -> Skyrim; DESIGN.md "Step 2b in detail") ----
// A second shared memory block, so the link block above (and GMSKY_VERSION) stay as they are.
// Garry's Mod's finished picture (hands, weapon, HUD on a transparent background) goes into one of
// three slots, a "triple buffer" (adapted from SkyCraft, MIT: skycraft_protocol.h overlay slots,
// Link.cpp AcquireOverlayFrame): the writer always owns one slot to fill, the reader always owns
// one whole slot to read, the third ("middle") holds the newest finished picture. Neither waits.
//
// Unlike SkyCraft, the whole arrangement lives in ONE 32-bit word, `state`, changed only by
// compare-and-swap, so the three slot numbers are always a permutation of 0, 1, 2 even when a
// game restarts or crashes (each side re-attaches to the slot it had, or a free one):
//   bits 0-1  middle slot               bit 2   middle holds a picture the reader hasn't taken
//   bits 4-5  writer's slot             bit 6   writer attached
//   bits 8-9  reader's slot             bit 10  reader attached
// Writer publishes: (middle, writer) = (writer, middle), new-picture bit set.
// Reader takes (only if the bit is set): (middle, reader) = (reader, middle), bit cleared.
// A zero-filled block is valid: middle 0, nobody attached.
#define GMSKY_OV_SHM_NAME L"Local\\GmodSkyrim_overlay_v1"
#define GMSKY_OV_MAGIC 0x564F534Du      // "MSOV"
#define GMSKY_OV_VERSION 1u
#define GMSKY_OV_MAX_W 3840u
#define GMSKY_OV_MAX_H 2160u
#define GMSKY_OV_MASK_MAX_W (GMSKY_OV_MAX_W / 4u)  // plate mask for the planned blur, quarter size
#define GMSKY_OV_MASK_MAX_H (GMSKY_OV_MAX_H / 4u)
#define GMSKY_OV_PIXELS_OFF 0x1000u              // slot 0's pixels; slot i at + i * GMSKY_OV_SLOT_BYTES
#define GMSKY_OV_MASK_OFF (GMSKY_OV_MAX_W * GMSKY_OV_MAX_H * 4u)  // the mask, inside each slot
#define GMSKY_OV_SLOT_BYTES ((GMSKY_OV_MASK_OFF + GMSKY_OV_MASK_MAX_W * GMSKY_OV_MASK_MAX_H + 0xFFFFu) & ~0xFFFFu)
#define GMSKY_OV_MAP_SIZE (GMSKY_OV_PIXELS_OFF + 3u * GMSKY_OV_SLOT_BYTES)

#define GMSKY_OVS_MIDDLE_MASK 0x3u
#define GMSKY_OVS_NEW 0x4u
#define GMSKY_OVS_WRITER_SHIFT 4u
#define GMSKY_OVS_WRITER_ON 0x40u
#define GMSKY_OVS_READER_SHIFT 8u
#define GMSKY_OVS_READER_ON 0x400u

// GmskyOverlaySlotHdr.format
#define GMSKY_OV_FMT_BGRA8 1u  // bytes B, G, R, A per pixel (Direct3D 9 A8R8G8B8 = DXGI B8G8R8A8_UNORM)
// GmskyOverlaySlotHdr.flags
#define GMSKY_OVF_PREMULTIPLIED 0x1u  // colour already multiplied by alpha (drawn over transparent black)
#define GMSKY_OVF_SRGB 0x2u           // colour values are sRGB-encoded (blend in linear light, test 32)

typedef struct GmskyOverlayCtl {
    uint32_t magic;              // GMSKY_OV_MAGIC once initialized (GMSKY_MAGIC_INIT while one side sets it up)
    uint32_t version;            // GMSKY_OV_VERSION
    uint64_t map_size;           // GMSKY_OV_MAP_SIZE
    uint32_t state;              // the triple buffer (see above); compare-and-swap only
    uint32_t reserved0;
    uint64_t frames_published;   // by Garry's Mod
    uint64_t frames_taken;       // by Skyrim
    uint32_t skyrim_width;       // Skyrim's back buffer size (step 2b-4), 0 = not known yet
    uint32_t skyrim_height;
    int64_t skyrim_size_qpc;     // when Skyrim last wrote its size (QueryPerformanceCounter)
} GmskyOverlayCtl;

typedef struct GmskyOverlaySlotHdr {  // written by Garry's Mod into its own slot before publishing
    uint32_t width;
    uint32_t height;
    uint32_t format;          // GMSKY_OV_FMT_*
    uint32_t flags;           // GMSKY_OVF_*
    uint64_t frame;           // Garry's Mod's frame (presents counted by the hook)
    int64_t capture_qpc;      // when the picture was taken (QueryPerformanceCounter), for its age
    uint32_t row_bytes;       // bytes per row in the slot (width * 4, rows top to bottom)
    uint32_t mask_width;      // plate mask (1 byte per pixel at GMSKY_OV_MASK_OFF); 0 = none
    uint32_t mask_height;
    uint32_t reserved0;
    uint8_t reserved[16];
} GmskyOverlaySlotHdr;

#define GMSKY_OV_SLOT_HDR_OFF 0x40u  // three GmskyOverlaySlotHdr

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
GMSKY_ASSERT(sizeof(GmskyPlayerState) == 160);
GMSKY_ASSERT(offsetof(GmskyPlayerState, punch_deg) == 144);
GMSKY_ASSERT(offsetof(GmskyPlayerState, anchor) == 16);
GMSKY_ASSERT(offsetof(GmskyPlayerState, pos) == 40);
GMSKY_ASSERT(offsetof(GmskyPlayerState, eye_height) == 64);
GMSKY_ASSERT(offsetof(GmskyPlayerState, vel) == 80);
GMSKY_ASSERT(offsetof(GmskyPlayerState, fov_deg) == 92);
GMSKY_ASSERT(offsetof(GmskyPlayerState, input_frame) == 96);
GMSKY_ASSERT(offsetof(GmskyPlayerState, deg_per_count) == 120);
GMSKY_ASSERT(offsetof(GmskyPlayerState, tick_qpc) == 128);
GMSKY_ASSERT(offsetof(GmskyPlayerState, tick_ms) == 136);
GMSKY_ASSERT(sizeof(GmskyInput) == 40);
GMSKY_ASSERT(offsetof(GmskyInput, mouse_x_total) == 16);
GMSKY_ASSERT(offsetof(GmskyInput, wheel_total) == 32);
GMSKY_ASSERT(offsetof(GmskyInput, noclip_presses) == 36);
GMSKY_ASSERT(offsetof(GmskyOriginSlot, data) == 8);
GMSKY_ASSERT(offsetof(GmskyShared, origin) == 40);
GMSKY_ASSERT(offsetof(GmskyShared, skyrim_state) == 80);
GMSKY_ASSERT(offsetof(GmskyShared, player_state) == 144);
GMSKY_ASSERT(offsetof(GmskyShared, input) == 312);
GMSKY_ASSERT(sizeof(GmskyTerrain) == 16952);
GMSKY_ASSERT(offsetof(GmskyTerrain, center) == 16);
GMSKY_ASSERT(offsetof(GmskyTerrain, skyrim_frame) == 40);
GMSKY_ASSERT(offsetof(GmskyTerrain, height) == 48);
GMSKY_ASSERT(offsetof(GmskyShared, terrain) == 360);
GMSKY_ASSERT(sizeof(GmskyShared) == 17320);
GMSKY_ASSERT(sizeof(GmskyShared) <= GMSKY_SLOTS_SIZE);
GMSKY_ASSERT(sizeof(GmskyColMsgHeader) == 8);
GMSKY_ASSERT(sizeof(GmskyColClear) == 8);
GMSKY_ASSERT(sizeof(GmskyColRegion) == 40);
GMSKY_ASSERT(offsetof(GmskyColRegion, lo) == 24);
GMSKY_ASSERT(sizeof(GmskyColTri) == 40);
GMSKY_ASSERT(sizeof(GmskyOverlayCtl) == 56);
GMSKY_ASSERT(offsetof(GmskyOverlayCtl, state) == 16);
GMSKY_ASSERT(offsetof(GmskyOverlayCtl, frames_published) == 24);
GMSKY_ASSERT(offsetof(GmskyOverlayCtl, skyrim_width) == 40);
GMSKY_ASSERT(offsetof(GmskyOverlayCtl, skyrim_size_qpc) == 48);
GMSKY_ASSERT(sizeof(GmskyOverlaySlotHdr) == 64);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, frame) == 16);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, capture_qpc) == 24);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, row_bytes) == 32);
GMSKY_ASSERT(sizeof(GmskyOverlayCtl) <= GMSKY_OV_SLOT_HDR_OFF);
GMSKY_ASSERT(GMSKY_OV_SLOT_HDR_OFF + 3 * sizeof(GmskyOverlaySlotHdr) <= GMSKY_OV_PIXELS_OFF);
GMSKY_ASSERT(GMSKY_OV_SLOT_BYTES == 33751040u);
GMSKY_ASSERT(GMSKY_OV_MAP_SIZE == 101257216u);

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

// ---- Overlay triple buffer (see "Overlay picture block"); the same rules as protocol/src/overlay.rs ----
namespace ov {

// Pure rules: old state word -> new state word; *slot = the caller's slot afterwards.
inline uint32_t Attach(uint32_t s, bool writer, uint32_t* slot) {
    const uint32_t shift = writer ? GMSKY_OVS_WRITER_SHIFT : GMSKY_OVS_READER_SHIFT;
    const uint32_t on = writer ? GMSKY_OVS_WRITER_ON : GMSKY_OVS_READER_ON;
    const uint32_t oshift = writer ? GMSKY_OVS_READER_SHIFT : GMSKY_OVS_WRITER_SHIFT;
    const uint32_t oon = writer ? GMSKY_OVS_READER_ON : GMSKY_OVS_WRITER_ON;
    const uint32_t m = s & GMSKY_OVS_MIDDLE_MASK;
    uint32_t own;
    if (s & on) {
        own = (s >> shift) & 3u;  // a restarted side takes back the slot it had
    } else if (s & oon) {
        own = 3u - m - ((s >> oshift) & 3u);  // the one neither the middle nor the other side has
    } else {
        own = (m + (writer ? 1u : 2u)) % 3u;  // nobody else: writer and reader pick different ones
    }
    *slot = own;
    return (s & ~(3u << shift)) | (own << shift) | on;
}

inline uint32_t Publish(uint32_t s, uint32_t* slot) {  // writer: its slot becomes the middle
    const uint32_t w = (s >> GMSKY_OVS_WRITER_SHIFT) & 3u, m = s & GMSKY_OVS_MIDDLE_MASK;
    *slot = m;
    return (s & ~(GMSKY_OVS_MIDDLE_MASK | (3u << GMSKY_OVS_WRITER_SHIFT))) | w | (m << GMSKY_OVS_WRITER_SHIFT) | GMSKY_OVS_NEW;
}

inline bool Take(uint32_t s, uint32_t* next, uint32_t* slot) {  // reader: the middle becomes its slot
    if (!(s & GMSKY_OVS_NEW)) return false;
    const uint32_t r = (s >> GMSKY_OVS_READER_SHIFT) & 3u, m = s & GMSKY_OVS_MIDDLE_MASK;
    *slot = m;
    *next = (s & ~(GMSKY_OVS_MIDDLE_MASK | GMSKY_OVS_NEW | (3u << GMSKY_OVS_READER_SHIFT))) | r | (m << GMSKY_OVS_READER_SHIFT);
    return true;
}

inline uint32_t Detach(uint32_t s, bool writer) {
    return s & ~(writer ? GMSKY_OVS_WRITER_ON : GMSKY_OVS_READER_ON);
}

// The same, applied to the shared word with compare-and-swap.
inline uint32_t AttachShared(GmskyOverlayCtl& c, bool writer) {
    std::atomic_ref<uint32_t> st(c.state);
    uint32_t s = st.load(std::memory_order_acquire), slot = 0;
    while (!st.compare_exchange_weak(s, Attach(s, writer, &slot), std::memory_order_acq_rel, std::memory_order_acquire)) {
    }
    return slot;
}

// Reader: if there is a new picture, makes it the reader's slot (returned in *slot) and true.
inline bool TakeShared(GmskyOverlayCtl& c, uint32_t* slot) {
    std::atomic_ref<uint32_t> st(c.state);
    uint32_t s = st.load(std::memory_order_acquire), next = 0, got = 0;
    do {
        if (!Take(s, &next, &got)) return false;
    } while (!st.compare_exchange_weak(s, next, std::memory_order_acq_rel, std::memory_order_acquire));
    *slot = got;
    std::atomic_ref<uint64_t>(c.frames_taken).fetch_add(1, std::memory_order_relaxed);
    return true;
}

inline void DetachShared(GmskyOverlayCtl& c, bool writer) {
    std::atomic_ref<uint32_t> st(c.state);
    uint32_t s = st.load(std::memory_order_acquire);
    while (!st.compare_exchange_weak(s, Detach(s, writer), std::memory_order_acq_rel, std::memory_order_acquire)) {
    }
}

}  // namespace ov

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
// VERIFIED in game 2026-10-04 on both sides. Skyrim: walking straight, heading minus movement
// direction atan2(dx, dy) was 0.0-0.2 deg. Garry's Mod: walking straight, yaw minus atan2(vy, vx)
// was 0.0 in 11 of 11 samples, and strafing gave +-90 / diagonals +-45, +-135 as predicted.
inline double GmodYawToSkyrimHeading(double yaw_deg) { return WrapRad((90.0 - yaw_deg) * kPi / 180.0); }

// Both games: positive pitch looks down. VERIFIED in game 2026-10-04: looking straight down gave
// Skyrim 90.0 deg and Garry's Mod 89.0 deg.
inline double GmodPitchToSkyrimPitch(double pitch_deg) { return pitch_deg * kPi / 180.0; }

}  // namespace gmsky
#endif
