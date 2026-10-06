-- gmsky physics ground (DESIGN.md "One collision system"). Runs on both sides (server.lua and
-- client.lua include it).
--
-- Step S1c, the crash experiment: `gmsky_test_physground <rebuild|chunks> <skyrim|flat>` (`off`
-- stops it). Tests 12-17 and 101-107 crashed Garry's Mod's physics (vphysics.dll) when a physics
-- mesh (PhysicsFromMesh) of Skyrim's triangles was built where the player stood.
-- - rebuild: test 107 exactly: every half second a new mesh around the player's feet, the old
--   one's physics destroyed first (never two meshes for the same place at once).
-- - chunks: a fixed lattice of big chunk columns lined up with Skyrim's regions
--   (gmsky_physground_chunk regions wide). A chunk is built once, when the player comes within
--   one region of it and all its regions have arrived from Skyrim, so normally *beside* the
--   player, never under it; removed when the player is more than 2 chunks away. Every triangle is
--   cut exactly at the chunk's faces (test 109: whole triangles reached ~170 units past the box),
--   so a chunk's mesh never leaves its box and chunks never share a piece.
-- - skyrim: Skyrim's real triangles (the module's store, gmsky.TrisInBox); flat: 2 triangles over
--   the same box at the height of the player's feet when the experiment started; grid: the same
--   plane as an even grid of gmsky_physground_grid triangles (B: is it the number of triangles or
--   their shape?).
-- Every mesh uses InfMap2's chunk setup (https://codeberg.org/googer/infmap2,
-- lua/entities/inf_chunk.lua and lua/infmap2/ish/physics.lua). InfMap2 has no licence file: its
-- setup is described in DESIGN.md and written here from that description; no code is copied.
-- The client builds its copy too, from the same store (one module per process). A log line goes
-- out before every setup call (lines are flushed one by one), so a crash names the call.

local gmsky = gmsky
local G = gmsky.physground or {}
gmsky.physground = G

local HALF_XY, HALF_Z = 384, 256  -- rebuild: the box around the player's feet (768 x 768 x 512)
local MAX_TRIS = 127474           -- per mesh: the user's figure for Source models (not sure it binds physics meshes)
local EVERY = 0.5                 -- rebuild: seconds between builds
local CHUNK_EVERY = 0.25          -- chunks: seconds between looks at which chunks are needed
local LEAD = 1                    -- chunks: built when the player is within this many regions of one
local BELOW, ABOVE = 3, 2         -- regions Skyrim collects below / above the player's (Collision.cpp kBelow / kAbove)
local REMOVE_AT = 2               -- chunks: removed when more than this many chunks from the player's
local REBUILD, CHUNKS = 1, 2
local SKYRIM, FLAT, GRID = 1, 2, 3
local MODES = { [REBUILD] = "rebuild", [CHUNKS] = "chunks" }
local SOURCES = { [SKYRIM] = "skyrim", [FLAT] = "flat", [GRID] = "grid" }

G.side = G.side or { builds = 0, tris = 0, worst = 0 }

local function ThePlayer()
	if SERVER then return player.GetAll()[1] end
	return LocalPlayer()
end

-- Where the player is against a test mesh entity and its box, for the log.
local function PlayerLine(ent, lo, hi)
	local ply = ThePlayer()
	if not IsValid(ply) then return "no player" end
	local o = ent:GetPos()
	local p, v = ply:GetPos() - o, ply:GetVelocity()
	local ground = ply:GetGroundEntity()
	local on = IsValid(ground) and ground:GetClass() or (ground == game.GetWorld() and "the map" or "nothing")
	local dx = math.max(lo.x - p.x, 0, p.x - hi.x)
	local dy = math.max(lo.y - p.y, 0, p.y - hi.y)
	local where = (dx == 0 and dy == 0) and "player INSIDE the box: UNDER THE PLAYER"
		or string.format("player %.0f units outside the box", math.max(dx, dy))
	return string.format("%s; player feet (%.1f, %.1f, %.1f) relative to the anchor, speed %.0f, %s, on %s",
		where, p.x, p.y, p.z, v:Length(), ply:IsOnGround() and "on the ground" or "in the air", on)
end

-- Rebuild mode: never two meshes at once. Destroys the physics of every older test mesh on this
-- side, first. Returns how many were destroyed, or nil if a newer mesh already exists.
local function ClearOlder(self)
	local destroyed = 0
	for _, other in ipairs(ents.FindByClass("gmsky_physground_test")) do
		if other ~= self and IsValid(other) and other:GetMode() == REBUILD then
			if other:GetBuild() > self:GetBuild() and other.gmskyBuilt then return nil end
			if other:GetBuild() < self:GetBuild() and IsValid(other:GetPhysicsObject()) then
				other:PhysicsDestroy()
				other:SetSolid(SOLID_NONE)
				destroyed = destroyed + 1
			end
		end
	end
	return destroyed
end

-- How many test meshes on this side have a physics object.
local function MeshesWithPhysics()
	local n = 0
	for _, e in ipairs(ents.FindByClass("gmsky_physground_test")) do
		if IsValid(e) and IsValid(e:GetPhysicsObject()) then n = n + 1 end
	end
	return n
end

-- The vertices PhysicsFromMesh wants, relative to the entity (which sits at the anchor), their
-- triangle count, and the mesh's own bounds (triangles can stick out of the box).
local function Verts(self, lo, hi)
	local verts, mlo, mhi = {}, Vector(math.huge, math.huge, math.huge), Vector(-math.huge, -math.huge, -math.huge)
	local function Add(x, y, z)
		verts[#verts + 1] = { pos = Vector(x, y, z) }
		mlo.x, mlo.y, mlo.z = math.min(mlo.x, x), math.min(mlo.y, y), math.min(mlo.z, z)
		mhi.x, mhi.y, mhi.z = math.max(mhi.x, x), math.max(mhi.y, y), math.max(mhi.z, z)
	end
	if self:GetSource() == FLAT then
		local z = self:GetFlatZ()
		Add(lo.x, lo.y, z) Add(hi.x, lo.y, z) Add(hi.x, hi.y, z)
		Add(lo.x, lo.y, z) Add(hi.x, hi.y, z) Add(lo.x, hi.y, z)
		return verts, 2, mlo, mhi, "flat: 2 even triangles"
	end
	if self:GetSource() == GRID then
		-- n x n squares, 2 triangles each, all the same shape (no thin ones).
		local z, n = self:GetFlatZ(), self:GetGridN()
		local sx, sy = (hi.x - lo.x) / n, (hi.y - lo.y) / n
		for i = 0, n - 1 do
			for j = 0, n - 1 do
				local x0, y0 = lo.x + i * sx, lo.y + j * sy
				local x1, y1 = x0 + sx, y0 + sy
				Add(x0, y0, z) Add(x1, y0, z) Add(x1, y1, z)
				Add(x0, y0, z) Add(x1, y1, z) Add(x0, y1, z)
			end
		end
		return verts, n * n * 2, mlo, mhi, string.format("grid: %d x %d squares of %.1f x %.1f units, 2 even triangles each, 0 thin", n, n, sx, sy)
	end
	local flat, n, cut, thin, shortest, angle = gmsky.TrisInBox(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, MAX_TRIS + 1, self:GetMode() == CHUNKS)
	if n > MAX_TRIS then return nil, n end
	for i = 1, n * 9, 3 do Add(flat[i], flat[i + 1], flat[i + 2]) end
	local shape = string.format("%d triangles cut at the box's faces; %d thin (a corner under 1 degree or an edge under 0.5), shortest edge %.3f, smallest corner %.3f degrees",
		cut, thin, shortest, angle)
	return verts, n, mlo, mhi, shape
end

-- ---- the test entity ---------------------------------------------------------------------------

local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "Mode")      -- REBUILD or CHUNKS
	self:NetworkVar("Int", 1, "Build")     -- the server's build number
	self:NetworkVar("Int", 2, "Source")    -- SKYRIM or FLAT
	self:NetworkVar("Int", 3, "ChunkX")    -- chunks: the chunk's place on the lattice
	self:NetworkVar("Int", 4, "ChunkY")
	self:NetworkVar("Int", 5, "GridN")     -- grid: squares per side
	self:NetworkVar("Float", 0, "FlatZ")   -- flat: the plane's height, relative to the anchor
	self:NetworkVar("Vector", 0, "BoxLo")  -- the box the triangles were taken from, relative to the anchor
	self:NetworkVar("Vector", 1, "BoxHi")
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction
end

function ENT:GmskyName()
	local name = string.format("build %d (%s %s", self:GetBuild(), MODES[self:GetMode()] or "?", SOURCES[self:GetSource()] or "?")
	if self:GetMode() == CHUNKS then name = name .. string.format(", chunk %d %d", self:GetChunkX(), self:GetChunkY()) end
	return name .. ")"
end

function ENT:BuildMesh()
	self.gmskyBuilt = true
	local lo, hi, name = self:GetBoxLo(), self:GetBoxHi(), self:GmskyName()
	local destroyed = 0
	if self:GetMode() == REBUILD then
		destroyed = ClearOlder(self)
		if not destroyed then
			gmsky.Log(string.format("physground: %s skipped on this side: a newer mesh already exists", name))
			return
		end
	end
	local t0 = SysTime()
	local verts, n, mlo, mhi, shape = Verts(self, lo, hi)
	local s = G.side
	self.gmskyTris = n
	if not verts then
		gmsky.Log(string.format("physground: %s: %d triangles, MORE than the cap of %d: NOT built; %s", name, n, MAX_TRIS, PlayerLine(self, lo, hi)))
		return
	end
	if n == 0 then
		gmsky.Log(string.format("physground: %s: no Skyrim triangles in the box, nothing built; %s", name, PlayerLine(self, lo, hi)))
		return
	end
	local ply = ThePlayer()
	-- The gap between the player's body (its collision box) and the mesh's bounding box: the
	-- body is at least this far from every triangle of the new mesh (0 = they may touch).
	local gap = "no player"
	if IsValid(ply) then
		local p = ply:GetPos() - self:GetPos()
		local blo, bhi = p + ply:OBBMins(), p + ply:OBBMaxs()
		local dx = math.max(mlo.x - bhi.x, 0, blo.x - mhi.x)
		local dy = math.max(mlo.y - bhi.y, 0, blo.y - mhi.y)
		local dz = math.max(mlo.z - bhi.z, 0, blo.z - mhi.z)
		local d = math.sqrt(dx * dx + dy * dy + dz * dz)
		gap = d > 0 and string.format("player's body at least %.0f units from the new mesh", d) or "player's body MAY TOUCH the new mesh (inside its bounds)"
	end
	s.builds, s.tris = s.builds + 1, s.tris + n
	gmsky.Log(string.format("physground: %s: %d triangles (%d vertices), box %.0f x %.0f x %.0f, mesh bounds %.0f x %.0f x %.0f, %s; %s; triangles read in %.2f ms (build %d on this side, %d triangles so far); older meshes destroyed first: %d, meshes with physics now: %d; %s",
		name, n, #verts, hi.x - lo.x, hi.y - lo.y, hi.z - lo.z, mhi.x - mlo.x, mhi.y - mlo.y, mhi.z - mlo.z, gap, shape,
		(SysTime() - t0) * 1000, s.builds, s.tris, destroyed, MeshesWithPhysics(), PlayerLine(self, lo, hi)))
	local function Step(what, fn)
		gmsky.Log(string.format("physground: %s step: %s", name, what))
		return fn()
	end
	t0 = SysTime()
	-- InfMap2's chunk setup, in its order.
	Step("PhysicsDestroy", function() self:PhysicsDestroy() end)
	self.gmskyOk = Step("PhysicsFromMesh", function() return self:PhysicsFromMesh(verts) end)
	Step("SetSolid(SOLID_VPHYSICS)", function() self:SetSolid(SOLID_VPHYSICS) end)
	Step("SetMoveType(MOVETYPE_NONE)", function() self:SetMoveType(MOVETYPE_NONE) end)
	Step("EnableCustomCollisions", function() self:EnableCustomCollisions(true) end)
	local phys = self:GetPhysicsObject()
	if IsValid(phys) then
		Step("EnableMotion(false)", function() phys:EnableMotion(false) end)
		Step("SetMass(50000)", function() phys:SetMass(50000) end)
		Step("AddGameFlag(FVPHYSICS_CONSTRAINT_STATIC)", function() phys:AddGameFlag(FVPHYSICS_CONSTRAINT_STATIC) end)
		Step("AddGameFlag(FVPHYSICS_NO_SELF_COLLISIONS)", function() phys:AddGameFlag(FVPHYSICS_NO_SELF_COLLISIONS) end)
	end
	Step("DrawShadow(false)", function() self:DrawShadow(false) end)
	Step("AddSolidFlags(FSOLID_FORCE_WORLD_ALIGNED)", function() self:AddSolidFlags(FSOLID_FORCE_WORLD_ALIGNED) end)
	Step("AddFlags(FL_STATICPROP)", function() self:AddFlags(FL_STATICPROP) end)
	Step("AddEFlags(EFL_KEEP_ON_RECREATE_ENTITIES)", function() self:AddEFlags(EFL_KEEP_ON_RECREATE_ENTITIES) end)
	Step("SetCustomCollisionCheck(true)", function() self:SetCustomCollisionCheck(true) end)
	local ms = (SysTime() - t0) * 1000
	s.worst = math.max(s.worst, ms)
	phys = self:GetPhysicsObject()
	gmsky.Log(string.format("physground: built %s in %.2f ms: %d triangles; meshes with physics on this side %d; PhysicsFromMesh=%s physics object=%s solid=%d solid flags=%d%s",
		name, ms, n, MeshesWithPhysics(), tostring(self.gmskyOk), tostring(IsValid(phys)), self:GetSolid(), self:GetSolidFlags(),
		IsValid(phys) and string.format(" mass %.0f, constraint-static flag %s, no-self-collisions flag %s, moving %s", phys:GetMass(),
			tostring(phys:HasGameFlag(FVPHYSICS_CONSTRAINT_STATIC)), tostring(phys:HasGameFlag(FVPHYSICS_NO_SELF_COLLISIONS)),
			tostring(phys:IsMotionEnabled())) or ""))
end

function ENT:Initialize()
	if SERVER then self:BuildMesh() end
end

function ENT:Think()
	-- The client builds its copy as soon as the entity's data has arrived.
	if CLIENT and not self.gmskyBuilt and self:GetMode() ~= 0 then self:BuildMesh() end
end

if CLIENT then
	function ENT:Draw() end
end

scripted_ents.Register(ENT, "gmsky_physground_test")

hook.Add("PhysgunPickup", "gmsky_physground_nograb", function(_, ent)
	if IsValid(ent) and ent:GetClass() == "gmsky_physground_test" then return false end
end)

-- InfMap2 also turns on custom collision checks for every entity in its world and answers
-- ShouldCollide (all of ours are in one "chunk", so it never says no). Counted, to see it runs.
G.shouldCollide = 0
hook.Add("ShouldCollide", "gmsky_physground_count", function()
	G.shouldCollide = G.shouldCollide + 1
end)

-- ---- the experiment (server) -----------------------------------------------------------------

if SERVER then
	G.test = G.test or nil -- { mode, source, size, flatZ, serial, next, started, ground, flagged, chunks, waiting, removed }
	G.chunkSize = CreateConVar("gmsky_physground_chunk", "4", 0,
		"gmsky_test_physground chunks: chunk width in Skyrim regions (4 = the most today's collection window allows)")
	G.gridTris = CreateConVar("gmsky_physground_grid", "7400", 0,
		"gmsky_test_physground ... grid: about this many triangles per mesh (test 109's crashing chunk had 7,413)")

	local function Flag(ent)
		if not G.test or not IsValid(ent) then return end
		if ent:IsPlayer() or ent:GetMoveType() == MOVETYPE_VPHYSICS then
			ent:SetCustomCollisionCheck(true)
			G.test.flagged[ent] = true
		end
	end

	local function Stop(why)
		if IsValid(G.current) then G.current:Remove() end
		G.current = nil
		if G.test then
			for _, e in pairs(G.test.chunks) do
				if IsValid(e) then e:Remove() end
			end
			for ent in pairs(G.test.flagged) do
				if IsValid(ent) then ent:SetCustomCollisionCheck(false) end
			end
			gmsky.Log(string.format("physground: experiment stopped (%s) after %d builds, %.0f s", why, G.test.serial, CurTime() - G.test.started))
		end
		G.test = nil
	end

	local function NewMesh(t, lo, hi, cx, cy)
		local ax, ay, az = gmsky.Anchor()
		t.serial = t.serial + 1
		local e = ents.Create("gmsky_physground_test")
		e:SetMode(t.mode)
		e:SetSource(t.source)
		e:SetBuild(t.serial)
		e:SetChunkX(cx or 0)
		e:SetChunkY(cy or 0)
		e:SetFlatZ(t.flatZ)
		e:SetGridN(t.gridN)
		e:SetBoxLo(lo)
		e:SetBoxHi(hi)
		e:SetPos(Vector(ax, ay, az))
		e:SetAngles(Angle(0, 0, 0))
		e:Spawn()
		Flag(e)
		return e
	end

	local function BuildRebuild(t)
		local ply = player.GetAll()[1]
		local ax, ay, az = gmsky.Anchor()
		if not IsValid(ply) or not ax then return end
		local p = ply:GetPos() - Vector(ax, ay, az)
		local e = NewMesh(t, p - Vector(HALF_XY, HALF_XY, HALF_Z), p + Vector(HALF_XY, HALF_XY, HALF_Z))
		-- The new one destroyed the previous one's physics before building (ClearOlder); the entity
		-- itself goes now.
		if IsValid(G.current) then G.current:Remove() end
		G.current = e
	end

	-- Chunks: build at most one needed chunk per look, remove far ones.
	local function UpdateChunks(t)
		local ply = player.GetAll()[1]
		local ax, ay, az = gmsky.Anchor()
		if not IsValid(ply) or not ax then return end
		local bx, by, bz, size = gmsky.ShapesLattice()
		if not bx then
			t.waiting = { ["all"] = "Skyrim's region grid not known yet (no region arrived)" }
			return
		end
		local p = ply:GetPos() - Vector(ax, ay, az)
		local N = t.size
		local prx, pry, prz = math.floor((p.x - bx) / size), math.floor((p.y - by) / size), math.floor((p.z - bz) / size)
		local pcx, pcy = math.floor(prx / N), math.floor(pry / N)
		t.waiting = {}
		local built = false
		for cx = pcx - 1, pcx + 1 do
			for cy = pcy - 1, pcy + 1 do
				local key = cx .. " " .. cy
				if not t.chunks[key] then
					local lo = Vector(bx + cx * N * size, by + cy * N * size, bz + (prz - BELOW) * size)
					local hi = Vector(lo.x + N * size, lo.y + N * size, bz + (prz + ABOVE + 1) * size)
					local d = math.max(lo.x - p.x, 0, p.x - hi.x, lo.y - p.y, p.y - hi.y)
					if d <= LEAD * size then
						local arrived, total = 1, 1
						if t.source == SKYRIM then -- flat and grid need nothing from Skyrim
							arrived, total = gmsky.ShapesRegionsArrived(cx * N, cy * N, prz - BELOW, cx * N + N - 1, cy * N + N - 1, prz + ABOVE)
						end
						if arrived < total then
							t.waiting[key] = string.format("%d of %d regions arrived, player %.0f units away", arrived, total, d)
						elseif not built then
							built = true
							t.chunks[key] = NewMesh(t, lo, hi, cx, cy)
						end
					end
				end
			end
		end
		for key, e in pairs(t.chunks) do
			local cx, cy = key:match("^(%-?%d+) (%-?%d+)$")
			cx, cy = tonumber(cx), tonumber(cy)
			if math.abs(cx - pcx) > REMOVE_AT or math.abs(cy - pcy) > REMOVE_AT then
				gmsky.Log(string.format("physground: chunk %s removed (player's chunk %d %d), had %s triangles",
					key, pcx, pcy, IsValid(e) and tostring(e.gmskyTris) or "?"))
				if IsValid(e) then e:Remove() end
				t.chunks[key] = nil
				t.removed = t.removed + 1
			end
		end
	end

	hook.Add("Tick", "gmsky_physground_test", function()
		local t = G.test
		if not t then return end
		local ply = player.GetAll()[1]
		if IsValid(ply) then
			local g = ply:GetGroundEntity()
			local k = IsValid(g) and g:GetClass() or (g == game.GetWorld() and "the map" or (ply:IsOnGround() and "?" or "air"))
			t.ground[k] = (t.ground[k] or 0) + 1
		end
		-- A re-centre or new origin moved the anchor: the meshes are in the wrong place.
		local ax, ay, az = gmsky.Anchor()
		if ax and t.anchor and t.anchor:DistToSqr(Vector(ax, ay, az)) > 0.01 then
			if t.mode == CHUNKS then
				Stop("the anchor moved (re-centre or new origin); chunks are not moved in this experiment")
				return
			end
			gmsky.Log("physground: the anchor moved (re-centre or new origin): rebuilding now")
			t.next = 0
		end
		if ax then t.anchor = Vector(ax, ay, az) end
		if CurTime() >= t.next then
			if t.mode == REBUILD then
				t.next = CurTime() + EVERY
				BuildRebuild(t)
			else
				t.next = CurTime() + CHUNK_EVERY
				UpdateChunks(t)
			end
		end
	end)

	hook.Add("OnEntityCreated", "gmsky_physground_flag", function(ent)
		timer.Simple(0, function() Flag(ent) end)
	end)

	timer.Create("gmsky_physground_status", 5, 0, function()
		local t = G.test
		if not t then return end
		local parts = {}
		for k, n in pairs(t.ground) do parts[#parts + 1] = string.format("%s %d", k, n) end
		table.sort(parts)
		local s = G.side
		local chunks = ""
		if t.mode == CHUNKS then
			local alive, waiting = {}, {}
			for key, e in pairs(t.chunks) do alive[#alive + 1] = string.format("%s: %s", key, IsValid(e) and tostring(e.gmskyTris) or "gone") end
			for key, why in pairs(t.waiting) do waiting[#waiting + 1] = string.format("%s: %s", key, why) end
			table.sort(alive)
			table.sort(waiting)
			chunks = string.format("; chunks alive (triangles) %d [%s], removed %d, waiting [%s]", #alive, table.concat(alive, ", "),
				t.removed, #waiting > 0 and table.concat(waiting, "; ") or "-")
		end
		gmsky.Log(string.format("physground: %s %s running %.0f s: %d builds (server side %d, %d triangles in all, slowest setup %.2f ms); ShouldCollide asked %d times; player's ground in the last 5 s (ticks): %s%s",
			MODES[t.mode], SOURCES[t.source], CurTime() - t.started, t.serial, s.builds, s.tris, s.worst, G.shouldCollide,
			#parts > 0 and table.concat(parts, ", ") or "-", chunks))
		t.ground = {}
		G.shouldCollide = 0
	end)

	concommand.Add("gmsky_test_physground", function(_, _, args)
		Stop("new command")
		local mode = args[1] == "rebuild" and REBUILD or args[1] == "chunks" and CHUNKS or nil
		local source = (args[2] == nil or args[2] == "skyrim") and SKYRIM or args[2] == "flat" and FLAT or args[2] == "grid" and GRID or nil
		if not mode or not source then
			gmsky.Log("physground: experiment off (use gmsky_test_physground rebuild|chunks skyrim|flat|grid, or off)")
			return
		end
		local ply = player.GetAll()[1]
		local ax, ay, az = gmsky.Anchor()
		if not IsValid(ply) or not ax then
			gmsky.Log("physground: no player or no anchor yet (Skyrim not connected?): not started")
			return
		end
		G.test = { mode = mode, source = source, size = math.max(1, math.floor(G.chunkSize:GetFloat())),
			flatZ = ply:GetPos().z - az, serial = 0, next = 0, started = CurTime(), ground = {}, flagged = {},
			chunks = {}, waiting = {}, removed = 0,
			gridN = math.max(1, math.floor(math.sqrt(math.max(2, G.gridTris:GetFloat()) / 2) + 0.5)) }
		for _, ent in ipairs(ents.GetAll()) do Flag(ent) end
		gmsky.Log(string.format("physground: experiment START, %s %s (InfMap2's setup, custom collision checks on the player and physics objects): %s; flat plane height %.1f relative to the anchor%s",
			MODES[mode], SOURCES[source],
			mode == REBUILD and string.format("a new mesh within %d x %d x %d units of the player every %.1f s, the old one's physics destroyed first", HALF_XY * 2, HALF_XY * 2, HALF_Z * 2, EVERY)
				or string.format("chunks %d x %d regions wide, %d regions tall, built once when the player is within %d region of one, removed beyond %d chunks; cap %d triangles per mesh",
					G.test.size, G.test.size, BELOW + ABOVE + 1, LEAD, REMOVE_AT, MAX_TRIS),
			G.test.flatZ, source == GRID and string.format("; grid %d x %d squares = %d triangles per mesh", G.test.gridN, G.test.gridN, G.test.gridN * G.test.gridN * 2) or ""))
	end)
end
