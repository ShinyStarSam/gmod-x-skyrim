-- gmsky physics objects against Skyrim's ground: measurement only (DESIGN.md "One collision
-- system", step S0). Changes nothing in the game.
-- Runs on both sides (server.lua and client.lua include it). The server watches every physics
-- object (props, ragdolls, grenades); the client watches the physics objects that exist only on
-- the client (death ragdolls, gibs), which live in the client's own physics world.
--
-- How "fell through" is judged: every tick, each object's centre moved from A to B (relative to
-- the anchor). If the straight line A -> B crosses one of Skyrim's triangles (gmsky.Sweep with a
-- zero-size box), the object went through Skyrim's world there. Through a floor while moving down
-- = FELL THROUGH; anything else (a wall, a ceiling) = PASSED THROUGH. When the anchor changes (a
-- re-centre or a new origin) the line is not tested for that tick: today props aren't moved by a
-- re-centre, so their anchor-relative position jumps without them moving.

local gmsky = gmsky
local P = gmsky.props or {}
gmsky.props = P

local DOWN = 4000      -- how far below an object to look for Skyrim's ground in the status line
local DETAIL = 20      -- crossings logged in full; after that only counted
local LISTED = 6       -- objects listed in each status line

P.track = {}           -- entity -> { x, y, z (anchor-relative centre), fell = count, passed = count }
P.total = P.total or { fell = 0, passed = 0, logged = 0 }
P.second = { fell = 0, passed = 0, sweeps = 0 }
P.anchor = nil         -- the anchor the stored positions are relative to

-- Our own entities and things that aren't free physics objects.
local function Watched(ent)
	if not IsValid(ent) or ent:IsPlayer() or ent:IsWorld() then return false end
	local class = ent:GetClass()
	if class:sub(1, 6) == "gmsky_" then return false end
	if IsValid(ent:GetParent()) then return false end
	if CLIENT then
		-- Client side: only things that exist on the client alone (no server entity behind them).
		return ent:EntIndex() < 0 and IsValid(ent:GetPhysicsObject())
	end
	return ent:GetMoveType() == MOVETYPE_VPHYSICS and IsValid(ent:GetPhysicsObject())
end

local function Short(ent)
	local m = ent:GetModel() or ""
	m = m:match("([^/]+)%.mdl$") or m
	return ent:GetClass() .. (m ~= "" and (" " .. m) or "")
end

local function AllEntities()
	if ents.Iterator then
		local list = {}
		for _, e in ents.Iterator() do list[#list + 1] = e end
		return list
	end
	return ents.GetAll and ents.GetAll() or {}
end

-- One step: test every watched object's movement since the last step.
function P.Step()
	local ax, ay, az = gmsky.Anchor()
	if not ax then P.track = {} P.anchor = nil return end
	local moved = not P.anchor or P.anchor[1] ~= ax or P.anchor[2] ~= ay or P.anchor[3] ~= az
	P.anchor = { ax, ay, az }
	local seen = {}
	for _, ent in ipairs(AllEntities()) do
		if Watched(ent) then
			seen[ent] = true
			local c = ent:WorldSpaceCenter()
			local x, y, z = c.x - ax, c.y - ay, c.z - az
			local t = P.track[ent]
			if not t then
				t = { fell = 0, passed = 0 }
				P.track[ent] = t
			elseif not moved then
				local dx, dy, dz = x - t.x, y - t.y, z - t.z
				if dx * dx + dy * dy + dz * dz > 0.0001 then
					P.second.sweeps = P.second.sweeps + 1
					local frac, nx, ny, nz = gmsky.Sweep(t.x, t.y, t.z, 0, 0, 0, dx, dy, dz)
					if frac then
						local fell = nz > 0.5 and dz < 0
						if fell then
							t.fell = t.fell + 1
							P.total.fell = P.total.fell + 1
							P.second.fell = P.second.fell + 1
						else
							t.passed = t.passed + 1
							P.total.passed = P.total.passed + 1
							P.second.passed = P.second.passed + 1
						end
						if P.total.logged < DETAIL then
							P.total.logged = P.total.logged + 1
							local v = ent:GetVelocity()
							gmsky.Log(string.format("props: %s %s (#%d of this object) at (%.1f, %.1f, %.1f) relative to the anchor: crossed Skyrim's surface at (%.1f, %.1f, %.1f), its normal (%.2f, %.2f, %.2f); moved (%.1f, %.1f, %.1f) this step, speed %.0f",
								fell and "FELL THROUGH" or "PASSED THROUGH", Short(ent), fell and t.fell or t.passed, x, y, z,
								t.x + dx * frac, t.y + dy * frac, t.z + dz * frac, nx, ny, nz, dx, dy, dz, v:Length()))
						end
					end
				end
			end
			t.x, t.y, t.z = x, y, z
		end
	end
	for ent in pairs(P.track) do
		if not seen[ent] then P.track[ent] = nil end
	end
	if moved then P.anchorChanges = (P.anchorChanges or 0) + 1 end
end

-- Height of gm_flatmap's own floor (the map's brushes) under a point, Garry's Mod world z.
local function MapFloor(pos)
	local tr = util.TraceLine({ start = pos, endpos = pos - Vector(0, 0, 40000), mask = MASK_SOLID_BRUSHONLY })
	return tr.Hit and tr.HitPos.z or nil
end

-- Once a second: what every watched object is doing.
function P.Status()
	local s = P.second
	P.second = { fell = 0, passed = 0, sweeps = 0 }
	local n, awake = 0, 0
	local list = {}
	for ent, t in pairs(P.track) do
		if IsValid(ent) then
			n = n + 1
			local phys = ent:GetPhysicsObject()
			local isAwake = IsValid(phys) and phys:IsAsleep() == false
			if isAwake then awake = awake + 1 end
			list[#list + 1] = { ent = ent, t = t, awake = isAwake }
		end
	end
	if n == 0 and s.fell == 0 and s.passed == 0 then return end
	if not P.floorLogged then
		local ply = SERVER and player.GetAll()[1] or LocalPlayer()
		if IsValid(ply) then
			local f = MapFloor(ply:GetPos())
			P.floorLogged = true
			gmsky.Log(string.format("props: gm_flatmap's own floor under the player is at z %s (Garry's Mod world); the player is at z %.1f",
				f and string.format("%.1f", f) or "(none found)", ply:GetPos().z))
		end
	end
	table.sort(list, function(a, b) return a.ent:EntIndex() < b.ent:EntIndex() end)
	-- Every class watched (the list below shows only the first few objects).
	local classes, cparts = {}, {}
	for _, it in ipairs(list) do local c = it.ent:GetClass() classes[c] = (classes[c] or 0) + 1 end
	for c, k in pairs(classes) do cparts[#cparts + 1] = c .. " " .. k end
	table.sort(cparts)
	local parts = {}
	for i = 1, math.min(#list, LISTED) do
		local e, t = list[i].ent, list[i].t
		local frac = gmsky.Sweep(t.x, t.y, t.z, 0, 0, 0, 0, 0, -DOWN)
		local ground = frac and string.format("%.1f above Skyrim's ground", DOWN * frac)
			or (gmsky.ShapesKnown(t.x, t.y, t.z) and string.format("NO Skyrim surface within %d below", DOWN)
			or "Skyrim's collision here not arrived")
		local floor = MapFloor(e:WorldSpaceCenter())
		parts[#parts + 1] = string.format("%s at (%.0f, %.0f, %.0f) speed %.0f %s, %s, %s%s",
			Short(e), t.x, t.y, t.z, e:GetVelocity():Length(), list[i].awake and "awake" or "asleep", ground,
			floor and string.format("%.0f above the map's floor", e:WorldSpaceCenter().z - floor) or "no map floor below",
			(t.fell + t.passed > 0) and string.format(", crossed %d down / %d other", t.fell, t.passed) or "")
	end
	gmsky.Log(string.format("props: %d watched (%s; %d awake); this second FELL THROUGH %d, PASSED THROUGH %d (%d line tests, also counted in the shapes status' sweeps); total %d / %d%s%s",
		n, table.concat(cparts, ", "), awake, s.fell, s.passed, s.sweeps, P.total.fell, P.total.passed,
		#parts > 0 and "; " or "", table.concat(parts, "; ")))
end

local function SafeStep()
	local ok, err = pcall(P.Step)
	if not ok and not P.stepError then
		P.stepError = true -- logged once, so a broken step can't flood the log
		gmsky.Log("props: ERROR in step: " .. tostring(err))
	end
end
hook.Add(SERVER and "Tick" or "Think", "gmsky_props_watch", SafeStep)
timer.Create("gmsky_props_status", 1, 0, function()
	local ok, err = pcall(P.Status)
	if not ok then gmsky.Log("props: ERROR in status: " .. tostring(err)) end
end)
