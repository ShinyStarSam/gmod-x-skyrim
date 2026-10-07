# STATUS

**Resolved (test 133, 2026-10-06; ramps player-only since test 135):** the stone stairs work in game ("much, much, much better").
The rough spots found in test 133 are fixed (S2i-3, MODLOG); test 134 confirmed it in game ("good").
Ramps not being player-only was fixed in S2i-4 (test 135). Kept below as the record.

## Goal
Garry's Mod's player walks smoothly up and down Skyrim's stairs (step S2i: invisible,
player-only ramps laid over every staircase, built from Skyrim's collision triangles).

## Setup
- Games: Garry's Mod 64-bit (September 2026 update), Skyrim Anniversary Edition 1.7.104 with SKSE.
- Agent / model: Claude Code (Claude Opus 5.5).
- Where things are: Skyrim plugin `skse/src/Collision.cpp` (sends triangles, marks stair
  materials); Garry's Mod module `gmod/module/src/shapes.rs` (`ramps_in_box`, `find_noses`,
  `pair_noses`, `ramp_check`, `dump`); `gmod/lua/gmsky/chunks.lua` (ramp entities, STUCK lines).
  Logs: `build/logs-ingame-test-<N>/`.

## What works (tested)
- Stair materials arrive from Skyrim (test 128): StoneStairs, StoneStairsBroken, StoneAsStairs,
  WoodStairs. All 7 stair materials in Skyrim's list of 78 are checked (also SnowStairs,
  WoodAsStairs, GlassStairs).
- Ramps are built and the player stands on them on *some* stairs (tests 129-131: 14-18 ramp
  entities, 123-252 strips; "standing on gmsky_ramp" in 11-19 status seconds). Server and client
  build identical ramps.

## What doesn't work yet
- **The big stone stairs (fort, inside and outside) get no ramp.** Every STUCK reading in tests
  129-131 is on them.
- Ramps are not player-only: bullets, props and grenades hit them (tests 129-130). Parked by the
  user behind the stairs.

## Which game owns the player
Garry's Mod.

## The current problem
To lay a ramp, the code must find each step's front edge (its "nose") in Skyrim's triangles.
Both ways tried so far assumed a tidy shape that the stone stairs don't have.

## Evidence
- Test 130, riser-based finder: of 45-95 stair triangles around each stuck spot, 21-38 steep
  ones have no single level top edge; 0-24 noses found, mostly unpaired.
- Test 131, tread-based finder: of 72-105 tread edges around each stuck spot, **52-77 are "not
  level"** (ends more than 1 unit apart in height); 1-4 noses found.
- So the stone stairs' collision is neither tidy risers nor tidy flat treads. What it actually
  is has not been seen yet.

## What we've already tried
- **Riser top edges** (S2i-2 as first built, test 129): works on tidy stairs; stone stairs no.
- **Tread front edges** (after test 130, test 131): same result on the stone stairs.
- Both were designed from summary counts, without the real triangles. That is the mistake.

## Update after test 132
The dumps showed the cause: the stone steps are tilted along their length (2-3 units over 30)
and the flight curves; both finders demanded level edges. Noses may now tilt. On all 5 real
dumps a ramp now lies under the player's box (checked offline, `ramps_on_real_dumps`). Not
tested in game yet.

## Next step (done in test 132)
- Test 132 dumps the real triangles: every STUCK reading writes every surface within 96 units
  (stair material or not, with flags and normals) to `logs/tris/stuck_<N>.txt`, and the console
  command `gmsky_dump_tris <name>` does the same anywhere. From those files the shape of the
  stone stairs can be read exactly, and the finder can be built and unit-tested against the real
  geometry before another in-game test.

## Ideas not tried yet
- **The material read may miss triangles.** SkyCraft's method reads the material at the key of
  the *first* container level only. In a shape nested deeper (a list holding a MOPP tree holding
  a mesh), the per-triangle material inside may never be read, so part of a staircase could come
  through without the stair mark. The dump will show whether the stone stairs are fully marked.
- A shape-based finder that doesn't need any stair material (heights sampled on a grid, steps
  found as repeated rises).

## Files that matter
- `gmod/module/src/shapes.rs`: ramp shape, nose finder, ramp check, dump.
- `gmod/lua/gmsky/chunks.lua`: ramp entities, STUCK lines, `gmsky_dump_tris`.
- `skse/src/Collision.cpp`: stair-material marking (`FlagsFor`, `GuardedShapeMaterial`).
