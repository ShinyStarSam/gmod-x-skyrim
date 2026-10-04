-- gmsky collision field (DESIGN.md "Phase 1 re-plan", step A). Runs on both sides (server.lua and
-- client.lua include it): Garry's Mod predicts the player's movement on the client too, so both
-- sides must answer the same.
--
-- Instead of Garry's Mod physics objects (PhysicsFromMesh: its physics engine crashes when one is
-- created near the moving player, STATUS.md "Known engine bug"), Skyrim's triangles live here in
-- Lua. An invisible entity with custom collisions and no physics object asks ENT:TestCollision
-- whenever a trace crosses it (player movement, bullets); we answer where the moving box first
-- touches a triangle. Garry's Mod's own movement (walking, sliding, step-up) does the rest.
-- Idea from SkyCraft (MIT): it feeds Skyrim's exact triangles to the other game's collision query
-- instead of making objects (SkyCraft fabric/.../world/TriCollider.java).
--
-- Step A: a test scene (console: gmsky_test_field, gmsky_test_field 0). Step B: Skyrim's ground
-- grid (terrain.lua puts each new grid into one `gmsky_field` entity with F.UseGrid).

local gmsky = gmsky
local F = gmsky.field or {}
gmsky.field = F

local CELL = 64        -- spatial grid cell for looking up triangles, Garry's Mod units
local EPS = 0.03125    -- Source's DIST_EPSILON: stop this far before the surface

-- "startpos is the bottom of the box" (Garry's Mod wiki's TestCollision example) or its centre.
-- Logged on the first calls; switchable while testing.
F.feet = F.feet or CreateConVar("gmsky_field_feet", "1", FCVAR_REPLICATED,
	"1 = TestCollision's startpos is the bottom of the moving box, 0 = its centre")

-- ---- triangle store -------------------------------------------------------------------------

-- A new, empty field. Triangles are kept as flat arrays of numbers (no Vector objects: fast).
function F.New()
	return { tris = {}, cells = {}, stamp = 0 }
end

local function CellKey(cx, cy) return cx * 65536 + cy end

-- Triangle (ax..cz) as the flat array the sweep uses: vertices, unit normal, bounds. nil if
-- degenerate.
local function MakeTri(ax, ay, az, bx, by, bz, cx, cy, cz)
	local e1x, e1y, e1z = bx - ax, by - ay, bz - az
	local e2x, e2y, e2z = cx - ax, cy - ay, cz - az
	local nx, ny, nz = e1y * e2z - e1z * e2y, e1z * e2x - e1x * e2z, e1x * e2y - e1y * e2x
	local len = math.sqrt(nx * nx + ny * ny + nz * nz)
	if len < 1e-6 then return nil end
	return {
		ax, ay, az, bx, by, bz, cx, cy, cz,
		nx / len, ny / len, nz / len,
		math.min(ax, bx, cx), math.min(ay, by, cy), math.min(az, bz, cz),
		math.max(ax, bx, cx), math.max(ay, by, cy), math.max(az, bz, cz),
		stamp = 0,
	}
end

-- Adds triangle (a, b, c), each {x, y, z}. Degenerate triangles are skipped.
function F.AddTri(f, a, b, c)
	local t = MakeTri(a[1], a[2], a[3], b[1], b[2], b[3], c[1], c[2], c[3])
	if not t then return end
	f.tris[#f.tris + 1] = t
	for cx = math.floor(t[13] / CELL), math.floor(t[16] / CELL) do
		for cy = math.floor(t[14] / CELL), math.floor(t[17] / CELL) do
			local k = CellKey(cx, cy)
			local list = f.cells[k]
			if not list then list = {}; f.cells[k] = list end
			list[#list + 1] = t
		end
	end
end

-- Quad a-b-c-d (in order around it) as two triangles.
function F.AddQuad(f, a, b, c, d)
	F.AddTri(f, a, b, c)
	F.AddTri(f, a, c, d)
end

-- Axis-aligned box from lo to hi ({x, y, z}): 12 triangles.
function F.AddBox(f, lo, hi)
	local x0, y0, z0, x1, y1, z1 = lo[1], lo[2], lo[3], hi[1], hi[2], hi[3]
	local p = {
		{ x0, y0, z0 }, { x1, y0, z0 }, { x1, y1, z0 }, { x0, y1, z0 },
		{ x0, y0, z1 }, { x1, y0, z1 }, { x1, y1, z1 }, { x0, y1, z1 },
	}
	F.AddQuad(f, p[1], p[2], p[3], p[4]) -- bottom
	F.AddQuad(f, p[5], p[6], p[7], p[8]) -- top
	F.AddQuad(f, p[1], p[2], p[6], p[5])
	F.AddQuad(f, p[2], p[3], p[7], p[6])
	F.AddQuad(f, p[3], p[4], p[8], p[7])
	F.AddQuad(f, p[4], p[1], p[5], p[8])
end

-- Upright prism with `sides` sides (a tree trunk), centre (x, y), radius r, from z0 to z1.
function F.AddPrism(f, x, y, r, z0, z1, sides)
	local ring = {}
	for i = 0, sides - 1 do
		local a = i * 2 * math.pi / sides
		ring[i] = { x + r * math.cos(a), y + r * math.sin(a) }
	end
	for i = 0, sides - 1 do
		local p, q = ring[i], ring[(i + 1) % sides]
		F.AddQuad(f, { p[1], p[2], z0 }, { q[1], q[2], z0 }, { q[1], q[2], z1 }, { p[1], p[2], z1 })
		F.AddTri(f, { x, y, z1 }, { p[1], p[2], z1 }, { q[1], q[2], z1 })
		F.AddTri(f, { x, y, z0 }, { q[1], q[2], z0 }, { p[1], p[2], z0 })
	end
end

-- ---- swept box against triangles --------------------------------------------------------------

-- Separating-axis test for a box (centre c, half size e) moving by d against triangle t, on one
-- axis (ax, ay, az; unit length). Returns enter, exit times (fractions of d), or nil if the axis
-- separates them for the whole move.
local function AxisTimes(t, cx, cy, cz, ex, ey, ez, dx, dy, dz, ax, ay, az)
	local p0 = t[1] * ax + t[2] * ay + t[3] * az
	local p1 = t[4] * ax + t[5] * ay + t[6] * az
	local p2 = t[7] * ax + t[8] * ay + t[9] * az
	local lo, hi = math.min(p0, p1, p2), math.max(p0, p1, p2)
	local r = ex * math.abs(ax) + ey * math.abs(ay) + ez * math.abs(az)
	local p = cx * ax + cy * ay + cz * az
	local s = dx * ax + dy * ay + dz * az
	if s == 0 then
		-- (a hair of tolerance: a ray running exactly along a shared edge must not slip between
		-- the two triangles on either side of it)
		if p + r < lo - 1e-4 or p - r > hi + 1e-4 then return nil end
		return -math.huge, math.huge, s
	end
	local t1, t2 = (lo - (p + r)) / s, (hi - (p - r)) / s
	if t1 > t2 then t1, t2 = t2, t1 end
	return t1, t2, s
end

-- Earliest touch of the moving box with triangle t. Returns fraction, normal x, y, z (pointing
-- back against the movement) or nil. A box already overlapping the triangle and moving further
-- into it is stopped at once (fraction 0); one moving out of it is let go.
local function SweepTri(t, cx, cy, cz, ex, ey, ez, dx, dy, dz)
	local enter, exit = -math.huge, math.huge
	local nx, ny, nz = 0, 0, 0
	local function axis(ax, ay, az)
		local len = math.sqrt(ax * ax + ay * ay + az * az)
		if len < 1e-6 then return true end
		ax, ay, az = ax / len, ay / len, az / len
		local t1, t2, s = AxisTimes(t, cx, cy, cz, ex, ey, ez, dx, dy, dz, ax, ay, az)
		if not t1 then return false end
		if t1 > enter then
			enter = t1
			if s > 0 then nx, ny, nz = -ax, -ay, -az else nx, ny, nz = ax, ay, az end
		end
		if t2 < exit then exit = t2 end
		return enter <= exit
	end
	-- Box axes, face normal, then edge x box-axis crosses.
	if not axis(1, 0, 0) or not axis(0, 1, 0) or not axis(0, 0, 1) or not axis(t[10], t[11], t[12]) then return nil end
	local e = {
		{ t[4] - t[1], t[5] - t[2], t[6] - t[3] },
		{ t[7] - t[4], t[8] - t[5], t[9] - t[6] },
		{ t[1] - t[7], t[2] - t[8], t[3] - t[9] },
	}
	for k = 1, 3 do
		local ux, uy, uz = e[k][1], e[k][2], e[k][3]
		-- (u x X), (u x Y), (u x Z)
		if not axis(0, uz, -uy) or not axis(-uz, 0, ux) or not axis(uy, -ux, 0) then return nil end
	end
	if enter > 1 or exit < 0 then return nil end
	if enter == -math.huge then return nil end -- not moving along any separating axis: no touch
	if enter < 0 then
		-- Already overlapping: the triangle faces the box's centre (a floor the box sank into is
		-- below it, not above). Stop only if moving further in; moving out or along is free.
		local fx, fy, fz = t[10], t[11], t[12]
		if (cx - t[1]) * fx + (cy - t[2]) * fy + (cz - t[3]) * fz < 0 then fx, fy, fz = -fx, -fy, -fz end
		if dx * fx + dy * fy + dz * fz >= 0 then return nil end
		return 0, fx, fy, fz
	end
	return enter, nx, ny, nz
end

F.stats = F.stats or { box = 0, ray = 0, hits = 0, ms = 0, worst = 0, tested = 0 }

-- A grid field's two triangles for square (i, j), made the first time a trace needs them;
-- false if a corner is a miss.
local function GridSquare(f, i, j)
	local g = f.grid
	local n = g.n
	local key = i * n + j
	local c = f.cache[key]
	if c ~= nil then return c end
	local h = g.h
	local za, zb = h[i * n + j + 1], h[(i + 1) * n + j + 1]
	local zc, zd = h[(i + 1) * n + j + 2], h[i * n + j + 2]
	if za == za and zb == zb and zc == zc and zd == zd then
		local xa, ya = f.x0 + i * g.spacing, f.y0 + j * g.spacing
		local xb, yb = xa + g.spacing, ya + g.spacing
		c = { MakeTri(xa, ya, za, xb, ya, zb, xb, yb, zc), MakeTri(xa, ya, za, xb, yb, zc, xa, yb, zd) }
		f.made = f.made + 2
	else
		c = false
	end
	f.cache[key] = c
	return c
end

-- One section of a sweep: earliest touch within this (short) move, or nil.
local function SweepSection(f, cx, cy, cz, ex, ey, ez, dx, dy, dz)
	-- The moving box's bounds, 0.01 wider: a trace along a square's edge also looks at the square
	-- on the other side of it.
	local lox, hix = math.min(cx, cx + dx) - ex - 0.01, math.max(cx, cx + dx) + ex + 0.01
	local loy, hiy = math.min(cy, cy + dy) - ey - 0.01, math.max(cy, cy + dy) + ey + 0.01
	local loz, hiz = math.min(cz, cz + dz) - ez - 0.01, math.max(cz, cz + dz) + ez + 0.01
	local best, bnx, bny, bnz = nil, 0, 0, 0
	local tested = 0
	local function test(t)
		if t and t[13] <= hix and t[16] >= lox and t[14] <= hiy and t[17] >= loy and t[15] <= hiz and t[18] >= loz then
			tested = tested + 1
			local frac, nx, ny, nz = SweepTri(t, cx, cy, cz, ex, ey, ez, dx, dy, dz)
			if frac and (not best or frac < best) then best, bnx, bny, bnz = frac, nx, ny, nz end
		end
	end
	if f.grid then
		-- A height grid: only the squares under the moving box.
		if hiz < f.lo.z or loz > f.hi.z then return nil end
		local sp, last = f.grid.spacing, f.grid.n - 2
		local i0, i1 = math.max(0, math.floor((lox - f.x0) / sp)), math.min(last, math.floor((hix - f.x0) / sp))
		local j0, j1 = math.max(0, math.floor((loy - f.y0) / sp)), math.min(last, math.floor((hiy - f.y0) / sp))
		for i = i0, i1 do
			for j = j0, j1 do
				local sq = GridSquare(f, i, j)
				if sq then test(sq[1]) test(sq[2]) end
			end
		end
	else
		f.stamp = f.stamp + 1
		local stamp = f.stamp
		for gx = math.floor(lox / CELL), math.floor(hix / CELL) do
			for gy = math.floor(loy / CELL), math.floor(hiy / CELL) do
				local list = f.cells[CellKey(gx, gy)]
				if list then
					for k = 1, #list do
						local t = list[k]
						if t.stamp ~= stamp then
							t.stamp = stamp
							test(t)
						end
					end
				end
			end
		end
	end
	F.stats.tested = F.stats.tested + tested
	if not best then return nil end
	return best, bnx, bny, bnz
end

-- Sweeps a box (centre c, half size e) by d through field f. Returns fraction, normal or nil.
-- Long moves are tested in SECTION-unit sections in order, so a long ray (a view trace) only
-- looks at what lies along it, not at everything in its bounding box.
local SECTION = 128
function F.Sweep(f, cx, cy, cz, ex, ey, ez, dx, dy, dz)
	local len = math.sqrt(dx * dx + dy * dy + dz * dz)
	local m = math.max(1, math.ceil(len / SECTION))
	local sx, sy, sz = dx / m, dy / m, dz / m
	for k = 0, m - 1 do
		local frac, nx, ny, nz = SweepSection(f, cx + dx * k / m, cy + dy * k / m, cz + dz * k / m, ex, ey, ez, sx, sy, sz)
		if frac then
			-- Stop DIST_EPSILON short of the surface, as Source's own traces do.
			local best = (k + frac) / m
			if len > 0 then best = math.max(0, best - EPS / len) end
			return best, nx, ny, nz
		end
	end
	return nil
end

-- ---- the entity ---------------------------------------------------------------------------------

local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "SceneId")      -- test scene (step A); 0 = none
	self:NetworkVar("Int", 1, "GridId")       -- Skyrim ground grid (step B); 0 = none
	self:NetworkVar("Vector", 0, "BoundsLo")  -- collision bounds around the entity
	self:NetworkVar("Vector", 1, "BoundsHi")
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction, wherever it is
end

-- No physics object: a box solid with custom collisions, so traces ask TestCollision.
function ENT:SetupCollision()
	self:SetSolid(SOLID_OBB)
	self:SetMoveType(MOVETYPE_NONE)
	self:EnableCustomCollisions(true)
	self:SetCollisionBounds(self:GetBoundsLo(), self:GetBoundsHi())
	self:DrawShadow(false)
	self.gmskySetUp = true
end

function ENT:Initialize()
	if SERVER then
		if self:GetSceneId() ~= 0 then self.field = F.BuildScene(self:GetSceneId(), Vector(0, 0, 0)) end
		self:SetupCollision()
	end
end

function ENT:Think()
	-- The client builds its copy as soon as the entity and its data have arrived (a grid: once
	-- the client has read that grid from shared memory too).
	if not CLIENT or not self.GetSceneId then return end
	if self:GetSceneId() ~= 0 then
		if not self.field then
			self.field = F.BuildScene(self:GetSceneId(), Vector(0, 0, 0))
			self:SetupCollision()
		end
		return
	end
	local id = self:GetGridId()
	if id ~= (self.gmskyGridId or 0) then
		local g = gmsky.terrain and gmsky.terrain.grids[id]
		if id == 0 then
			self.field, self.gmskyGridId = nil, 0
		elseif g then
			self.field, self.gmskyGridId = F.FromGrid(g), id
			self:SetupCollision()
			gmsky.Log(string.format("field: grid %d in use on this side (%d squares, %.1f ms)", id, self.field.squares, self.field.buildMs))
		end
	end
	self:SetNextClientThink(CurTime())
	return true
end

F.argLogs = F.argLogs or 0
function ENT:TestCollision(startpos, delta, isbox, extents, mask)
	local f = self.field
	if not f then return end
	local t0 = SysTime()
	-- The moving box: half size `extents`; its centre is startpos, or extents.z above it when
	-- startpos is the bottom of the box (gmsky_field_feet 1).
	local ex, ey, ez = 0, 0, 0
	if isbox then ex, ey, ez = extents.x, extents.y, extents.z end
	-- The field's triangles are relative to the entity (step C: the ground entity sits at the
	-- anchor, so a re-centre moves only the entity).
	local o = self:GetPos()
	local cz = startpos.z - o.z
	if isbox and F.feet:GetBool() then cz = cz + ez end
	local frac, nx, ny, nz = F.Sweep(f, startpos.x - o.x, startpos.y - o.y, cz, ex, ey, ez, delta.x, delta.y, delta.z)
	local ms = (SysTime() - t0) * 1000
	local s = F.stats
	if isbox then s.box = s.box + 1 else s.ray = s.ray + 1 end
	s.ms = s.ms + ms
	s.worst = math.max(s.worst, ms)
	if F.argLogs < 6 and isbox then
		F.argLogs = F.argLogs + 1
		local ply = SERVER and player.GetAll()[1] or LocalPlayer()
		local p = IsValid(ply) and ply:GetPos() or Vector()
		gmsky.Log(string.format("field: TestCollision #%d: startpos (%.1f, %.1f, %.1f) delta (%.1f, %.1f, %.1f) extents (%.1f, %.1f, %.1f) mask %s; player feet (%.1f, %.1f, %.1f); %s",
			F.argLogs, startpos.x, startpos.y, startpos.z, delta.x, delta.y, delta.z, extents.x, extents.y, extents.z,
			tostring(mask), p.x, p.y, p.z, frac and string.format("hit at %.3f, normal (%.2f, %.2f, %.2f)", frac, nx, ny, nz) or "no hit"))
	end
	if not frac then return end
	s.hits = s.hits + 1
	return {
		HitPos = startpos + delta * frac,
		Fraction = frac,
		Normal = Vector(nx, ny, nz),
	}
end

if CLIENT then
	function ENT:Draw() end -- invisible (not SetNoDraw: that would stop it being sent to the client)
end

scripted_ents.Register(ENT, "gmsky_field")

-- ---- step B: Skyrim's ground grid ----------------------------------------------------------------

-- A field for grid g (terrain.lua's grid table: n, cx, cy, spacing, heights h with NaN = miss),
-- in the grid's coordinates: relative to the anchor (protocol 7), where the ground entity sits. Only the heights are kept: a square's two triangles are made the first
-- time a trace needs them (test 20: making all 8,190 at once took 7-20 ms per grid). Squares
-- touching a miss have none. f.lo / f.hi: its bounds; f.squares: squares with ground.
function F.FromGrid(g)
	local t0 = SysTime()
	local n, sp, h = g.n, g.spacing, g.h
	local half = (n - 1) * sp / 2
	local f = { grid = g, cache = {}, made = 0, x0 = g.cx - half, y0 = g.cy - half }
	local lo, hi, squares = math.huge, -math.huge, 0
	for k = 1, #h do
		local z = h[k]
		if z == z then lo = math.min(lo, z); hi = math.max(hi, z) end
	end
	for i = 0, n - 2 do
		for j = 0, n - 2 do
			local za, zb = h[i * n + j + 1], h[(i + 1) * n + j + 1]
			local zc, zd = h[(i + 1) * n + j + 2], h[i * n + j + 2]
			if za == za and zb == zb and zc == zc and zd == zd then squares = squares + 1 end
		end
	end
	if lo > hi then lo, hi = 0, 0 end
	f.squares = squares
	f.lo = Vector(f.x0 - 1, f.y0 - 1, lo - 1)
	f.hi = Vector(f.x0 + 2 * half + 1, f.y0 + 2 * half + 1, hi + 1)
	f.buildMs = (SysTime() - t0) * 1000
	return f
end

-- Server: makes `ent` (at the anchor, terrain.lua) answer with grid g (nil = no ground). The client
-- follows through the networked grid id. Returns the field.
function F.UseGrid(ent, g)
	if not g then
		ent.field = nil
		ent:SetGridId(0)
		return nil
	end
	local f = F.FromGrid(g)
	ent.field = f
	ent:SetBoundsLo(f.lo)
	ent:SetBoundsHi(f.hi)
	ent:SetGridId(g.id)
	ent:SetupCollision()
	return f
end

-- ---- step A test scene ------------------------------------------------------------------------

local N, SPACING = 65, 25.5
local HALF = (N - 1) * SPACING / 2
F.HILL_HALF = HALF

-- The test hill (same formula as 3a, gmsky.terrain.TestHeight): 0..400 above the base.
function F.HillHeight(dx, dy)
	return 120 * (1 + math.sin(dx / 250)) + 80 * (1 + math.cos(dy / 180))
end

-- Scene 1, around `base` (the entity uses (0, 0, 0): a field's coordinates are relative to its
-- entity): the hill west of base.x, a flat floor east of it with a post, a fence, a tree trunk,
-- stairs and a 50-degree ramp.
function F.BuildScene(id, base)
	local f = F.New()
	local t0 = SysTime()
	local bx, by, bz = base.x, base.y, base.z
	-- Hill: centre (bx - HALF - 50, by).
	local hx = bx - HALF - 50
	for i = 0, N - 2 do
		for j = 0, N - 2 do
			local function p(a, b)
				local dx, dy = a * SPACING - HALF, b * SPACING - HALF
				return { hx + dx, by + dy, bz + F.HillHeight(dx, dy) }
			end
			F.AddQuad(f, p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1))
		end
	end
	-- Flat floor east of base: 900 x 900.
	F.AddQuad(f, { bx, by - 450, bz }, { bx + 900, by - 450, bz }, { bx + 900, by + 450, bz }, { bx, by + 450, bz })
	-- A post (8 x 8, 100 tall), a fence (4 thick, 200 long, 40 tall), a trunk (r 14, 150 tall, 12 sides).
	F.AddBox(f, { bx + 150, by - 154, bz }, { bx + 158, by - 146, bz + 100 })
	F.AddBox(f, { bx + 300, by - 100, bz }, { bx + 304, by + 100, bz + 40 })
	F.AddPrism(f, bx + 150, by + 150, 14, bz, bz + 150, 12)
	-- Stairs going +x: 6 steps of 12 up and 24 deep, 96 wide, then a landing.
	for k = 0, 5 do
		F.AddBox(f, { bx + 450 + k * 24, by - 250, bz }, { bx + 450 + (k + 1) * 24, by - 154, bz + (k + 1) * 12 })
	end
	F.AddBox(f, { bx + 594, by - 250, bz }, { bx + 700, by - 154, bz + 72 })
	-- A 50-degree ramp going +x (too steep to walk up: Garry's Mod walks up to ~45.6 degrees).
	local rise = 150
	local run = rise / math.tan(math.rad(50))
	F.AddQuad(f, { bx + 450, by + 150, bz }, { bx + 450 + run, by + 150, bz + rise },
		{ bx + 450 + run, by + 250, bz + rise }, { bx + 450, by + 250, bz })
	f.buildMs = (SysTime() - t0) * 1000
	gmsky.Log(string.format("field: scene %d built at (%.0f, %.0f, %.0f): %d triangles in %.1f ms", id, bx, by, bz, #f.tris, f.buildMs))
	return f
end

-- Once a second (server) / every 5 s (client): what TestCollision did.
function F.Status(ply)
	local s = F.stats
	local line = ""
	if s.box + s.ray > 0 then
		line = string.format(" | field: %d box + %d ray tests, %d hits, %d triangles tested, %.2f ms Lua (slowest %.3f ms)",
			s.box, s.ray, s.hits, s.tested, s.ms, s.worst)
	end
	if IsValid(ply) then
		local ent = F.ent
		if CLIENT then
			for _, e in ipairs(ents.FindByClass("gmsky_field")) do ent = e end
		end
		if IsValid(ent) then
			local ground = ply:GetGroundEntity()
			local on = IsValid(ground) and ground:GetClass() or (ground == game.GetWorld() and "the map" or "nothing")
			local p, b = ply:GetPos(), ent:GetPos()
			local dx, dy = p.x - (b.x - HALF - 50), p.y - b.y
			local overHill = math.abs(dx) < HALF and math.abs(dy) < HALF
			line = line .. string.format(" | field: standing on %s%s", on,
				overHill and string.format(", feet minus hill %.1f", p.z - (b.z + F.HillHeight(dx, dy))) or "")
		end
	end
	s.box, s.ray, s.hits, s.ms, s.worst, s.tested = 0, 0, 0, 0, 0, 0
	return line
end

if SERVER then
	concommand.Add("gmsky_test_field", function(ply, _, args)
		if IsValid(F.ent) then F.ent:Remove() end
		F.ent = nil
		if args[1] == "0" then
			gmsky.Log("field: test scene removed")
			return
		end
		ply = IsValid(ply) and ply or player.GetAll()[1]
		if not IsValid(ply) then return end
		local pos = ply:GetPos()
		-- Base just above the map floor under the player, so the whole scene is reachable.
		local floor = util.TraceLine({ start = pos + Vector(0, 0, 40), endpos = pos - Vector(0, 0, 40000), mask = MASK_SOLID_BRUSHONLY })
		local base = Vector(pos.x, pos.y, floor.HitPos.z + 8)
		F.sceneId = (F.sceneId or 0) + 1
		F.argLogs = 0
		local ent = ents.Create("gmsky_field")
		ent:SetPos(base)
		ent:SetSceneId(F.sceneId)
		-- Bounds (relative to the entity) around the whole scene.
		ent:SetBoundsLo(Vector(-2 * HALF - 100, -HALF - 50, -16))
		ent:SetBoundsHi(Vector(950, HALF + 50, 420))
		ent:Spawn()
		F.ent = ent
		-- Stand the player on the flat floor, between the post and the trunk.
		ply:SetPos(base + Vector(60, 0, 4))
		ply:SetVelocity(-ply:GetVelocity())
		gmsky.Log(string.format("field: test scene %d spawned at (%.0f, %.0f, %.0f); player placed on its floor", F.sceneId, base.x, base.y, base.z))
	end)
end
