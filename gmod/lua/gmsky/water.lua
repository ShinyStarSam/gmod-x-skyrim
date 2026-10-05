-- gmsky, both sides, Phase 4 step 4b: swimming in Skyrim's water. Garry's Mod's map has no water
-- where Skyrim has rivers and lakes, so Skyrim sends the water surface where its player is
-- (gmsky.WaterZ) and, from waist deep, this replaces Garry's Mod's movement with Source's own water
-- movement (CGameMovement::WaterMove, re-written in Lua): no gravity, swim where you look, Jump
-- swims up, sink slowly with no keys, water friction, slide along Skyrim's collision. Waist out of
-- the water, Garry's Mod's own movement (and gravity) takes over again, so you bob at the surface.
-- Both sides run it (movement prediction). DESIGN.md "4b. Water and swimming".

local gmsky = gmsky

local W = gmsky.water or {}
gmsky.water = W
W.SINK = 60         -- units/s down with no keys held (Source)
W.SWIM_SPEED = 0.8  -- of the walking speed (Source)
W.JUMP_OUT = 300    -- units/s up when climbing out onto a bank
W.stats = W.stats or { ticks = 0, deepest = 0, jumpsOut = 0 }

local function ConVarNum(name, default)
	local cv = GetConVar and GetConVar(name)
	return cv and cv:GetFloat() or default
end

local function Hull(ply)
	if ply:Crouching() then return ply:GetHullDuck() end
	return ply:GetHull()
end

-- How deep the player is, as Source counts it: 0 out, 1 feet, 2 waist, 3 eyes under the surface.
function W.Level(ply, feetZ, water)
	if not water or feetZ >= water then return 0 end
	local mins, maxs = Hull(ply)
	if feetZ + (mins.z + maxs.z) * 0.5 >= water then return 1 end
	local eye = 64
	if ply.GetViewOffset then eye = (ply:Crouching() and ply:GetViewOffsetDucked() or ply:GetViewOffset()).z end
	if feetZ + eye >= water then return 2 end
	return 3
end

local function Dot(a, b) return a.x * b.x + a.y * b.y + a.z * b.z end

-- Moves the hull by vel * dt, sliding along whatever it hits (Source's TryPlayerMove, simplified).
local function SlideMove(ply, pos, vel, dt, mins, maxs)
	local left = dt
	for _ = 1, 4 do
		if left <= 0 then break end
		local tr = util.TraceHull({ start = pos, endpos = pos + vel * left, mins = mins, maxs = maxs, mask = MASK_PLAYERSOLID, filter = ply })
		if tr.StartSolid then break end -- stuck: stay (the terrain code lifts the player out)
		pos = tr.HitPos
		if not tr.Hit then break end
		left = left * (1 - tr.Fraction)
		local into = Dot(vel, tr.HitNormal)
		if into < 0 then vel = vel - tr.HitNormal * into end
	end
	return pos, vel
end

-- Jump at the surface facing a bank: a wall in front at the waist, free space in front at the eyes
-- (Source's CheckWaterJump).
local function CanClimbOut(ply, pos, flat, mins, maxs)
	local waist = pos + Vector(0, 0, (mins.z + maxs.z) * 0.5)
	local tr = util.TraceLine({ start = waist, endpos = waist + flat * 24, mask = MASK_PLAYERSOLID, filter = ply })
	if not tr.Hit or (tr.HitNormal and tr.HitNormal.z > 0.7) then return false end
	local eye = pos + Vector(0, 0, maxs.z)
	tr = util.TraceLine({ start = eye, endpos = eye + flat * 24, mask = MASK_PLAYERSOLID, filter = ply })
	return not tr.Hit
end

-- Server: entering and leaving the water, and per-second numbers.
local function Note(level, water, pos)
	if not SERVER then return end
	W.last = { level = level, water = water, z = pos.z }
	local swimming = level >= 2
	if swimming ~= (W.swimming or false) then
		W.swimming = swimming
		gmsky.Log(string.format("water: %s at feet z %.1f, surface z %.1f (level %d)", swimming and "SWIMMING" or "out of the water",
			pos.z, water or -1, level))
	end
	if swimming then
		W.stats.ticks = W.stats.ticks + 1
		W.stats.deepest = math.max(W.stats.deepest, water - pos.z)
	end
end

hook.Add("Move", "gmsky_water", function(ply, mv)
	if not ply:Alive() or ply:GetMoveType() ~= MOVETYPE_WALK then return end
	if gmsky.terrain and gmsky.terrain.holding then return end
	local water = gmsky.WaterZ()
	local pos = mv:GetOrigin()
	local level = W.Level(ply, pos.z, water)
	Note(level, water, pos)
	if level < 2 then return end

	-- Source's WaterMove.
	local dt = FrameTime()
	local ang = mv:GetMoveAngles()
	local fwd, right = ang:Forward(), ang:Right()
	local fmove, smove, umove = mv:GetForwardSpeed(), mv:GetSideSpeed(), mv:GetUpSpeed()
	local maxspeed = mv:GetMaxSpeed()
	-- Source's CheckParameters: the keys ask for "full" (10,000 units/s); scaled to the top speed
	-- first (after test 67: without it W swamped Jump's 200 up, so W + Space barely rose).
	local asked = math.sqrt(fmove * fmove + smove * smove + umove * umove)
	if asked > maxspeed and asked > 0 then
		local k = maxspeed / asked
		fmove, smove, umove = fmove * k, smove * k, umove * k
	end
	local wish = fwd * fmove + right * smove
	local jump = mv:KeyDown(IN_JUMP)
	if jump then
		wish.z = wish.z + maxspeed
	elseif fmove == 0 and smove == 0 and umove == 0 then
		wish.z = wish.z - W.SINK
	else
		-- Source: swimming forward while looking up rises extra.
		wish.z = wish.z + umove + math.Clamp(fmove * fwd.z * 2, 0, maxspeed)
	end
	local wishspeed = wish:Length()
	local wishdir = wishspeed > 0 and wish * (1 / wishspeed) or Vector(0, 0, 0)
	wishspeed = math.min(wishspeed, maxspeed) * W.SWIM_SPEED

	local vel = mv:GetVelocity()
	local speed = vel:Length()
	local newspeed = 0
	if speed > 0 then
		newspeed = math.max(speed - dt * speed * ConVarNum("sv_friction", 8), 0)
		vel = vel * (newspeed / speed)
	end
	if wishspeed >= 0.1 then
		local add = wishspeed - newspeed
		if add > 0 then
			vel = vel + wishdir * math.min(ConVarNum("sv_accelerate", 10) * wishspeed * dt, add)
		end
	end

	local mins, maxs = Hull(ply)
	if jump and level == 2 then
		local flat = Vector(fwd.x, fwd.y, 0)
		local l = flat:Length()
		if l > 0.01 and CanClimbOut(ply, pos, flat * (1 / l), mins, maxs) then
			vel = flat * (100 / l) + Vector(0, 0, W.JUMP_OUT)
			if SERVER then W.stats.jumpsOut = W.stats.jumpsOut + 1 end
		end
	end

	pos, vel = SlideMove(ply, pos, vel, dt, mins, maxs)
	mv:SetVelocity(vel)
	mv:SetOrigin(pos)
	if ply.SetGroundEntity then ply:SetGroundEntity(NULL) end
	return true -- Garry's Mod's own movement (gravity) skipped this tick
end)

-- Once a second (server.lua's status timer).
function W.Status()
	local st, last = W.stats, W.last
	local line = ""
	if last and last.water then
		line = string.format(" | water surface z %.1f, feet %.1f %s it", last.water, math.abs(last.z - last.water), last.z < last.water and "below" or "above")
		if st.ticks > 0 then
			line = line .. string.format(", swam %d ticks (deepest %.1f)%s", st.ticks, st.deepest,
				st.jumpsOut > 0 and string.format(", %d climbs out", st.jumpsOut) or "")
		end
	end
	W.stats = { ticks = 0, deepest = 0, jumpsOut = 0 }
	return line
end

gmsky.Log("water.lua loaded")
