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
#define GMSKY_VERSION 16u  // 2: GmskyInput; 3: applied input in PlayerState; 4: tick stamps; 5: GmskyTerrain; 6: noclip_presses; 7: anchor-relative GmskyTerrain; 8: collision ring; 9: view punch; 10: GmskyActors; 11: GmskyHitboxes; 12: hit test; 13: hit events; 14: hurt events; 15: water_z; 16: radius
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
#define GMSKY_PF_CURSOR 0x100u        // a Garry's Mod menu cursor is up: the mouse doesn't turn the view, Skyrim must not predict (step 3a)

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
#define GMSKY_IN_MENU 0x800u        // Q: Sandbox's spawn menu, +menu while held (step 3a)
#define GMSKY_IN_CONSOLE 0x1000u    // F1 held: open Garry's Mod's console in its own window (after test 62)
#define GMSKY_IN_SLOT_SHIFT 16u     // bits 16-19: weapon slot key held, 1-6 (0 = none) (step 3a)
#define GMSKY_IN_SLOT_MASK 0xF0000u

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
    uint32_t flags;     // GMSKY_SF_*
    float water_z;      // water surface where the player is, Garry's Mod units relative to the anchor (if GMSKY_SF_WATER; v15)
    float radius;       // the player's collision capsule radius, Garry's Mod units (0 = unknown; v16, S2j)
    uint32_t reserved0;
} GmskySkyrimState;
#define GMSKY_SF_IN_COMBAT 0x1u  // someone is fighting the player (step 3f: no calm spot now)
#define GMSKY_SF_WATER 0x2u      // water_z is valid: there is water here (step 4b)

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

// Skyrim -> Garry's Mod (Phase 3 step 3b; adapted from SkyCraft's ActorTable, MIT). Skyrim's
// actors near the player, nearest first, written every Skyrim frame. Positions in Garry's Mod units
// *relative to the anchor* for origin_epoch, like GmskyTerrain (Garry's Mod adds its anchor).
#define GMSKY_MAX_ACTORS 64
#define GMSKY_ACT_HOSTILE 0x1u    // hostile to the player right now
#define GMSKY_ACT_DEAD 0x2u
#define GMSKY_ACT_ESSENTIAL 0x4u  // can't be killed (Skyrim knocks them down instead)
#define GMSKY_ACT_IN_COMBAT 0x8u
typedef struct GmskyActor {
    uint32_t form_id;      // Skyrim's FormID of the actor (reference)
    uint32_t flags;        // GMSKY_ACT_*
    float pos[3];          // feet, Garry's Mod units relative to the anchor
    float yaw_deg;         // Source convention (0 = +X, counter-clockwise)
    float radius;          // Garry's Mod units (Skyrim's bound radius, scaled)
    float height;          // Garry's Mod units
    float health_frac;     // 0..1
    uint16_t level;
    uint16_t reserved0;
    char name[32];         // display name, UTF-8, NUL-terminated (truncated)
} GmskyActor;

typedef struct GmskyActors {
    uint32_t count;         // valid entries in actors[]
    uint32_t origin_epoch;  // the GmskyOrigin.epoch these positions belong to (0 = none)
    uint64_t skyrim_frame;
    GmskyActor actors[GMSKY_MAX_ACTORS];
} GmskyActors;

// Skyrim -> Garry's Mod (Phase 3 step 3c-2, DESIGN.md "Hitboxes"). The per-bone body shapes of
// the actors near the player (the ones Skyrim's arrows hit and the ragdoll is made of), nearest
// first, living and dead, written every Skyrim frame. Garry's Mod units relative to the anchor.
#define GMSKY_HB_MAX_ACTORS 24
#define GMSKY_HB_MAX_PARTS 512
#define GMSKY_HBK_CAPSULE 1u  // a, b = the two end points, r = radius (a sphere: a == b). Exact.
#define GMSKY_HBK_BOX 2u      // a = min corner, b = max corner (world-aligned box around the shape)
#define GMSKY_HBF_APPROX 0x1u // the shape isn't a capsule: its box is sent instead
#define GMSKY_HBF_GEAR 0x2u   // on a bone that isn't body (a carried bow, weapon, shield, quiver)
// GmskyHitPart.group: Garry's Mod's own hit groups (HITGROUP_*)
#define GMSKY_HBG_GENERIC 0u
#define GMSKY_HBG_HEAD 1u
#define GMSKY_HBG_CHEST 2u
#define GMSKY_HBG_STOMACH 3u
#define GMSKY_HBG_LEFTARM 4u
#define GMSKY_HBG_RIGHTARM 5u
#define GMSKY_HBG_LEFTLEG 6u
#define GMSKY_HBG_RIGHTLEG 7u
#define GMSKY_HBG_GEAR 10u
// GmskyHitActor.flags
#define GMSKY_HBA_DEAD 0x1u
#define GMSKY_HBA_IN_WORLD 0x2u  // its parts are in Skyrim's physics world (test 56: corpses, some NPCs)

typedef struct GmskyHitPart {
    float a[3];
    float b[3];
    float r;
    uint8_t kind;   // GMSKY_HBK_*
    uint8_t group;  // GMSKY_HBG_*
    uint8_t flags;  // GMSKY_HBF_*
    uint8_t bone;   // index of the part in the actor's collision walk (for logs and 3d)
} GmskyHitPart;

typedef struct GmskyHitActor {
    uint32_t form_id;
    uint16_t first_part;  // index into GmskyHitboxes.parts
    uint16_t part_count;
    uint32_t flags;       // GMSKY_HBA_*
    uint32_t reserved0;
} GmskyHitActor;

typedef struct GmskyHitboxes {
    uint32_t actor_count;
    uint32_t part_count;
    uint32_t origin_epoch;  // the GmskyOrigin.epoch these positions belong to (0 = none)
    uint32_t reserved0;
    uint64_t skyrim_frame;
    int64_t qpc;            // when written (QueryPerformanceCounter), for the age in Garry's Mod
    GmskyHitActor actors[GMSKY_HB_MAX_ACTORS];
    GmskyHitPart parts[GMSKY_HB_MAX_PARTS];
} GmskyHitboxes;

// Hitbox test (Phase 3 step 3c-3, DESIGN.md "3c-3 in detail", gmsky_hitbox_test): Garry's Mod
// sends rays over one NPC, Skyrim answers each against its current parts and with its own physics.
#define GMSKY_HT_MAX_RAYS 441  // 21 x 21
typedef struct GmskyHitTestRay {
    float o[3];  // start, Garry's Mod units relative to the anchor
    float d[3];  // the whole ray (end = o + d)
} GmskyHitTestRay;

typedef struct GmskyHitTestRequest {  // Garry's Mod -> Skyrim
    uint32_t id;            // +1 per test; Skyrim answers each new id once
    uint32_t form_id;       // the NPC
    uint32_t count;         // rays
    uint32_t origin_epoch;
    GmskyHitTestRay rays[GMSKY_HT_MAX_RAYS];
} GmskyHitTestRequest;

// GmskyHitTestAnswer.havok: what Skyrim's own physics ray hit first
#define GMSKY_HTH_NOTHING 0u
#define GMSKY_HTH_THIS_NPC 1u   // a collision body of this NPC (one of its parts, or its character controller)
#define GMSKY_HTH_OTHER 2u      // something else (the world, another actor)
#define GMSKY_HTH_FAILED 3u     // couldn't cast (no physics world)
typedef struct GmskyHitTestAnswer {
    float now_t;        // fraction along the ray where Skyrim's current parts are hit; < 0 = missed
    float havok_t;      // fraction where Skyrim's physics ray hit (if havok != NOTHING)
    uint8_t now_group;  // GMSKY_HBG_* of the part hit
    uint8_t havok;      // GMSKY_HTH_*
    uint8_t havok_layer;  // collision layer of what it hit
    uint8_t reserved0;
} GmskyHitTestAnswer;

typedef struct GmskyHitTestResult {  // Skyrim -> Garry's Mod
    uint32_t id;            // the request answered
    uint32_t form_id;
    uint32_t count;
    uint32_t flags;         // GMSKY_HBA_* of the NPC at that moment (dead, parts in the physics world)
    uint64_t skyrim_frame;
    uint32_t parts;         // parts read for it
    uint32_t reserved0;
    GmskyHitTestAnswer answers[GMSKY_HT_MAX_RAYS];
} GmskyHitTestResult;

// Hits on Skyrim's NPCs, Garry's Mod -> Skyrim (Phase 3 step 3d, DESIGN.md "3d in detail"). Not a
// latest-value slot but a ring of events: Garry's Mod writes ev[head % GMSKY_EV_MAX], then adds 1
// to head (release). Skyrim keeps its own read position, starting at head when it first reads it
// (so old hits never land). head only counts up: the block lives as long as either game has it
// open, so a restarted game continues from it. More than GMSKY_EV_MAX events between two reads
// are lost (and counted by Skyrim).
#define GMSKY_EV_MAX 64
#define GMSKY_HITK_OTHER 0u
#define GMSKY_HITK_BULLET 1u
#define GMSKY_HITK_MELEE 2u
#define GMSKY_HITK_EXPLOSION 3u
typedef struct GmskyHitEvent {
    uint32_t form_id;
    float damage;          // Skyrim damage (already scaled)
    float gmod_damage;     // what Garry's Mod's weapon did (for the logs)
    float pos[3];          // where it landed, Garry's Mod units relative to the anchor (explosions: the blast)
    float dir[3];          // unit direction of the hit
    float force;           // Garry's Mod damage force length
    uint32_t dmg_type;     // Garry's Mod DMG_* bits
    uint32_t origin_epoch; // the origin pos[] belongs to
    uint8_t group;         // GMSKY_HBG_* body group hit
    uint8_t bone;          // index of the part (GmskyHitPart.bone), 255 = none
    uint8_t kind;          // GMSKY_HITK_*
    uint8_t flags;
} GmskyHitEvent;

typedef struct GmskyHitEvents {
    uint32_t head;      // events written so far (Garry's Mod); atomic
    uint32_t session;   // process id of the Garry's Mod that wrote last (for logs)
    GmskyHitEvent ev[GMSKY_EV_MAX];
} GmskyHitEvents;

// ---- Hurt events (Skyrim -> Garry's Mod; Phase 3 step 3e) ----
// Skyrim's NPCs hurting the player. A ring like GmskyHitEvents, the other way: Skyrim writes
// ev[head % GMSKY_HURT_MAX], then adds 1 to head (release); Garry's Mod keeps its own read
// position (starting at head, so old hurts never land).
#define GMSKY_HURT_MAX 32
#define GMSKY_HURTK_MELEE 0u
#define GMSKY_HURTK_PROJECTILE 1u
#define GMSKY_HURTK_MAGIC 2u
#define GMSKY_HURTK_OTHER 3u
#define GMSKY_HURTF_BLOCKED 0x1u  // blocked in Skyrim (the damage is already reduced)
#define GMSKY_HURTF_POWER 0x2u    // a power attack
typedef struct GmskyHurtEvent {
    uint32_t attacker;  // FormID of the actor who did it, 0 = none / unknown
    float damage;       // Skyrim damage (the health Skyrim took off; armour already applied)
    uint32_t source;    // FormID of the weapon / spell, 0 = unknown
    uint8_t kind;       // GMSKY_HURTK_*
    uint8_t flags;      // GMSKY_HURTF_*
    uint16_t reserved;
} GmskyHurtEvent;

typedef struct GmskyHurtEvents {
    uint32_t head;      // events written so far (Skyrim); atomic
    uint32_t session;   // process id of the Skyrim that wrote last (for logs)
    GmskyHurtEvent ev[GMSKY_HURT_MAX];
} GmskyHurtEvents;

GMSKY_DEFINE_SLOT(GmskyOriginSlot, GmskyOrigin);
GMSKY_DEFINE_SLOT(GmskySkyrimStateSlot, GmskySkyrimState);
GMSKY_DEFINE_SLOT(GmskyPlayerStateSlot, GmskyPlayerState);
GMSKY_DEFINE_SLOT(GmskyInputSlot, GmskyInput);
GMSKY_DEFINE_SLOT(GmskyTerrainSlot, GmskyTerrain);
GMSKY_DEFINE_SLOT(GmskyActorsSlot, GmskyActors);
GMSKY_DEFINE_SLOT(GmskyHitboxesSlot, GmskyHitboxes);
GMSKY_DEFINE_SLOT(GmskyHitTestRequestSlot, GmskyHitTestRequest);
GMSKY_DEFINE_SLOT(GmskyHitTestResultSlot, GmskyHitTestResult);

typedef struct GmskyShared {
    GmskyHeader header;
    GmskyOriginSlot origin;            // written by Skyrim
    GmskySkyrimStateSlot skyrim_state; // written by Skyrim
    GmskyPlayerStateSlot player_state; // written by Garry's Mod
    GmskyInputSlot input;              // written by Skyrim
    GmskyTerrainSlot terrain;          // written by Skyrim
    GmskyActorsSlot actors;            // written by Skyrim (v10)
    GmskyHitboxesSlot hitboxes;        // written by Skyrim (v11)
    GmskyHitTestRequestSlot hit_test_request;  // written by Garry's Mod (v12)
    GmskyHitTestResultSlot hit_test_result;    // written by Skyrim (v12)
    GmskyHitEvents hit_events;                 // written by Garry's Mod, read by Skyrim (v13)
    GmskyHurtEvents hurt_events;               // written by Skyrim, read by Garry's Mod (v14)
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
#define GMSKY_TRI_STAIRS 0x2u        // S2i-1: the triangle's Skyrim material is a stair material

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
#define GMSKY_OV_VERSION 3u  // 2: slot header 128 bytes with the picture's camera (step W4); 3: world layer removed (G8)
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
#define GMSKY_OVF_BLUR 0x4u           // Skyrim blurs its picture behind see-through parts (gmsky_overlay_blur)
// (0x8 and 0x10 were the world layer and its depth, steps W2-W4; removed in G8.)

typedef struct GmskyOverlayCtl {
    uint32_t magic;              // GMSKY_OV_MAGIC once initialized (GMSKY_MAGIC_INIT while one side sets it up)
    uint32_t version;            // GMSKY_OV_VERSION
    uint64_t map_size;           // GMSKY_OV_MAP_SIZE
    uint32_t state;              // the triple buffer (see above); compare-and-swap only
    uint32_t shot_request;       // Garry's Mod adds 1 (gmsky_overlay_shot): Skyrim saves its next drawn frame (step 2c-1)
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
    uint8_t reserved[84];     // (the world layer's depth size and camera until G8)
} GmskyOverlaySlotHdr;

#define GMSKY_OV_SLOT_HDR_OFF 0x40u  // three GmskyOverlaySlotHdr

// ---- Render block (Garry's Mod -> Skyrim; geometry route step G2, DESIGN.md §12) ----
// Garry's Mod's models, textures and, every frame, where its objects are and how their bones are
// posed; Skyrim draws them inside its own frame. A third shared memory block, so the link block
// and GMSKY_VERSION stay as they are. A byte ring (as SkyCraft's render ring, MIT, and our
// collision ring): u64 head (bytes written by Garry's Mod) at GMSKY_RND_HEAD_OFF, u64 tail (bytes
// read by Skyrim) at GMSKY_RND_TAIL_OFF, data from GMSKY_RND_DATA_OFF. Every message starts 8-byte
// aligned with a GmskyColMsgHeader {type, bytes}, its size rounded up to 8; GMSKY_RND_PAD = "the
// rest of the ring is unused: go to its start". A message is at most half the ring.
// Restarts: Garry's Mod's stream starts with RESET. A Skyrim that attaches skips everything unread
// (tail = head) and adds 1 to reader_epoch; Garry's Mod then sends RESET and everything again.
#define GMSKY_RND_SHM_NAME L"Local\\GmodSkyrim_render_v1"
#define GMSKY_RND_MAGIC 0x444E524Du  // "MRND"
#define GMSKY_RND_VERSION 6u  // 2: the frame carries Garry's Mod's view; objects their entity and flags (G5a + G6); 3: tangents, normal maps, phong (G5b bundle); 4: phong Fresnel, tint, exponent texture; 5: objects carry feet and size (contact shadows); 6: self-illumination tint and mask
#define GMSKY_RND_RING_BYTES (64u << 20)
#define GMSKY_RND_HEAD_OFF 0x1000u
#define GMSKY_RND_TAIL_OFF 0x1040u
#define GMSKY_RND_DATA_OFF 0x1080u
#define GMSKY_RND_MAP_SIZE (GMSKY_RND_DATA_OFF + GMSKY_RND_RING_BYTES)

#define GMSKY_RND_PAD 0u
#define GMSKY_RND_RESET 1u    // GmskyRndReset: drop every model and texture
#define GMSKY_RND_TEXTURE 2u  // GmskyRndTexture + pixels (bytes)
#define GMSKY_RND_MODEL 3u    // GmskyRndModel + GmskyRndPart[part_count] + GmskyRndVertex[vertex_count] + uint32_t[index_count]
#define GMSKY_RND_FORGET 4u   // GmskyRndForget: a model or texture Skyrim may free
#define GMSKY_RND_FRAME 5u    // GmskyRndFrame + GmskyRndObject[object_count] + GmskyRndMatrix[matrix_count]

// GmskyRndTexture.format: one mip level, top row first
#define GMSKY_RND_TEX_BC1 1u    // Source DXT1: 8 bytes per 4x4 block
#define GMSKY_RND_TEX_BC2 2u    // DXT3: 16 bytes per block
#define GMSKY_RND_TEX_BC3 3u    // DXT5: 16 bytes per block
#define GMSKY_RND_TEX_RGBA8 4u  // bytes R, G, B, A per pixel

// GmskyRndPart.flags (from the material)
#define GMSKY_RND_MAT_TRANSLUCENT 0x1u
#define GMSKY_RND_MAT_ADDITIVE 0x2u
#define GMSKY_RND_MAT_ALPHATEST 0x4u
#define GMSKY_RND_MAT_SELFILLUM 0x8u
#define GMSKY_RND_MAT_NOCULL 0x10u
#define GMSKY_RND_MAT_PHONG 0x20u                // phong shine (exponent, boost in the part)
#define GMSKY_RND_MAT_BASEALPHA_PHONGMASK 0x40u  // the shine's mask is the base texture's alpha (else the normal map's)
#define GMSKY_RND_MAT_ALBEDOTINT 0x80u           // $phongalbedotint: the shine takes the base colour (by the exponent texture's green)

// GmskyRndTexture.flags
#define GMSKY_RND_TEXF_LINEAR 0x1u  // not colour: read as is, not as sRGB (normal maps)

// GmskyRndFrame.flags
#define GMSKY_RND_FRAME_VIEW 0x1u         // view_eye / view_angles are valid
// Switches from Garry's Mod's console, to compare in game (G5b bundle):
#define GMSKY_RND_FRAME_FLIP_GREEN 0x2u   // normal maps' green channel points the other way (gmsky_render_normal_flip 1)
#define GMSKY_RND_FRAME_NO_NORMALS 0x4u   // normal maps off (gmsky_render_normals 0)
#define GMSKY_RND_FRAME_NO_PHONG 0x8u     // phong shine off (gmsky_render_phong 0)
#define GMSKY_RND_FRAME_NO_SHADOWS 0x10u  // shadows off (gmsky_render_shadows 0)
#define GMSKY_RND_FRAME_LINEAR_ALBEDO 0x20u  // colour textures converted sRGB -> linear (the old way; gmsky_render_linear_albedo 1)
#define GMSKY_RND_FRAME_SHADOW_DEBUG 0x40u   // our objects coloured by their sun shadow (gmsky_render_shadow_debug 1; step N1)
// GmskyRndObject.flags
#define GMSKY_RND_OBJ_HELD 0x1u    // held by the player (physgun, gravity gun, +use): placed relative to the camera
#define GMSKY_RND_OBJ_CONTACT 0x2u // an NPC, nextbot or player: feet[] and width are set (Skyrim's NPC sun-shadow probe)

#define GMSKY_RND_FORGET_MODEL 1u
#define GMSKY_RND_FORGET_TEXTURE 2u

typedef struct GmskyRenderCtl {
    uint32_t magic;            // GMSKY_RND_MAGIC once initialized (GMSKY_MAGIC_INIT while one side sets it up)
    uint32_t version;          // GMSKY_RND_VERSION
    uint64_t map_size;         // GMSKY_RND_MAP_SIZE
    uint32_t writer_session;   // Garry's Mod's stream (its process id); 0 = no writer yet
    uint32_t reader_epoch;     // +1 each time a Skyrim attaches
    uint64_t messages_written;
    uint64_t messages_read;
    uint64_t writes_refused;   // ring too full (Garry's Mod drops a frame, retries a model later)
} GmskyRenderCtl;

typedef struct GmskyRndReset {
    uint32_t session;          // = writer_session
    uint32_t reader_epoch;     // the reader_epoch this RESET answers
} GmskyRndReset;

typedef struct GmskyRndTexture {
    uint32_t id;               // 1+, referenced by GmskyRndPart.texture_id
    uint32_t width, height;
    uint32_t format;           // GMSKY_RND_TEX_*
    uint32_t bytes;            // pixel bytes that follow (exactly what the format and size need)
    uint32_t flags;            // GMSKY_RND_TEXF_*
} GmskyRndTexture;

typedef struct GmskyRndModel {
    uint32_t id;               // 1+, referenced by GmskyRndObject.model_id
    uint32_t vertex_count;
    uint32_t index_count;      // whole triangles
    uint32_t part_count;
    uint32_t bone_count;       // an object of this model sends this many matrices
    uint32_t flags;            // reserved, 0
} GmskyRndModel;

typedef struct GmskyRndPart {  // triangles drawn with one material
    uint32_t first_index;
    uint32_t index_count;
    uint32_t texture_id;       // 0 = none (white)
    uint32_t flags;            // GMSKY_RND_MAT_*
    float alpha_ref;           // GMSKY_RND_MAT_ALPHATEST: pixels below this alpha are dropped
    uint32_t normal_texture_id;  // the part's $bumpmap (a GMSKY_RND_TEXF_LINEAR texture); 0 = none
    float phong_exponent;      // GMSKY_RND_MAT_PHONG: $phongexponent
    float phong_boost;         // GMSKY_RND_MAT_PHONG: $phongboost
    float phong_fresnel[3];    // $phongfresnelranges: min (head-on), mid, max (grazing)
    float phong_tint[3];       // $phongtint (white if none)
    uint32_t phong_exponent_texture_id;  // $phongexponenttexture (linear; exponent = 1 + 149 x red); 0 = none
    uint32_t reserved;
    float selfillum_tint[3];   // GMSKY_RND_MAT_SELFILLUM: $selfillumtint (white if none)
    uint32_t selfillum_mask_texture_id;  // $selfillummask (linear); 0 = the base texture's alpha is the mask
} GmskyRndPart;

typedef struct GmskyRndVertex {
    float pos[3];              // model space (rest pose), Garry's Mod units
    float normal[3];
    float tangent[4];          // xyz model space, w handedness (+1 / -1): binormal = w x (normal x tangent)
    float u, v;
    uint8_t bones[4];          // bone numbers (< bone_count)
    uint8_t weights[4];        // 0-255, summing to 255
} GmskyRndVertex;

typedef struct GmskyRndForget {
    uint32_t kind;             // GMSKY_RND_FORGET_*
    uint32_t id;
} GmskyRndForget;

typedef struct GmskyRndFrame {
    uint64_t frame;            // Garry's Mod's frame number
    int64_t qpc;               // QueryPerformanceCounter when the poses were taken
    uint32_t origin_epoch;     // the anchor the matrices are relative to (GmskyPlayerState.origin_epoch)
    uint32_t object_count;
    uint32_t matrix_count;
    uint32_t flags;            // GMSKY_RND_FRAME_*
    float view_eye[3];         // Garry's Mod's camera for this frame, relative to the anchor (Garry's Mod units)
    float view_angles[3];      // its pitch, yaw, roll (degrees, Source's angles)
} GmskyRndFrame;

typedef struct GmskyRndObject {
    uint32_t model_id;
    uint32_t first_matrix;
    uint32_t matrix_count;     // = the model's bone_count
    uint32_t color;            // RGBA8 tint (ent:GetColor()), R in the low byte
    uint32_t flags;            // GMSKY_RND_OBJ_*
    uint32_t entity;           // Garry's Mod's entity number (stable while it exists; for motion vectors)
    float feet[3];             // GMSKY_RND_OBJ_CONTACT: its feet (GetPos), relative to the anchor (Garry's Mod units)
    float width;               // GMSKY_RND_OBJ_CONTACT: its collision box width (Garry's Mod units)
    float height;              // GMSKY_RND_OBJ_CONTACT: its collision box height (Garry's Mod units)
    uint32_t reserved;
} GmskyRndObject;

// Rows of [rotation | translation]: a model-space point -> Garry's Mod units relative to the anchor.
// Bone b: ent:GetBoneMatrix(b) x the model's inverse bind pose ("poseToBone"). A vertex is drawn at
// sum(weight x matrix[bone] x pos), then GmodToSkyrim (no axis swap).
typedef struct GmskyRndMatrix {
    float m[3][4];
} GmskyRndMatrix;

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
GMSKY_ASSERT(sizeof(GmskySkyrimState) == 64);
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
GMSKY_ASSERT(offsetof(GmskyShared, player_state) == 152);
GMSKY_ASSERT(offsetof(GmskyShared, input) == 320);
GMSKY_ASSERT(sizeof(GmskyTerrain) == 16952);
GMSKY_ASSERT(offsetof(GmskyTerrain, center) == 16);
GMSKY_ASSERT(offsetof(GmskyTerrain, skyrim_frame) == 40);
GMSKY_ASSERT(offsetof(GmskyTerrain, height) == 48);
GMSKY_ASSERT(offsetof(GmskyShared, terrain) == 368);
GMSKY_ASSERT(sizeof(GmskyActor) == 72);
GMSKY_ASSERT(offsetof(GmskyActor, yaw_deg) == 20);
GMSKY_ASSERT(offsetof(GmskyActor, level) == 36);
GMSKY_ASSERT(offsetof(GmskyActor, name) == 40);
GMSKY_ASSERT(sizeof(GmskyActors) == 16 + 72 * GMSKY_MAX_ACTORS);
GMSKY_ASSERT(offsetof(GmskyShared, actors) == 17328);
GMSKY_ASSERT(sizeof(GmskyHitPart) == 32);
GMSKY_ASSERT(offsetof(GmskyHitPart, kind) == 28);
GMSKY_ASSERT(sizeof(GmskyHitActor) == 16);
GMSKY_ASSERT(offsetof(GmskyHitboxes, actors) == 32);
GMSKY_ASSERT(sizeof(GmskyHitboxes) == 32 + 16 * GMSKY_HB_MAX_ACTORS + 32 * GMSKY_HB_MAX_PARTS);
GMSKY_ASSERT(offsetof(GmskyShared, hitboxes) == 21960);
GMSKY_ASSERT(sizeof(GmskyHitTestRay) == 24);
GMSKY_ASSERT(sizeof(GmskyHitTestRequest) == 16 + 24 * GMSKY_HT_MAX_RAYS);
GMSKY_ASSERT(sizeof(GmskyHitTestAnswer) == 12);
GMSKY_ASSERT(offsetof(GmskyHitTestResult, answers) == 32);
GMSKY_ASSERT(sizeof(GmskyHitTestResult) == 32 + 12 * GMSKY_HT_MAX_RAYS + 4);
GMSKY_ASSERT(offsetof(GmskyShared, hitboxes) + 8 + sizeof(GmskyHitboxes) == offsetof(GmskyShared, hit_test_request));
GMSKY_ASSERT(offsetof(GmskyShared, hit_test_request) == 38768);
GMSKY_ASSERT(offsetof(GmskyShared, hit_test_result) == 38768 + 8 + 10600);
GMSKY_ASSERT(sizeof(GmskyHitEvent) == 52);
GMSKY_ASSERT(offsetof(GmskyHitEvent, dmg_type) == 40);
GMSKY_ASSERT(sizeof(GmskyHitEvents) == 8 + 52 * GMSKY_EV_MAX);
GMSKY_ASSERT(offsetof(GmskyShared, hit_events) == 54712);
GMSKY_ASSERT(sizeof(GmskyHurtEvent) == 16);
GMSKY_ASSERT(sizeof(GmskyHurtEvents) == 8 + 16 * GMSKY_HURT_MAX);
GMSKY_ASSERT(offsetof(GmskyShared, hurt_events) == 54712 + 3336);
GMSKY_ASSERT(sizeof(GmskyShared) == 54712 + 3336 + 520);
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
GMSKY_ASSERT(sizeof(GmskyOverlaySlotHdr) == 128);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, frame) == 16);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, capture_qpc) == 24);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, row_bytes) == 32);
GMSKY_ASSERT(offsetof(GmskyOverlaySlotHdr, reserved) == 44);
GMSKY_ASSERT(sizeof(GmskyOverlayCtl) <= GMSKY_OV_SLOT_HDR_OFF);
GMSKY_ASSERT(GMSKY_OV_SLOT_HDR_OFF + 3 * sizeof(GmskyOverlaySlotHdr) <= GMSKY_OV_PIXELS_OFF);
GMSKY_ASSERT(GMSKY_OV_SLOT_BYTES == 33751040u);
GMSKY_ASSERT(GMSKY_OV_MAP_SIZE == 101257216u);
GMSKY_ASSERT(sizeof(GmskyRenderCtl) == 48);
GMSKY_ASSERT(offsetof(GmskyRenderCtl, writer_session) == 16);
GMSKY_ASSERT(offsetof(GmskyRenderCtl, messages_written) == 24);
GMSKY_ASSERT(offsetof(GmskyRenderCtl, writes_refused) == 40);
GMSKY_ASSERT(sizeof(GmskyRndReset) == 8);
GMSKY_ASSERT(sizeof(GmskyRndTexture) == 24);
GMSKY_ASSERT(sizeof(GmskyRndModel) == 24);
GMSKY_ASSERT(sizeof(GmskyRndPart) == 80);
GMSKY_ASSERT(offsetof(GmskyRndPart, selfillum_tint) == 64);
GMSKY_ASSERT(offsetof(GmskyRndPart, phong_fresnel) == 32);
GMSKY_ASSERT(sizeof(GmskyRndVertex) == 56);
GMSKY_ASSERT(offsetof(GmskyRndVertex, u) == 40);
GMSKY_ASSERT(offsetof(GmskyRndVertex, bones) == 48);
GMSKY_ASSERT(sizeof(GmskyRndForget) == 8);
GMSKY_ASSERT(sizeof(GmskyRndFrame) == 56);
GMSKY_ASSERT(offsetof(GmskyRndFrame, view_eye) == 32);
GMSKY_ASSERT(offsetof(GmskyRndFrame, origin_epoch) == 16);
GMSKY_ASSERT(sizeof(GmskyRndObject) == 48);
GMSKY_ASSERT(offsetof(GmskyRndObject, feet) == 24);
GMSKY_ASSERT(sizeof(GmskyRndMatrix) == 48);
GMSKY_ASSERT(GMSKY_RND_MAP_SIZE == 67113088u);

#ifdef __cplusplus
#include <atomic>
#include <cmath>
#include <cstring>

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
// The other way (step 3b, NPC facing): Source yaw in [-180, 180).
inline double SkyrimHeadingToGmodYaw(double heading_rad) {
    const double y = std::fmod(90.0 - heading_rad * 180.0 / kPi + 180.0, 360.0);
    return (y < 0 ? y + 360.0 : y) - 180.0;
}

// Both games: positive pitch looks down. VERIFIED in game 2026-10-04: looking straight down gave
// Skyrim 90.0 deg and Garry's Mod 89.0 deg.
inline double GmodPitchToSkyrimPitch(double pitch_deg) { return pitch_deg * kPi / 180.0; }

// ---- Render ring (step G2): the C++ reader Skyrim uses, its message checks, and a C++ copy of
// the writer (for protocol_test). The same rules as protocol/src/render.rs. ----
namespace rnd {

inline uint64_t TextureBytes(uint32_t width, uint32_t height, uint32_t format) {
    const uint64_t bw = (uint64_t(width) + 3) / 4, bh = (uint64_t(height) + 3) / 4;
    switch (format) {
    case GMSKY_RND_TEX_BC1: return bw * bh * 8;
    case GMSKY_RND_TEX_BC2:
    case GMSKY_RND_TEX_BC3: return bw * bh * 16;
    case GMSKY_RND_TEX_RGBA8: return uint64_t(width) * height * 4;
    default: return 0;
    }
}

inline std::atomic_ref<uint64_t> Counter(uint8_t* a_base, uint32_t a_off) {
    return std::atomic_ref<uint64_t>(*reinterpret_cast<uint64_t*>(a_base + a_off));
}

// Writer: one message; false if the ring is too full now or the message is over half the ring.
inline bool Write(uint8_t* a_base, uint32_t a_type, const void* a_payload, uint32_t a_bytes) {
    constexpr uint64_t size = GMSKY_RND_RING_BYTES;
    const uint64_t msg = (8 + uint64_t(a_bytes) + 7) & ~7ull;
    uint64_t head = Counter(a_base, GMSKY_RND_HEAD_OFF).load(std::memory_order_relaxed);
    const uint64_t tail = Counter(a_base, GMSKY_RND_TAIL_OFF).load(std::memory_order_acquire);
    uint64_t pos = head % size;
    const uint64_t pad = pos + msg > size ? size - pos : 0;
    if (msg > size / 2 || size - (head - tail) < msg + pad) {
        return false;
    }
    uint8_t* data = a_base + GMSKY_RND_DATA_OFF;
    if (pad) {
        const GmskyColMsgHeader p{GMSKY_RND_PAD, 0};
        std::memcpy(data + pos, &p, sizeof(p));
        head += pad;
        pos = 0;
    }
    const GmskyColMsgHeader h{a_type, a_bytes};
    std::memcpy(data + pos, &h, sizeof(h));
    std::memcpy(data + pos + 8, a_payload, a_bytes);
    Counter(a_base, GMSKY_RND_HEAD_OFF).store(head + msg, std::memory_order_release);
    return true;
}

// Reader: calls a_fn(type, payload, bytes) per message, oldest first, until the ring is empty or
// about a_maxBytes were read. Returns the bytes read; a_bad is set (and everything pending skipped)
// if the ring holds something that isn't a message.
template <class F>
inline uint64_t Drain(uint8_t* a_base, F&& a_fn, uint64_t a_maxBytes, bool& a_bad) {
    constexpr uint64_t size = GMSKY_RND_RING_BYTES;
    a_bad = false;
    const uint64_t head = Counter(a_base, GMSKY_RND_HEAD_OFF).load(std::memory_order_acquire);
    const uint64_t start = Counter(a_base, GMSKY_RND_TAIL_OFF).load(std::memory_order_relaxed);
    uint64_t tail = start;
    const uint8_t* data = a_base + GMSKY_RND_DATA_OFF;
    while (tail < head && tail - start < a_maxBytes) {
        const uint64_t pos = tail % size;
        GmskyColMsgHeader h;
        std::memcpy(&h, data + pos, sizeof(h));
        if (h.type == GMSKY_RND_PAD) {
            tail += size - pos;
            continue;
        }
        const uint64_t msg = (8 + uint64_t(h.bytes) + 7) & ~7ull;
        if (pos + msg > size || tail + msg > head) {
            a_bad = true;
            tail = head;
            break;
        }
        a_fn(h.type, data + pos + 8, h.bytes);
        tail += msg;
    }
    Counter(a_base, GMSKY_RND_TAIL_OFF).store(tail, std::memory_order_release);
    return tail - start;
}

// A checked view into a MODEL message (pointers into the payload; unaligned reads are fine on x64).
struct ModelView {
    GmskyRndModel model;
    const GmskyRndPart* parts;
    const GmskyRndVertex* vertices;
    const uint32_t* indices;
};

// Every check render.rs's decode_model makes. Returns nullptr if fine, else what's wrong.
inline const char* ParseModel(const uint8_t* a_p, uint32_t a_bytes, ModelView& a_out) {
    if (a_bytes < sizeof(GmskyRndModel)) return "model header cut short";
    std::memcpy(&a_out.model, a_p, sizeof(GmskyRndModel));
    const auto& m = a_out.model;
    const uint64_t atV = sizeof(GmskyRndModel) + uint64_t(m.part_count) * sizeof(GmskyRndPart);
    const uint64_t atI = atV + uint64_t(m.vertex_count) * sizeof(GmskyRndVertex);
    if (a_bytes != atI + uint64_t(m.index_count) * 4) return "model sizes don't add up";
    if (m.index_count % 3) return "model indices aren't whole triangles";
    a_out.parts = reinterpret_cast<const GmskyRndPart*>(a_p + sizeof(GmskyRndModel));
    a_out.vertices = reinterpret_cast<const GmskyRndVertex*>(a_p + atV);
    a_out.indices = reinterpret_cast<const uint32_t*>(a_p + atI);
    for (uint32_t k = 0; k < m.part_count; ++k) {
        const auto& part = a_out.parts[k];
        if (part.index_count % 3 || uint64_t(part.first_index) + part.index_count > m.index_count) return "model part outside its indices";
    }
    for (uint32_t k = 0; k < m.index_count; ++k) {
        if (a_out.indices[k] >= m.vertex_count) return "model index past its vertices";
    }
    for (uint32_t k = 0; k < m.vertex_count; ++k) {
        const auto& v = a_out.vertices[k];
        for (int b = 0; b < 4; ++b) {
            if (v.weights[b] && v.bones[b] >= m.bone_count) return "model vertex on a bone past its bones";
        }
    }
    return nullptr;
}

struct FrameView {
    GmskyRndFrame frame;
    const GmskyRndObject* objects;
    const GmskyRndMatrix* matrices;
};

inline const char* ParseFrame(const uint8_t* a_p, uint32_t a_bytes, FrameView& a_out) {
    if (a_bytes < sizeof(GmskyRndFrame)) return "frame header cut short";
    std::memcpy(&a_out.frame, a_p, sizeof(GmskyRndFrame));
    const auto& f = a_out.frame;
    const uint64_t atM = sizeof(GmskyRndFrame) + uint64_t(f.object_count) * sizeof(GmskyRndObject);
    if (a_bytes != atM + uint64_t(f.matrix_count) * sizeof(GmskyRndMatrix)) return "frame sizes don't add up";
    a_out.objects = reinterpret_cast<const GmskyRndObject*>(a_p + sizeof(GmskyRndFrame));
    a_out.matrices = reinterpret_cast<const GmskyRndMatrix*>(a_p + atM);
    for (uint32_t k = 0; k < f.object_count; ++k) {
        const auto& o = a_out.objects[k];
        if (!o.matrix_count || uint64_t(o.first_matrix) + o.matrix_count > f.matrix_count) return "frame object's matrices outside the frame";
    }
    return nullptr;
}

inline const char* ParseTexture(const uint8_t* a_p, uint32_t a_bytes, GmskyRndTexture& a_out, const uint8_t*& a_pixels) {
    if (a_bytes < sizeof(GmskyRndTexture)) return "texture header cut short";
    std::memcpy(&a_out, a_p, sizeof(a_out));
    const uint64_t want = TextureBytes(a_out.width, a_out.height, a_out.format);
    if (!want) return "texture format unknown";
    if (!a_out.width || !a_out.height || a_out.bytes != want || a_bytes != sizeof(GmskyRndTexture) + a_out.bytes) return "texture size doesn't match its format";
    a_pixels = a_p + sizeof(GmskyRndTexture);
    return nullptr;
}

}  // namespace rnd

}  // namespace gmsky
#endif
