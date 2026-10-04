-- gmsky terrain (Phase 1 step 3, DESIGN.md). Runs on both sides: server.lua and client.lua call
-- gmsky.Include("terrain"). Garry's Mod predicts the player's movement on the client too, so both
-- sides must build the same collision, or the Garry's Mod window would stutter.
--
-- Step 3a: a test hill from a formula (console: gmsky_test_terrain, gmsky_test_terrain 0).
-- Step 3c: the ground itself. Skyrim sends a 65 x 65 grid of its ground heights (already in Garry's
-- Mod units); each new grid becomes an invisible, frozen PhysicsFromMesh entity, so Garry's Mod's
-- own movement walks on Skyrim's ground. On a new origin the player moves to the middle of the
-- map's open space; while there is no Skyrim ground under it, it is held in place instead of
-- falling to the map floor. Logs: gmod_server.log / gmod_client.log.

local gmsky = gmsky
local T = gmsky.terrain or {}
gmsky.terrain = T

local N = 65         -- grid points per side (as Skyrim's grid will be: 65 x 65, 48 Skyrim units apart)
local SPACING = 25.5 -- Garry's Mod units (48 Skyrim units x 0.5311)
local HALF = (N - 1) * SPACING / 2

-- The test hill, relative to its centre: two waves, slopes up to about 35 degrees (Garry's Mod's
-- player walks up to ~45). Never below 0, so it sits on whatever base height we give it.
function T.TestHeight(dx, dy)
	return 120 * (1 + math.sin(dx / 250)) + 80 * (1 + math.cos(dy / 180))
end

-- Triangles for PhysicsFromMesh, in the entity's own coordinates (its origin = grid centre).
local function TestMesh()
	local h = {}
	for i = 0, N - 1 do
		h[i] = {}
		for j = 0, N - 1 do
			h[i][j] = T.TestHeight(i * SPACING - HALF, j * SPACING - HALF)
		end
	end
	local verts = {}
	local function v(i, j) return { pos = Vector(i * SPACING - HALF, j * SPACING - HALF, h[i][j]) } end
	for i = 0, N - 2 do
		for j = 0, N - 2 do
			local a, b, c, d = v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1)
			verts[#verts + 1] = a; verts[#verts + 1] = b; verts[#verts + 1] = c
			verts[#verts + 1] = a; verts[#verts + 1] = c; verts[#verts + 1] = d
		end
	end
	return verts
end

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

-- A grid is built as PIECES x PIECES separate entities of CHUNK x CHUNK squares (512 triangles).
-- Skyrim puts grid centres on a lattice one piece wide, so pieces of different grids line up
-- exactly: a piece that already exists is kept, never built again on top of itself. Tests 12 and
-- 13 crashed inside Garry's Mod's physics (vphysics.dll, from PhysicsFromMesh) exactly when a
-- grid was built over the previous grid's identical, overlapping mesh.
local CHUNK = 16
T.PIECES = 4

-- Triangles for grid g (or only piece (ci, cj) of it), relative to an entity at (cx, cy, 0).
-- Squares touching a miss are left out.
function T.GridMesh(g, ci, cj)
	local n, sp, half, h = g.n, g.spacing, (g.n - 1) * g.spacing / 2, g.h
	local verts, lo, hi = {}, math.huge, -math.huge
	local i0, i1, j0, j1 = 0, n - 2, 0, n - 2
	if ci then
		i0, j0 = ci * CHUNK, cj * CHUNK
		i1, j1 = math.min(n - 2, i0 + CHUNK - 1), math.min(n - 2, j0 + CHUNK - 1)
	end
	for i = i0, i1 do
		for j = j0, j1 do
			local za, zb = h[i * n + j + 1], h[(i + 1) * n + j + 1]
			local zc, zd = h[(i + 1) * n + j + 2], h[i * n + j + 2]
			if za == za and zb == zb and zc == zc and zd == zd then
				local x0, x1, y0, y1 = i * sp - half, (i + 1) * sp - half, j * sp - half, (j + 1) * sp - half
				local a, b = { pos = Vector(x0, y0, za) }, { pos = Vector(x1, y0, zb) }
				local c, d = { pos = Vector(x1, y1, zc) }, { pos = Vector(x0, y1, zd) }
				verts[#verts + 1] = a; verts[#verts + 1] = b; verts[#verts + 1] = c
				verts[#verts + 1] = a; verts[#verts + 1] = c; verts[#verts + 1] = d
				lo = math.min(lo, za, zb, zc, zd)
				hi = math.max(hi, za, zb, zc, zd)
			end
		end
	end
	return verts, lo, hi, half
end

-- For the build log: is the player over this mesh (inside its x/y box), how far from its middle,
-- and is it standing on something. Server: the only player; client: the local one.
function T.PlayerAgainst(ent, verts)
	local ply = SERVER and player.GetAll()[1] or LocalPlayer()
	if not IsValid(ply) or #verts == 0 then return "" end
	local o, p = ent:GetPos(), ply:GetPos()
	local x0, x1, y0, y1 = math.huge, -math.huge, math.huge, -math.huge
	for k = 1, #verts do
		local v = verts[k].pos
		x0, x1, y0, y1 = math.min(x0, v.x), math.max(x1, v.x), math.min(y0, v.y), math.max(y1, v.y)
	end
	local cx, cy = o.x + (x0 + x1) / 2, o.y + (y0 + y1) / 2
	local over = p.x >= o.x + x0 - 16 and p.x <= o.x + x1 + 16 and p.y >= o.y + y0 - 16 and p.y <= o.y + y1 + 16
	return string.format("; player %s it, %.0f units from its middle, %s", over and "OVER" or "not over",
		math.sqrt((p.x - cx) ^ 2 + (p.y - cy) ^ 2), ply:IsOnGround() and "on the ground" or "in the air")
end

-- Invisible, frozen, collision only.
local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "TestId") -- test hill (3a); 0 = not one
	self:NetworkVar("Int", 1, "GridId") -- Skyrim grid (3c); 0 = not one
	self:NetworkVar("Int", 2, "Piece") -- which piece of the grid: ci * PIECES + cj + 1; 0 = all of it
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction, wherever it is
end

function ENT:BuildCollision()
	local verts, lo, hi, half, what
	if self:GetTestId() ~= 0 then
		verts, lo, hi, half, what = TestMesh(), -1, 401, HALF, "test " .. self:GetTestId()
	else
		local g = T.grids[self:GetGridId()]
		if not g then return end -- client: this grid hasn't arrived here yet
		local piece = self:GetPiece()
		local ci, cj = nil, nil
		if piece > 0 then ci, cj = math.floor((piece - 1) / T.PIECES), (piece - 1) % T.PIECES end
		verts, lo, hi, half = T.GridMesh(g, ci, cj)
		what = "Skyrim grid " .. g.id .. (piece > 0 and (" piece " .. piece) or "")
		if #verts == 0 then
			gmsky.Log("terrain: " .. what .. " has no ground at all (every ray missed)")
			self.gmskyBuilt = true
			return
		end
	end
	-- Logged before the call: if Garry's Mod's physics crashes inside it, the log names the piece.
	-- Also: how many meshes this side has built so far (the crash is a full 16-bit list inside the
	-- physics, which could fill up over many builds), and where the player is against this mesh.
	T.builds, T.buildTris = (T.builds or 0) + 1, (T.buildTris or 0) + #verts / 3
	gmsky.Log(string.format("terrain: building %s: %d triangles (build %d on this side, %d triangles so far)%s...",
		what, #verts / 3, T.builds, T.buildTris, T.PlayerAgainst(self, verts)))
	local t0 = SysTime()
	local ok = self:PhysicsFromMesh(verts)
	local ms = (SysTime() - t0) * 1000
	self:SetSolid(SOLID_VPHYSICS)
	self:SetMoveType(MOVETYPE_NONE)
	self:EnableCustomCollisions(true)
	self:DrawShadow(false)
	-- Bounds of the whole grid around the entity (a piece lies somewhere inside them).
	self:SetCollisionBounds(Vector(-half, -half, lo - 1), Vector(half, half, hi + 1))
	local phys = self:GetPhysicsObject()
	if IsValid(phys) then
		phys:EnableMotion(false)
	end
	self.gmskyBuilt = true
	self.gmskyBuildMs = ms
	if not ok or not IsValid(phys) or self:GetTestId() ~= 0 or T.replay then
		gmsky.Log(string.format("terrain: built %s: %d triangles in %.1f ms, PhysicsFromMesh=%s physics object=%s solid=%d flags=%d at (%.1f, %.1f, %.1f), z %.1f..%.1f",
			what, #verts / 3, ms, tostring(ok), tostring(IsValid(phys)), self:GetSolid(), self:GetSolidFlags(),
			self:GetPos().x, self:GetPos().y, self:GetPos().z, lo, hi))
	end
end

function ENT:Initialize()
	if SERVER then self:BuildCollision() end
end

function ENT:Think()
	-- The client builds its copy as soon as the entity and its data have arrived.
	if CLIENT and not self.gmskyBuilt and (self:GetTestId() ~= 0 or self:GetGridId() ~= 0) then self:BuildCollision() end
end

if CLIENT then
	function ENT:Draw() end -- invisible (not SetNoDraw: that would stop it being sent to the client)
end

scripted_ents.Register(ENT, "gmsky_terrain")

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

-- Replay (crash investigation): a grid recorded in a test (logs/terrain/grid_<id>.bin, saved
-- under build/logs-ingame-test-N/terrain), loaded on this side as if Skyrim had just sent it.
-- Returns the grid, or nil after logging why not.
T.REPLAY_DIR = "build/logs-ingame-test-13/terrain"
function T.ReplayLoad(id, dir)
	local path = (dir or T.REPLAY_DIR) .. "/grid_" .. id .. ".bin"
	local gid, epoch, n, cx, cy, spacing, misses, h = gmsky.ReadGridFile(path)
	if not gid then
		gmsky.Log("terrain: REPLAY: can't load " .. path .. ": " .. tostring(epoch))
		return nil
	end
	local g = { id = gid, epoch = epoch, n = n, cx = cx, cy = cy, spacing = spacing, h = h, replay = true }
	T.grids[gid] = g
	if SERVER then T.grid = g end
	gmsky.Log(string.format("terrain: REPLAY: loaded %s: grid %d, %dx%d points %.1f apart, centre (%.1f, %.1f), %d misses",
		path, gid, n, n, spacing, cx, cy, misses))
	return g
end

local function GridStatus(ply)
	local g = T.grid
	if not g or not IsValid(ply) then return "" end
	local p = ply:GetPos()
	return string.format(" | grid %d: player %.0f units from its centre", g.id, math.sqrt((p.x - g.cx) ^ 2 + (p.y - g.cy) ^ 2))
end

-- Once a second (server) / every 5 s (client): where the player is against the test hill.
function T.Status(ply)
	local ent = T.testEnt
	if CLIENT then
		for _, e in ipairs(ents.FindByClass("gmsky_terrain")) do
			if e:GetTestId() ~= 0 then ent = e end
		end
	end
	if not IsValid(ent) or not IsValid(ply) then return GridStatus(ply) end
	local p = ply:GetPos()
	local c = ent:GetPos()
	local dx, dy = p.x - c.x, p.y - c.y
	local inside = math.abs(dx) < HALF and math.abs(dy) < HALF
	local ground = ply:GetGroundEntity()
	return string.format(" | terrain test %d: %s, feet minus hill %s, standing on %s", ent:GetTestId(),
		inside and "over it" or "off it", inside and string.format("%.1f", p.z - (c.z + T.TestHeight(dx, dy))) or "-",
		ground == ent and "the hill" or (IsValid(ground) and ground:GetClass()) or (ground == game.GetWorld() and "the map" or "nothing"))
		.. GridStatus(ply)
end

if SERVER then
	T.stats = T.stats or { lifts = 0, holdTicks = 0 }

	-- Every tick, before the player state goes to Skyrim (server.lua). `pending` = the origin epoch
	-- Skyrim published, `anchored` = the one Garry's Mod is anchored to, `connected` = Skyrim alive.
	-- Returns true if the player was moved home (the caller then anchors there).
	function T.BeforeAnchor(ply, pending, anchored)
		if pending == 0 or pending == anchored or not T.home then return false end
		ply:SetPos(T.home)
		ply:SetVelocity(-ply:GetVelocity())
		T.holding, T.holdPos = true, T.home
		T.ClearGround()
		gmsky.Log(string.format("terrain: new origin epoch %d: player moved home (%.0f, %.0f, %.0f), held until Skyrim's ground is built",
			pending, T.home.x, T.home.y, T.home.z))
		return true
	end

	-- Remove all ground (the old origin's ground is in the wrong place after a new origin).
	function T.ClearGround()
		for _, e in pairs(T.pieces or {}) do if IsValid(e) then e:Remove() end end
		T.pieces, T.buildQueue, T.ground, T.builtId, T.buildId, T.lattice = {}, nil, nil, nil, nil, nil
	end

	-- A piece's place on the lattice, as a key. The lattice offset comes from the first grid after
	-- an origin; every later piece corner is that plus whole piece sizes (rounding only absorbs
	-- float noise, far below a piece's ~408 units).
	-- 0 = rebuild every piece of every grid, like tests 12 and 13 (crash investigation).
	T.reuse = T.reuse or CreateConVar("gmsky_reuse_pieces", "1", 0,
		"1 = keep ground pieces a new grid shares with the old one, 0 = rebuild all 16 every grid")
	local function PieceKey(g, ci, cj)
		local size = CHUNK * g.spacing
		if not T.reuse:GetBool() then
			local half = (g.n - 1) * g.spacing / 2
			return "g" .. g.id .. ":" .. ci .. ":" .. cj, g.cx - half + (ci + 0.5) * size, g.cy - half + (cj + 0.5) * size
		end
		local half = (g.n - 1) * g.spacing / 2
		local x0, y0 = g.cx - half + ci * size, g.cy - half + cj * size
		if not T.lattice then T.lattice = { x = x0, y = y0 } end
		local fx, fy = (x0 - T.lattice.x) / size, (y0 - T.lattice.y) / size
		local kx, ky = math.floor(fx + 0.5), math.floor(fy + 0.5)
		-- Off the lattice by more than 1 unit (e.g. grids recorded before Skyrim snapped its centres,
		-- test 13): a piece of its own, never mistaken for one already built somewhere else.
		if math.abs(fx - kx) * size > 1 or math.abs(fy - ky) * size > 1 then
			return "g" .. g.id .. ":" .. ci .. ":" .. cj, x0 + size / 2, y0 + size / 2
		end
		return kx .. ":" .. ky, x0 + size / 2, y0 + size / 2
	end

	function T.UpdateGround(ply, anchored, connected)
		-- Replaying recorded grids without Skyrim: behave as if Skyrim were connected and anchored.
		if T.replay then anchored, connected = T.replay.epoch, true end
		T.pieces = T.pieces or {}
		local g = T.grid
		local pos = ply:GetPos()
		-- A new grid for the origin we're anchored to: queue only the pieces we don't have yet,
		-- nearest to the player first.
		if g and g.epoch == anchored and g.id ~= T.buildId and g.id ~= T.builtId then
			T.buildId, T.buildGrid, T.buildQueue, T.buildStart, T.buildMs, T.buildWorst = g.id, g, {}, SysTime(), 0, 0
			T.buildKeep = {}
			for ci = 0, T.PIECES - 1 do
				for cj = 0, T.PIECES - 1 do
					local key, x, y = PieceKey(g, ci, cj)
					T.buildKeep[key] = true
					if not IsValid(T.pieces[key]) then
						T.buildQueue[#T.buildQueue + 1] = { key = key, piece = ci * T.PIECES + cj + 1, d = (x - pos.x) ^ 2 + (y - pos.y) ^ 2 }
					end
				end
			end
			-- Nearest first; "far" (gmsky_replay order far, crash investigation) builds the farthest first.
			if T.buildOrder == "far" then
				table.sort(T.buildQueue, function(a, b) return a.d > b.d end)
			else
				table.sort(T.buildQueue, function(a, b) return a.d < b.d end)
			end
			T.buildNew = #T.buildQueue
		end
		-- One piece per tick.
		if T.buildQueue and #T.buildQueue > 0 then
			local item = table.remove(T.buildQueue, 1)
			local ent = ents.Create("gmsky_terrain")
			ent:SetPos(Vector(T.buildGrid.cx, T.buildGrid.cy, 0))
			ent:SetGridId(T.buildGrid.id)
			ent:SetPiece(item.piece)
			ent:Spawn()
			T.pieces[item.key] = ent
			T.buildMs = T.buildMs + (ent.gmskyBuildMs or 0)
			T.buildWorst = math.max(T.buildWorst, ent.gmskyBuildMs or 0)
		end
		if T.buildQueue and #T.buildQueue == 0 then
			-- All pieces of the new grid exist: it is the ground now; drop pieces outside it.
			local removed = 0
			for key, e in pairs(T.pieces) do
				if not T.buildKeep[key] then
					if IsValid(e) then e:Remove() end
					T.pieces[key] = nil
					removed = removed + 1
				end
			end
			T.ground, T.builtId, T.buildQueue = T.buildGrid, T.buildGrid.id, nil
			T.lastBuild = string.format("%d new pieces, %d kept, %d removed, %.1f ms in all, slowest %.1f ms, %.0f ms from arrival",
				T.buildNew, T.PIECES * T.PIECES - T.buildNew, removed, T.buildMs, T.buildWorst, (SysTime() - T.buildStart) * 1000)
			gmsky.Log(string.format("terrain: grid %d is the ground now (%s)", T.builtId, T.lastBuild))
		end

		local noclip = ply:GetMoveType() == MOVETYPE_NOCLIP
		local top = T.ground and T.FootprintTop(T.ground, pos.x, pos.y)
		if not top then
			-- No Skyrim ground here (not built yet, or the player outran the grid): hold, don't fall.
			if connected and anchored ~= 0 and not noclip and not T.holding then
				T.holding, T.holdPos = true, pos
				gmsky.Log(string.format("terrain: HOLD at (%.0f, %.0f, %.0f): no Skyrim ground under the player", pos.x, pos.y, pos.z))
			end
			if T.holding and (not connected or noclip) then
				T.holding = false
				gmsky.Log("terrain: hold released (" .. (noclip and "noclip" or "Skyrim not connected") .. ")")
			end
			return
		end
		-- Inside the ground (just placed, a higher grid replaced a lower one, or fell through a gap):
		-- lift out, 4 above the highest point under the footprint. 12 units of slack: the triangles
		-- and the bilinear height differ by a few units on curved ground, and a player standing
		-- normally must never be lifted (it would jitter every tick).
		local inside = pos.z < top - 12 and not noclip
		if inside or T.holding then
			if pos.z < top + 4 then
				ply:SetPos(Vector(pos.x, pos.y, top + 4))
				T.stats.lifts = T.stats.lifts + 1
			end
			if T.holding then
				gmsky.Log(string.format("terrain: ground ready under the player (grid %d): released at z %.1f (highest ground under it %.1f)",
					T.ground.id, math.max(pos.z, top + 4), top))
				T.holding = false
			elseif pos.z < top - 40 then
				gmsky.Log(string.format("terrain: LIFT: player was %.1f below the ground at (%.0f, %.0f), put back on top", top - pos.z, pos.x, pos.y))
			end
		end
	end

	-- While held: no movement, no falling.
	hook.Add("SetupMove", "gmsky_terrain_hold", function(ply, mv)
		if not T.holding or not T.holdPos then return end
		mv:SetVelocity(Vector(0, 0, 0))
		mv:SetOrigin(T.holdPos)
		T.stats.holdTicks = T.stats.holdTicks + 1
	end)

	function T.GroundStatus(ply)
		if not T.ground then return T.holding and " | ground: HOLDING (no grid yet)" or "" end
		local p = ply:GetPos()
		local z = T.GridHeight(T.ground, p.x, p.y)
		local ground = ply:GetGroundEntity()
		local onGround = IsValid(ground) and ground:GetClass() == "gmsky_terrain" and ground:GetTestId() == 0
		local line = string.format(" | ground: grid %d (%s), feet minus ground %s, standing on %s, lifts %d, held ticks %d%s",
			T.ground.id, T.lastBuild or "?", z and string.format("%.1f", p.z - z) or "- (off the grid)",
			onGround and ("Skyrim's ground (grid " .. ground:GetGridId() .. ")") or (IsValid(ground) and ground:GetClass()) or (ground == game.GetWorld() and "the map" or "nothing"),
			T.stats.lifts, T.stats.holdTicks, T.holding and ", HOLDING" or "")
		T.stats.lifts, T.stats.holdTicks = 0, 0
		return line
	end

	concommand.Add("gmsky_test_terrain", function(ply, _, args)
		-- "overlap": build a second hill exactly on top of the existing one, WITHOUT removing it.
		-- Test 13's hypothesis says this crashes Garry's Mod (an experiment; expect a crash).
		if args[1] == "overlap" then
			if not IsValid(T.testEnt) then gmsky.Log("terrain: overlap test needs a hill first (gmsky_test_terrain)") return end
			local ent = ents.Create("gmsky_terrain")
			ent:SetPos(T.testEnt:GetPos())
			T.testId = (T.testId or 0) + 1
			ent:SetTestId(T.testId)
			gmsky.Log(string.format("terrain: OVERLAP TEST: building test %d on top of test %d...", T.testId, T.testEnt:GetTestId()))
			ent:Spawn()
			gmsky.Log("terrain: OVERLAP TEST: survived (hypothesis wrong)")
			T.overlapEnt = ent
			return
		end
		if IsValid(T.testEnt) then T.testEnt:Remove() end
		if IsValid(T.overlapEnt) then T.overlapEnt:Remove() end
		T.testEnt = nil
		if args[1] == "0" then
			gmsky.Log("terrain: test removed")
			return
		end
		ply = IsValid(ply) and ply or player.GetAll()[1]
		if not IsValid(ply) then return end
		local pos = ply:GetPos()
		T.MeasureRoom(pos + Vector(0, 0, 40))
		-- Base just above the floor under the player, so the whole hill is reachable.
		local floor = util.TraceLine({ start = pos + Vector(0, 0, 40), endpos = pos - Vector(0, 0, 40000), mask = MASK_SOLID_BRUSHONLY })
		local base = Vector(pos.x, pos.y, floor.HitPos.z + 8)
		T.testId = (T.testId or 0) + 1
		local ent = ents.Create("gmsky_terrain")
		ent:SetPos(base)
		ent:SetTestId(T.testId)
		ent:Spawn()
		T.testEnt = ent
		-- Stand the player above the highest point under its whole 32 x 32 footprint (test 10: the
		-- centre point alone left the hull's edges inside the slope and the player stuck).
		local top = 0
		for x = -16, 16, 8 do
			for y = -16, 16, 8 do top = math.max(top, T.TestHeight(x, y)) end
		end
		ply:SetPos(base + Vector(0, 0, top + 4))
		ply:SetVelocity(-ply:GetVelocity())
		gmsky.Log(string.format("terrain: test %d spawned, base z=%.1f, highest point under the player %.1f above it; player placed 4 above that",
			T.testId, base.z, top))
	end)

	-- Crash investigation (STATUS.md): replay grids recorded in a real test, Garry's Mod alone.
	--   gmsky_replay <id> [dir]           load grid_<id>.bin and build it like a grid from Skyrim
	--                                     (dir: project-relative, default build/logs-ingame-test-13/terrain)
	--   gmsky_replay order near|far       build pieces nearest-first (normal) or farthest-first
	--   gmsky_replay piece <id> <k> here|away   build ONE extra copy of piece k of grid <id>, in its
	--                                     own place (here) or 4,000 units to the side (away)
	--   gmsky_replay repeat <id> <k> <count>    build <count> copies of that piece away, one per tick
	--   gmsky_replay clear                remove all ground and copies; leave replay mode
	local function Load(id, dir)
		local g = T.ReplayLoad(id, dir)
		if not g then return nil end
		local ply = player.GetAll()[1]
		if IsValid(ply) then ply:SendLua(string.format("gmsky.terrain.ReplayLoad(%d, %q)", id, dir or T.REPLAY_DIR)) end
		return g
	end
	local function Copy(g, k, offset)
		local ent = ents.Create("gmsky_terrain")
		ent:SetPos(Vector(g.cx + offset.x, g.cy + offset.y, 0))
		ent:SetGridId(g.id)
		ent:SetPiece(k)
		ent:Spawn()
		T.replayCopies = T.replayCopies or {}
		T.replayCopies[#T.replayCopies + 1] = ent
		return ent
	end
	concommand.Add("gmsky_replay", function(ply, _, args)
		ply = IsValid(ply) and ply or player.GetAll()[1]
		local what = args[1]
		if what == "clear" then
			T.ClearGround()
			for _, e in ipairs(T.replayCopies or {}) do if IsValid(e) then e:Remove() end end
			hook.Remove("Tick", "gmsky_replay_repeat")
			T.replayCopies, T.replay, T.grid, T.holding = nil, nil, nil, false
			gmsky.Log("terrain: REPLAY: cleared, replay mode off")
		elseif what == "hold" then
			-- Put the player at an exact spot and hold it there (test 14 crashed with the player held
			-- at 843.2 -167.4 58.0). "hold off" lets go.
			local x, y, z = tonumber(args[2]), tonumber(args[3]), tonumber(args[4])
			if args[2] == "off" or not (x and y and z) then
				T.holding = false
				gmsky.Log("terrain: REPLAY: hold off")
				return
			end
			T.holding, T.holdPos = true, Vector(x, y, z)
			if IsValid(ply) then ply:SetPos(T.holdPos) ply:SetVelocity(-ply:GetVelocity()) end
			gmsky.Log(string.format("terrain: REPLAY: player held at (%.1f, %.1f, %.1f)", x, y, z))
		elseif what == "order" then
			T.buildOrder = args[2] == "far" and "far" or "near"
			gmsky.Log("terrain: REPLAY: pieces are built " .. (T.buildOrder == "far" and "farthest" or "nearest") .. " first")
		elseif what == "piece" or what == "repeat" then
			local id, k = tonumber(args[2]), tonumber(args[3])
			if not id or not k or k < 1 or k > T.PIECES * T.PIECES then
				gmsky.Log("terrain: REPLAY: usage: gmsky_replay piece <grid id> <piece 1-16> here|away / repeat <grid id> <piece> <count>")
				return
			end
			local g = T.grids[id] or Load(id)
			if not g then return end
			T.replay = T.replay or { epoch = g.epoch }
			if what == "piece" then
				local away = args[4] ~= "here"
				gmsky.Log(string.format("terrain: REPLAY: one extra copy of grid %d piece %d, %s", id, k,
					away and "4,000 units to the side" or "in its own place"))
				Copy(g, k, away and Vector(4000, 0, 0) or Vector(0, 0, 0))
				gmsky.Log("terrain: REPLAY: extra copy survived")
			else
				local count, done = math.max(1, tonumber(args[4]) or 10), 0
				gmsky.Log(string.format("terrain: REPLAY: %d copies of grid %d piece %d, away, one per tick", count, id, k))
				hook.Add("Tick", "gmsky_replay_repeat", function()
					-- 10 per row, 900 apart (a piece is ~408 wide): never overlapping, within the map.
					Copy(g, k, Vector(4000 + (done % 10) * 900, (math.floor(done / 10) - 5) * 900, 0))
					done = done + 1
					if done >= count then
						hook.Remove("Tick", "gmsky_replay_repeat")
						gmsky.Log(string.format("terrain: REPLAY: all %d copies built, survived", count))
					end
				end)
			end
		elseif tonumber(what) then
			local first = not T.replay
			local g = Load(tonumber(what), args[2])
			if not g then return end
			T.replay = { epoch = g.epoch }
			if first and IsValid(ply) then
				-- Like a new origin: no old ground; the player waits above the grid's middle until built.
				T.ClearGround()
				local top = T.FootprintTop(g, g.cx, g.cy) or 0
				T.holding, T.holdPos = true, Vector(g.cx, g.cy, top + 40)
				ply:SetPos(T.holdPos)
				ply:SetVelocity(-ply:GetVelocity())
				gmsky.Log(string.format("terrain: REPLAY: player held at (%.0f, %.0f, %.0f) until grid %d is built",
					g.cx, g.cy, top + 40, g.id))
			end
		else
			gmsky.Log("terrain: REPLAY: usage: gmsky_replay <grid id> [dir] | order near|far | piece <id> <k> here|away | repeat <id> <k> <count> | clear")
		end
	end)
end
