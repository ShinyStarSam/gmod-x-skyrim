-- gmsky terrain (Phase 1 step 3, re-plan step B; DESIGN.md). Runs on both sides: server.lua and
-- client.lua call gmsky.Include("terrain").
--
-- Skyrim sends a 65 x 65 grid of its ground heights (already in Garry's Mod units). Each new grid
-- becomes the triangles of one invisible `gmsky_field` entity (field.lua): Garry's Mod's own
-- movement asks Lua where the player's box touches them (ENT:TestCollision). No physics objects
-- are made any more: building them near the moving player crashed Garry's Mod's physics
-- (STATUS.md "Known engine bug"), and swapping triangles is just data, so nothing is built in
-- pieces, kept, guarded or held for it.
-- On a new origin the player moves to the middle of the map's open space; while there is no Skyrim
-- ground under it, it is held in place instead of falling to the map floor.
-- Logs: gmod_server.log / gmod_client.log.

local gmsky = gmsky
local T = gmsky.terrain or {}
gmsky.terrain = T

-- Skyrim's grids by id (both sides). The client may build a grid a few frames after the server,
-- when a newer one has already arrived, so a few are kept.
T.grids = T.grids or {}

-- Ground height of grid g at (x, y): bilinear between its four points; nil outside or on a miss.
function T.GridHeight(g, x, y)
	local half = (g.n - 1) / 2
	local fi, fj = (x - g.cx) / g.spacing + half, (y - g.cy) / g.spacing + half
	local i, j = math.floor(fi), math.floor(fj)
	if i < 0 or j < 0 or i > g.n - 2 or j > g.n - 2 then return nil end
	local u, v = fi - i, fj - j
	local h = g.h
	local a, b = h[i * g.n + j + 1], h[(i + 1) * g.n + j + 1]
	local c, d = h[(i + 1) * g.n + j + 2], h[i * g.n + j + 2]
	local z = (1 - u) * (1 - v) * a + u * (1 - v) * b + u * v * c + (1 - u) * v * d
	if z ~= z then return nil end
	return z
end

-- The highest ground under the player's 32 x 32 footprint (3a's placement rule), or nil.
function T.FootprintTop(g, x, y)
	local top
	for dx = -16, 16, 8 do
		for dy = -16, 16, 8 do
			local z = T.GridHeight(g, x + dx, y + dy)
			if z then top = math.max(top or z, z) end
		end
	end
	return top
end

-- How much room the map has around a point: traces against the map's walls/floor only.
function T.MeasureRoom(pos)
	local function hit(dir)
		local tr = util.TraceLine({ start = pos, endpos = pos + dir * 40000, mask = MASK_SOLID_BRUSHONLY })
		return tr.Hit and (tr.HitPos - pos):Length() or -1
	end
	local dn, up, px, nx, py, ny = hit(Vector(0, 0, -1)), hit(Vector(0, 0, 1)),
		hit(Vector(1, 0, 0)), hit(Vector(-1, 0, 0)), hit(Vector(0, 1, 0)), hit(Vector(0, -1, 0))
	-- "Home": the middle of the open space, where the player goes on a new origin (3c).
	local home
	if dn >= 0 and up >= 0 and px >= 0 and nx >= 0 and py >= 0 and ny >= 0 then
		home = pos + Vector((px - nx) / 2, (py - ny) / 2, (up - dn) / 2)
	end
	gmsky.Log(string.format("terrain: map %s room from (%.0f, %.0f, %.0f): down %.0f up %.0f +x %.0f -x %.0f +y %.0f -y %.0f (-1 = nothing hit); home %s",
		game.GetMap(), pos.x, pos.y, pos.z, dn, up, px, nx, py, ny,
		home and string.format("(%.0f, %.0f, %.0f)", home.x, home.y, home.z) or "unknown (a wall wasn't found)"))
	return home
end

-- Step 3b: Skyrim's ground grid (GmskyTerrain), already in Garry's Mod units, on both sides.
function T.PollGrid()
	local id, epoch, n, cx, cy, spacing, misses, h = gmsky.ReadTerrain(T.grid and T.grid.id or 0)
	if not epoch then return end
	local lo, hi, nan = math.huge, -math.huge, 0
	for k = 1, #h do
		local z = h[k]
		if z ~= z then nan = nan + 1 else lo = math.min(lo, z); hi = math.max(hi, z) end
	end
	T.grid = { id = id, epoch = epoch, n = n, cx = cx, cy = cy, spacing = spacing, h = h }
	T.grids[id] = T.grid
	T.grids[id - 4] = nil
	gmsky.Log(string.format("terrain: grid %d from Skyrim (origin epoch %d): %dx%d points %.1f apart, centre (%.1f, %.1f), ground z %.1f..%.1f, %d misses (%d NaN)",
		id, epoch, n, n, spacing, cx, cy, lo, hi, misses, nan))
end
hook.Add("Think", "gmsky_terrain_poll", T.PollGrid)

-- Once a second (server) / every 5 s (client).
function T.Status(ply)
	local g = T.grid
	if not g or not IsValid(ply) then return "" end
	local p = ply:GetPos()
	return string.format(" | grid %d: player %.0f units from its centre", g.id, math.sqrt((p.x - g.cx) ^ 2 + (p.y - g.cy) ^ 2))
end

if SERVER then
	T.stats = T.stats or { lifts = 0, holdTicks = 0 }

	-- Hold the player still at pos (no movement, no falling; the SetupMove hook below). `why`:
	-- "origin" (new origin, waiting for grid 1) or "no ground" (none under it yet). A "no ground"
	-- hold gives the player its speed back when it ends (test 17: holds stopped a player launched
	-- out of noclip in mid-air).
	function T.Hold(ply, pos, why)
		T.holding, T.holdPos, T.holdWhy, T.heldTicks = true, pos, why, 0
		T.holdVel = nil
		if IsValid(ply) then
			if why == "no ground" then T.holdVel = ply:GetVelocity() end
			ply:SetPos(pos)
			ply:SetVelocity(-ply:GetVelocity())
		end
	end

	-- Ends the hold. Player velocity is zero while held, and SetVelocity adds, so this restores it.
	local function Release(ply, why)
		T.holding = false
		if T.holdVel and IsValid(ply) then
			ply:SetVelocity(T.holdVel)
			gmsky.Log(string.format("terrain: hold released (%s) after %d ticks, speed (%.0f, %.0f, %.0f) given back",
				why, T.heldTicks, T.holdVel.x, T.holdVel.y, T.holdVel.z))
		else
			gmsky.Log(string.format("terrain: hold released (%s) after %d ticks", why, T.heldTicks))
		end
		T.holdVel = nil
	end

	-- Every tick, before the player state goes to Skyrim (server.lua). `pending` = the origin epoch
	-- Skyrim published, `anchored` = the one Garry's Mod is anchored to, `connected` = Skyrim alive.
	-- Returns true if the player was moved home (the caller then anchors there).
	function T.BeforeAnchor(ply, pending, anchored)
		if pending == 0 or pending == anchored or not T.home then return false end
		T.Hold(ply, T.home, "origin")
		T.ClearGround()
		gmsky.Log(string.format("terrain: new origin epoch %d: player moved home (%.0f, %.0f, %.0f), held until Skyrim's ground is built",
			pending, T.home.x, T.home.y, T.home.z))
		return true
	end

	-- The ground entity (one, for good; its triangles change with each grid).
	local function GroundEntity()
		if not IsValid(T.ent) then
			T.ent = ents.Create("gmsky_field")
			T.ent:SetPos(Vector(0, 0, 0))
			T.ent:Spawn()
		end
		return T.ent
	end

	-- Remove all ground (the old origin's ground is in the wrong place after a new origin).
	function T.ClearGround()
		if IsValid(T.ent) then gmsky.field.UseGrid(T.ent, nil) end
		T.ground, T.builtId = nil, nil
	end

	function T.UpdateGround(ply, anchored, connected)
		local g = T.grid
		local pos = ply:GetPos()
		-- A new grid for the origin we're anchored to: it is the ground from this tick on.
		if g and g.epoch == anchored and g.id ~= T.builtId then
			local f = gmsky.field.UseGrid(GroundEntity(), g)
			T.ground, T.builtId = g, g.id
			T.lastBuild = string.format("%d squares, %.1f ms", f.squares, f.buildMs)
			gmsky.Log(string.format("terrain: grid %d is the ground now (%s)", g.id, T.lastBuild))
		end

		local noclip = ply:GetMoveType() == MOVETYPE_NOCLIP
		if T.holding then
			if noclip then
				Release(ply, "noclip")
			else
				T.heldTicks = T.heldTicks + 1
			end
		end
		local top = T.ground and T.FootprintTop(T.ground, pos.x, pos.y)
		if not top then
			-- No Skyrim ground here (not built yet, or the player outran the grid): hold, don't fall.
			if connected and anchored ~= 0 and not noclip and not T.holding then
				T.Hold(ply, pos, "no ground")
				gmsky.Log(string.format("terrain: HOLD at (%.0f, %.0f, %.0f): no Skyrim ground under the player", pos.x, pos.y, pos.z))
			end
			if T.holding and not connected then Release(ply, "Skyrim not connected") end
			return
		end
		-- Inside the ground (just placed, a higher grid replaced a lower one, or fell through a gap):
		-- lift out, 4 above the highest point under the footprint. "Inside" is judged by the ground
		-- under the player's centre, not the footprint's highest point: next to an object (a post,
		-- a rock) the 48-unit ray grid makes a steep spike, and the highest point under the
		-- footprint popped the player on top of it up to 23 times a second (test 17, "wonky"). 12
		-- units of slack: the triangles and the bilinear height differ by a few units on curved
		-- ground, and a player standing normally must never be lifted (it would jitter every tick).
		local under = T.GridHeight(T.ground, pos.x, pos.y)
		local inside = under ~= nil and pos.z < under - 12 and not noclip
		if inside or T.holding then
			if pos.z < top + 4 then
				ply:SetPos(Vector(pos.x, pos.y, top + 4))
				T.stats.lifts = T.stats.lifts + 1
			end
			if T.holding then
				gmsky.Log(string.format("terrain: ground ready under the player (grid %d): released at z %.1f (highest ground under it %.1f)",
					T.ground.id, math.max(pos.z, top + 4), top))
				Release(ply, T.holdWhy)
			elseif pos.z < under - 40 then
				gmsky.Log(string.format("terrain: LIFT: player was %.1f below the ground at (%.0f, %.0f), put back on top", under - pos.z, pos.x, pos.y))
			end
		end
	end

	-- While held: no movement, no falling.
	hook.Add("SetupMove", "gmsky_terrain_hold", function(ply, mv)
		if not T.holding or not T.holdPos or ply:GetMoveType() == MOVETYPE_NOCLIP then return end
		mv:SetVelocity(Vector(0, 0, 0))
		mv:SetOrigin(T.holdPos)
		T.stats.holdTicks = T.stats.holdTicks + 1
	end)

	function T.GroundStatus(ply)
		if not T.ground then return T.holding and " | ground: HOLDING (no grid yet)" or "" end
		local p = ply:GetPos()
		local z = T.GridHeight(T.ground, p.x, p.y)
		local ground = ply:GetGroundEntity()
		local made = IsValid(T.ent) and T.ent.field and T.ent.field.made or 0
		local line = string.format(" | ground: grid %d (%s, %d triangles made so far), feet minus ground %s, standing on %s, lifts %d, held ticks %d%s",
			T.ground.id, T.lastBuild or "?", made, z and string.format("%.1f", p.z - z) or "- (off the grid)",
			ground == T.ent and ("Skyrim's ground") or (IsValid(ground) and ground:GetClass()) or (ground == game.GetWorld() and "the map" or "nothing"),
			T.stats.lifts, T.stats.holdTicks, T.holding and (", HOLDING (" .. tostring(T.holdWhy) .. ")") or "")
		T.stats.lifts, T.stats.holdTicks = 0, 0
		return line
	end
end
