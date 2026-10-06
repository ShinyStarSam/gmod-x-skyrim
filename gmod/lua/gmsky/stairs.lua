-- gmsky, both sides, step S2h: stairs. SkyCraft's stepping rule (TriCollider.java, adapted from
-- SkyCraft (MIT): ground judged under a small footprint, steep surfaces below step height don't
-- stop a grounded player) added on top of Source's own movement, which stays in charge of speed,
-- friction, jumping and air control.
-- Test 125: on Skyrim's stairs the player's 32-unit box rests on the step edges, the surface under
-- it is a slanted riser (normal z 0.2-0.4, under Source's 0.7 for "ground"), so Source says "in the
-- air", and Source only steps up from the ground: stuck. Two rules, only against Skyrim's ground
-- (gmsky_chunk entities):
--  1. Ground under a small footprint: Source says "in the air", but straight-down line traces at the
--     centre and 4 points FOOT units from it find a walkable surface (normal z >= 0.7) within
--     FOOT_GAP units: the player is put on the ground (Source keeps the ground of the last tick
--     into the next one, so its next move is a ground move: friction, jumping, its own step-up).
--  2. Step-up when blocked: the move went less than BLOCKED of the way the keys asked, and the
--     player was on the ground (Source's or rule 1's) within RECENT seconds: Source's StepMove
--     (up by the step height, the move forward, back down), kept only if it lands on walkable
--     ground under the small footprint and gets further than Source did.
-- Propped on step edges (after test 126: 0 real steps, every try "lands on no walkable ground"):
-- the 32-unit box lands on the slanted edge of the next step, with the step's flat top a few units
-- under its centre. So both rules also count the player as standing when the box rests on
-- Skyrim's ground (a 2-unit box trace down hits it) and the small footprint finds walkable Skyrim
-- ground within the step height under the feet (Source lets a box stand on step edges too; here
-- the edges are slanted). SkyCraft doesn't need this: its body may overlap steps below step height.
-- Differs from SkyCraft: SkyCraft owns all of the player's collision; here Source still does and
-- only these two rules are added. Differs from Source: Source's own ground test uses the whole box.
-- Both sides run it (movement prediction); only the server counts and logs.
-- `gmsky_stairs 0` switches it off for comparison. DESIGN.md "S2h".

local gmsky = gmsky

local S = gmsky.stairs or {}
gmsky.stairs = S
S.FOOT = 8        -- units from the centre for the 4 outer footprint traces (the box is 16 each way)
S.FOOT_GAP = 2    -- walkable ground this close under the feet counts
S.WALKABLE = 0.7  -- Source's own limit for ground
S.BLOCKED = 0.6   -- a move that got less than this share of the asked distance was blocked
S.RECENT = 0.1    -- seconds after leaving the ground that a step is still allowed
S.RISING = 140    -- units/s up: faster than this is a jump or a launch, never a slanted riser's push
S.cvar = S.cvar or CreateConVar("gmsky_stairs", "1", FCVAR_REPLICATED,
	"1 = S2h stairs: ground under a small footprint and step-up when blocked, on Skyrim's ground; 0 = Source's movement only")

local function NewStats()
	return { footGround = 0, edgeGround = 0, kept = 0, keptOf = 0, steps = 0, edgeSteps = 0, stepHeight = 0, tries = 0, rejects = {} }
end
S.stats = S.stats or NewStats()
S.logged = S.logged or 0
local STEP_LOG_MAX = 300 -- single step lines per map load; counts go on in the status line

local function Hull(ply)
	if ply:Crouching() then return ply:GetHullDuck() end
	return ply:GetHull()
end

local function Skyrim(ent) return IsValid(ent) and ent:GetClass() == "gmsky_chunk" end

-- Walkable Skyrim ground within `reach` (default FOOT_GAP) under the feet at `pos`, under the
-- small footprint: the entity and the gap, or nil.
local OFFSETS = { { 0, 0 }, { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }
function S.FootGround(ply, pos, reach)
	local best, gap
	for _, o in ipairs(OFFSETS) do
		local p = pos + Vector(o[1] * S.FOOT, o[2] * S.FOOT, 0.5)
		local tr = util.TraceLine({ start = p, endpos = p - Vector(0, 0, (reach or S.FOOT_GAP) + 0.5), mask = MASK_PLAYERSOLID, filter = ply })
		if tr.Hit and not tr.StartSolid and tr.HitNormal.z >= S.WALKABLE and Skyrim(tr.Entity) then
			local g = pos.z - tr.HitPos.z
			if not gap or g < gap then best, gap = tr.Entity, g end
		end
	end
	return best, gap
end

-- Standing at `pos`: walkable ground within FOOT_GAP under the small footprint, or propped on step
-- edges (the box rests on Skyrim's ground and walkable ground is within the step height under the
-- footprint). The ground entity and whether it was the step-edge case, or nil.
function S.Standing(ply, pos, mins, maxs)
	local ground = S.FootGround(ply, pos)
	if ground then return ground, false end
	local tr = util.TraceHull({ start = pos, endpos = pos - Vector(0, 0, S.FOOT_GAP), mins = mins, maxs = maxs, mask = MASK_PLAYERSOLID, filter = ply })
	if tr.StartSolid or not tr.Hit or not Skyrim(tr.Entity) then return nil end
	ground = S.FootGround(ply, pos, ply:GetStepSize())
	if ground then return ground, true end
end

local function Reject(why)
	if SERVER then S.stats.rejects[why] = (S.stats.rejects[why] or 0) + 1 end
end

local function Flat(v) return Vector(v.x, v.y, 0) end

-- Before Source's move: where the player starts, and the direction and distance the keys ask for.
hook.Add("SetupMove", "gmsky_stairs", function(ply, mv, cmd)
	ply.gmskyStairs = nil
	if not S.cvar:GetBool() or not ply:Alive() or ply:GetMoveType() ~= MOVETYPE_WALK then return end
	if gmsky.terrain and gmsky.terrain.holding then return end
	local st = ply.gmskyStairsState or {}
	ply.gmskyStairsState = st
	-- Did rule 1's ground from the last tick survive into this one? (Source keeps it unless its
	-- sv_optimizedmovement is off; counted to check.)
	if SERVER and st.forced then
		S.stats.keptOf = S.stats.keptOf + 1
		if Skyrim(ply:GetGroundEntity()) then S.stats.kept = S.stats.kept + 1 end
	end
	st.forced = false
	local fmove, smove = mv:GetForwardSpeed(), mv:GetSideSpeed()
	if math.abs(fmove) < 1 and math.abs(smove) < 1 then
		ply.gmskyStairs = { origin = mv:GetOrigin() }
		return
	end
	local ang = Angle(0, mv:GetMoveAngles().y, 0)
	local wish = ang:Forward() * fmove + ang:Right() * smove
	local asked = wish:Length()
	local wishdir = wish * (1 / asked)
	local wishspeed = math.min(asked, mv:GetMaxSpeed())
	-- The speed Source's acceleration would reach this tick from the current speed along wishdir.
	local now = math.max(Flat(mv:GetVelocity()):Dot(wishdir), 0)
	local accel = GetConVar("sv_accelerate") and GetConVar("sv_accelerate"):GetFloat() or 10
	local speed = math.min(wishspeed, now + accel * wishspeed * FrameTime())
	ply.gmskyStairs = { origin = mv:GetOrigin(), wishdir = wishdir, speed = speed, dist = speed * FrameTime() }
end)

-- Rule 2: Source's StepMove from `p0` along `dir` for `dist`; the landing spot and its ground, or nil.
local function StepMove(ply, p0, dir, dist, mins, maxs)
	local step = ply:GetStepSize()
	local function Trace(a, b) return util.TraceHull({ start = a, endpos = b, mins = mins, maxs = maxs, mask = MASK_PLAYERSOLID, filter = ply }) end
	local up = Trace(p0, p0 + Vector(0, 0, step))
	if up.StartSolid then return Reject("starts inside something") end
	local pUp = up.HitPos
	local fwd = Trace(pUp, pUp + dir * dist)
	if fwd.StartSolid then return Reject("inside something at step height") end
	if fwd.Fraction < 0.05 then return Reject("blocked at step height too (a wall, not a step)") end
	local pF = fwd.HitPos
	local down = Trace(pF, pF - Vector(0, 0, (pUp.z - p0.z) + S.FOOT_GAP))
	if down.StartSolid then return Reject("inside something coming down") end
	if not down.Hit then return Reject("nothing under the far side (a drop, not a step)") end
	local land = down.HitPos
	local ground, edge = S.Standing(ply, land, mins, maxs)
	if not ground then return Reject("lands on no walkable Skyrim ground under the small footprint") end
	return land, ground, edge
end

-- After Source's move: the two rules.
hook.Add("FinishMove", "gmsky_stairs", function(ply, mv)
	local pre = ply.gmskyStairs
	ply.gmskyStairs = nil
	if not pre then return end
	local st = ply.gmskyStairsState
	if gmsky.water and gmsky.WaterZ and gmsky.water.Level(ply, mv:GetOrigin().z, gmsky.WaterZ()) >= 2 then return end
	local pos, vel = mv:GetOrigin(), mv:GetVelocity()
	local mins, maxs = Hull(ply)
	local onGround = IsValid(ply:GetGroundEntity()) or ply:GetGroundEntity() == game.GetWorld()
	local now = CurTime()
	-- Not while jumping or launched. A slanted riser pushes a player Source thinks is in the air
	-- up a little (Source slides the speed along it), so "going up" alone doesn't mean jumping.
	local rising = vel.z > S.RISING or (vel.z > 0 and mv:KeyDown(IN_JUMP))

	-- Was the player on the ground at the start (Source's or the small footprint's)?
	local startGround = S.Standing(ply, pre.origin, mins, maxs)
	if startGround or onGround then st.groundAt = now end

	-- Rule 2: step-up when blocked.
	if pre.wishdir and not rising and st.groundAt and now - st.groundAt <= S.RECENT + 1e-4 then
		local went = Flat(pos - pre.origin):Dot(pre.wishdir)
		if went < pre.dist * S.BLOCKED then
			if SERVER then S.stats.tries = S.stats.tries + 1 end
			local land, ground, edge = StepMove(ply, pre.origin, pre.wishdir, pre.dist, mins, maxs)
			if land then
				local wentStep = Flat(land - pre.origin):Dot(pre.wishdir)
				if wentStep <= went + 0.1 then
					Reject("no further than Source got")
				else
					mv:SetOrigin(land)
					local v = pre.wishdir * pre.speed
					mv:SetVelocity(Vector(v.x, v.y, 0))
					ply:SetGroundEntity(ground)
					st.groundAt = now
					if SERVER then
						S.stats.steps = S.stats.steps + 1
						if edge then S.stats.edgeSteps = S.stats.edgeSteps + 1 end
						S.stats.stepHeight = math.max(S.stats.stepHeight, land.z - pre.origin.z)
						if S.logged < STEP_LOG_MAX then
							S.logged = S.logged + 1
							gmsky.Log(string.format("stairs: STEP #%d at (%.1f, %.1f, %.1f): up %.1f, %.1f units forward (Source got %.1f of %.1f asked), lands on %s%s%s",
								S.logged, pre.origin.x, pre.origin.y, pre.origin.z, land.z - pre.origin.z, wentStep, went, pre.dist, ground:ChunkName(),
								edge and " (propped on step edges)" or "",
								S.logged == STEP_LOG_MAX and " (last single step line; counts go on in the status line)" or ""))
						end
					end
					return
				end
			end
		end
	end

	-- Rule 1: ground under the small footprint where Source says "in the air".
	if not onGround and not rising then
		local ground, edge = S.Standing(ply, pos, mins, maxs)
		if ground then
			ply:SetGroundEntity(ground)
			mv:SetVelocity(Vector(vel.x, vel.y, 0))
			st.forced, st.groundAt = true, now
			if SERVER then
				S.stats.footGround = S.stats.footGround + 1
				if edge then S.stats.edgeGround = S.stats.edgeGround + 1 end
			end
		end
	end
end)

-- Once a second (server.lua's status line).
function S.Status()
	if not S.cvar:GetBool() then return " | stairs: off (gmsky_stairs 0)" end
	local st = S.stats
	local rej = {}
	for why, n in pairs(st.rejects) do rej[#rej + 1] = string.format("%s %d", why, n) end
	table.sort(rej)
	local line = string.format(" | stairs: footprint-ground ticks %d (%d propped on step edges; ground kept into the next tick %d of %d), step tries %d, steps %d (%d onto step edges, highest %.1f)%s",
		st.footGround, st.edgeGround, st.kept, st.keptOf, st.tries, st.steps, st.edgeSteps, st.stepHeight, #rej > 0 and (", rejected: " .. table.concat(rej, ", ")) or "")
	S.stats = NewStats()
	return line
end

if SERVER then
	local opt = GetConVar("sv_optimizedmovement")
	gmsky.Log(string.format("stairs.lua loaded (gmsky_stairs %d; sv_optimizedmovement %s)", S.cvar:GetBool() and 1 or 0, opt and opt:GetString() or "not found"))
else
	gmsky.Log("stairs.lua loaded")
end
