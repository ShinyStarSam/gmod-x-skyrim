-- gmsky terrain (Phase 1 step 3, DESIGN.md). Runs on both sides: server.lua and client.lua call
-- gmsky.Include("terrain"). Garry's Mod predicts the player's movement on the client too, so both
-- sides must build the same collision, or the Garry's Mod window would stutter.
--
-- Step 3a: a test hill from a formula, to measure Garry's Mod's side before any Skyrim data:
-- the room gm_flatgrass has, whether the player walks smoothly on a PhysicsFromMesh object, and
-- how long building one takes. Console: gmsky_test_terrain (build under you), gmsky_test_terrain 0.
-- Logs: gmod_server.log / gmod_client.log.

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

-- Invisible, frozen, collision only.
local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "TestId") -- 0 = not set yet
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction, wherever it is
end

function ENT:BuildCollision()
	local verts = TestMesh()
	local t0 = SysTime()
	local ok = self:PhysicsFromMesh(verts)
	local ms = (SysTime() - t0) * 1000
	self:SetSolid(SOLID_VPHYSICS)
	self:SetMoveType(MOVETYPE_NONE)
	self:EnableCustomCollisions(true)
	self:DrawShadow(false)
	self:SetCollisionBounds(Vector(-HALF, -HALF, -1), Vector(HALF, HALF, 401))
	local phys = self:GetPhysicsObject()
	if IsValid(phys) then
		phys:EnableMotion(false)
	end
	self.gmskyBuilt = true
	gmsky.Log(string.format("terrain: built test %d: %d triangles in %.1f ms, PhysicsFromMesh=%s physics object=%s at (%.1f, %.1f, %.1f)",
		self:GetTestId(), #verts / 3, ms, tostring(ok), tostring(IsValid(phys)), self:GetPos().x, self:GetPos().y, self:GetPos().z))
end

function ENT:Initialize()
	if SERVER then self:BuildCollision() end
end

function ENT:Think()
	-- The client builds its copy as soon as the entity and its data have arrived.
	if CLIENT and not self.gmskyBuilt and self:GetTestId() ~= 0 then self:BuildCollision() end
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
	gmsky.Log(string.format("terrain: map %s room from (%.0f, %.0f, %.0f): down %.0f up %.0f +x %.0f -x %.0f +y %.0f -y %.0f (-1 = nothing hit)",
		game.GetMap(), pos.x, pos.y, pos.z, hit(Vector(0, 0, -1)), hit(Vector(0, 0, 1)),
		hit(Vector(1, 0, 0)), hit(Vector(-1, 0, 0)), hit(Vector(0, 1, 0)), hit(Vector(0, -1, 0))))
end

-- Step 3b: Skyrim's ground grid (GmskyTerrain), already in Garry's Mod units. For now it is
-- received and logged; 3c builds collision from it.
function T.PollGrid()
	local id, epoch, n, cx, cy, spacing, misses, h = gmsky.ReadTerrain(T.grid and T.grid.id or 0)
	if not epoch then return end
	local lo, hi, nan = math.huge, -math.huge, 0
	for k = 1, #h do
		local z = h[k]
		if z ~= z then nan = nan + 1 else lo = math.min(lo, z); hi = math.max(hi, z) end
	end
	T.grid = { id = id, epoch = epoch, n = n, cx = cx, cy = cy, spacing = spacing, h = h }
	gmsky.Log(string.format("terrain: grid %d from Skyrim (origin epoch %d): %dx%d points %.1f apart, centre (%.1f, %.1f), ground z %.1f..%.1f, %d misses (%d NaN)",
		id, epoch, n, n, spacing, cx, cy, lo, hi, misses, nan))
end
hook.Add("Think", "gmsky_terrain_poll", T.PollGrid)

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
		for _, e in ipairs(ents.FindByClass("gmsky_terrain")) do ent = e end
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
	concommand.Add("gmsky_test_terrain", function(ply, _, args)
		if IsValid(T.testEnt) then T.testEnt:Remove() end
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
end
