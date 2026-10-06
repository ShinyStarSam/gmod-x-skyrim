-- gmsky physics ground (DESIGN.md "One collision system"). Runs on both sides (server.lua and
-- client.lua include it).
--
-- Step S1, the crash experiment: `gmsky_test_physground old|infmap|off`. Every half second the
-- server makes a physics object out of Skyrim's triangles around the player (from the module's
-- store, gmsky.TrisInBox) and removes the previous one, with the player colliding with it. Tests
-- 12-17 crashed Garry's Mod's physics (vphysics.dll) building ground under the moving player; this
-- tries to crash it on purpose, with our old entity setup ("old", from commit 911fb85) and with
-- InfMap2's ("infmap"). InfMap2 (https://codeberg.org/googer/infmap2, lua/entities/inf_chunk.lua
-- and lua/infmap2/ish/physics.lua) has no licence file: its setup is described in DESIGN.md and
-- written here from that description; no code is copied.
-- The client builds its copy too (as in tests 12-17), from the same store (one module per process).
-- Every build is logged before and after, so a crash names the build.
--
-- Rule since tests 101-103 (both setups crashed with a new mesh built on top of the previous one,
-- which still existed): never two meshes for the same place at once, on either side. Before a
-- mesh is built, every older test mesh on that side loses its physics object (PhysicsDestroy is
-- immediate; Remove only happens at the end of the tick), and a mesh older than one that already
-- exists is never built. InfMap2 keeps the same rule: one mesh per chunk, ever.

local gmsky = gmsky
local G = gmsky.physground or {}
gmsky.physground = G

local HALF_XY, HALF_Z = 384, 256  -- the box around the player's feet (tests 12-17's pieces: 768 x 768)
local MAX_TRIS = 4000
local EVERY = 0.5                 -- seconds between builds
local OLD, INFMAP = 1, 2
local NAMES = { [OLD] = "old", [INFMAP] = "infmap" }

G.side = G.side or { builds = 0, tris = 0, worst = 0 }

local function ThePlayer()
	if SERVER then return player.GetAll()[1] end
	return LocalPlayer()
end

-- Where the player is against a test mesh entity, for the log.
local function PlayerLine(ent)
	local ply = ThePlayer()
	if not IsValid(ply) then return "no player" end
	local o = ent:GetPos()
	local p, v = ply:GetPos(), ply:GetVelocity()
	local ground = ply:GetGroundEntity()
	local on = IsValid(ground) and ground:GetClass() or (ground == game.GetWorld() and "the map" or "nothing")
	return string.format("player feet (%.1f, %.1f, %.1f) relative to the anchor, speed %.0f, %s, on %s",
		p.x - o.x, p.y - o.y, p.z - o.z, v:Length(), ply:IsOnGround() and "on the ground" or "in the air", on)
end

-- The vertices PhysicsFromMesh wants, relative to the entity (which sits at the anchor).
-- Never two meshes at once: destroys the physics of every older test mesh on this side, first.
-- Returns how many were destroyed, or nil if a newer mesh already exists (then this one is skipped).
local function ClearOlder(self)
	local destroyed = 0
	for _, other in ipairs(ents.FindByClass("gmsky_physground_test")) do
		if other ~= self and IsValid(other) then
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

-- How many test meshes on this side have a physics object (must be 1 after a build).
local function MeshesWithPhysics()
	local n = 0
	for _, e in ipairs(ents.FindByClass("gmsky_physground_test")) do
		if IsValid(e) and IsValid(e:GetPhysicsObject()) then n = n + 1 end
	end
	return n
end

local function Verts(lo, hi)
	local flat, n = gmsky.TrisInBox(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, MAX_TRIS)
	local verts = {}
	for i = 1, n * 9, 3 do
		verts[#verts + 1] = { pos = Vector(flat[i], flat[i + 1], flat[i + 2]) }
	end
	return verts, n
end

-- ---- the test entity ---------------------------------------------------------------------------

local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "Recipe")    -- OLD or INFMAP
	self:NetworkVar("Int", 1, "Build")     -- the server's build number
	self:NetworkVar("Vector", 0, "BoxLo")  -- the box the triangles were taken from, relative to the anchor
	self:NetworkVar("Vector", 1, "BoxHi")
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction
end

function ENT:BuildMesh()
	self.gmskyBuilt = true
	local recipe, lo, hi = self:GetRecipe(), self:GetBoxLo(), self:GetBoxHi()
	local destroyed = ClearOlder(self)
	if not destroyed then
		gmsky.Log(string.format("physground: build %d skipped on this side: a newer mesh already exists", self:GetBuild()))
		return
	end
	local verts, n = Verts(lo, hi)
	local s = G.side
	if n == 0 then
		gmsky.Log(string.format("physground: build %d (%s): no Skyrim triangles in the box, nothing built; %s",
			self:GetBuild(), NAMES[recipe] or "?", PlayerLine(self)))
		return
	end
	s.builds, s.tris = s.builds + 1, s.tris + n
	gmsky.Log(string.format("physground: build %d (%s): %d triangles (build %d on this side, %d triangles so far); older meshes destroyed first: %d, meshes with physics now: %d; %s...",
		self:GetBuild(), NAMES[recipe] or "?", n, s.builds, s.tris, destroyed, MeshesWithPhysics(), PlayerLine(self)))
	local t0 = SysTime()
	if recipe == INFMAP then
		-- InfMap2's chunk setup, in its order.
		self:PhysicsDestroy()
		local ok = self:PhysicsFromMesh(verts)
		self:SetSolid(SOLID_VPHYSICS)
		self:SetMoveType(MOVETYPE_NONE)
		self:EnableCustomCollisions(true)
		local phys = self:GetPhysicsObject()
		if IsValid(phys) then
			phys:EnableMotion(false)
			phys:SetMass(50000)
			phys:AddGameFlag(FVPHYSICS_CONSTRAINT_STATIC)
			phys:AddGameFlag(FVPHYSICS_NO_SELF_COLLISIONS)
		end
		self:DrawShadow(false)
		self:AddSolidFlags(FSOLID_FORCE_WORLD_ALIGNED)
		self:AddFlags(FL_STATICPROP)
		self:AddEFlags(EFL_KEEP_ON_RECREATE_ENTITIES)
		self:SetCustomCollisionCheck(true)
		self.gmskyOk = ok
	else
		-- Tests 12-17's setup (terrain.lua BuildCollision, commit 911fb85), in its order.
		local ok = self:PhysicsFromMesh(verts)
		self:SetSolid(SOLID_VPHYSICS)
		self:SetMoveType(MOVETYPE_NONE)
		self:EnableCustomCollisions(true)
		self:DrawShadow(false)
		self:SetCollisionBounds(lo, hi)
		local phys = self:GetPhysicsObject()
		if IsValid(phys) then phys:EnableMotion(false) end
		self.gmskyOk = ok
	end
	local ms = (SysTime() - t0) * 1000
	s.worst = math.max(s.worst, ms)
	local phys = self:GetPhysicsObject()
	gmsky.Log(string.format("physground: built %d in %.2f ms: meshes with physics on this side %d (must be 1); PhysicsFromMesh=%s physics object=%s solid=%d solid flags=%d%s",
		self:GetBuild(), ms, MeshesWithPhysics(), tostring(self.gmskyOk), tostring(IsValid(phys)), self:GetSolid(), self:GetSolidFlags(),
		IsValid(phys) and string.format(" mass %.0f, constraint-static flag %s, no-self-collisions flag %s, moving %s", phys:GetMass(),
			tostring(phys:HasGameFlag(FVPHYSICS_CONSTRAINT_STATIC)), tostring(phys:HasGameFlag(FVPHYSICS_NO_SELF_COLLISIONS)),
			tostring(phys:IsMotionEnabled())) or ""))
end

function ENT:Initialize()
	if SERVER then self:BuildMesh() end
end

function ENT:Think()
	-- The client builds its copy as soon as the entity's data has arrived.
	if CLIENT and not self.gmskyBuilt and self:GetRecipe() ~= 0 then self:BuildMesh() end
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
	G.test = G.test or nil -- { recipe, serial, next, started, ground = {class -> ticks}, flagged = {ent -> true} }

	local function Flag(ent)
		if not G.test or G.test.recipe ~= INFMAP or not IsValid(ent) then return end
		if ent:IsPlayer() or ent:GetMoveType() == MOVETYPE_VPHYSICS then
			ent:SetCustomCollisionCheck(true)
			G.test.flagged[ent] = true
		end
	end

	local function Stop(why)
		if IsValid(G.current) then G.current:Remove() end
		G.current = nil
		if G.test then
			for ent in pairs(G.test.flagged) do
				if IsValid(ent) then ent:SetCustomCollisionCheck(false) end
			end
			gmsky.Log(string.format("physground: experiment stopped (%s) after %d builds, %.0f s", why, G.test.serial, CurTime() - G.test.started))
		end
		G.test = nil
	end

	local function Build()
		local ply = player.GetAll()[1]
		local ax, ay, az = gmsky.Anchor()
		if not IsValid(ply) or not ax then return end
		local t = G.test
		local p = ply:GetPos() - Vector(ax, ay, az)
		t.serial = t.serial + 1
		local e = ents.Create("gmsky_physground_test")
		e:SetRecipe(t.recipe)
		e:SetBuild(t.serial)
		e:SetBoxLo(p - Vector(HALF_XY, HALF_XY, HALF_Z))
		e:SetBoxHi(p + Vector(HALF_XY, HALF_XY, HALF_Z))
		e:SetPos(Vector(ax, ay, az))
		e:SetAngles(Angle(0, 0, 0))
		e:Spawn()
		Flag(e)
		-- The new one destroyed the previous one's physics before building (ClearOlder); the entity
		-- itself goes now.
		if IsValid(G.current) then G.current:Remove() end
		G.current = e
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
		-- A re-centre or new origin moved the anchor: the mesh is in the wrong place, rebuild now.
		local ax, ay, az = gmsky.Anchor()
		if IsValid(G.current) and ax and G.current:GetPos():DistToSqr(Vector(ax, ay, az)) > 0.01 then
			gmsky.Log("physground: the anchor moved (re-centre or new origin): rebuilding now")
			t.next = 0
		end
		if CurTime() >= t.next then
			t.next = CurTime() + EVERY
			Build()
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
		gmsky.Log(string.format("physground: %s running %.0f s: %d builds (server side %d, %d triangles in all, slowest %.2f ms); ShouldCollide asked %d times; player's ground in the last 5 s (ticks): %s",
			NAMES[t.recipe], CurTime() - t.started, t.serial, s.builds, s.tris, s.worst, G.shouldCollide,
			#parts > 0 and table.concat(parts, ", ") or "-"))
		t.ground = {}
		G.shouldCollide = 0
	end)

	concommand.Add("gmsky_test_physground", function(_, _, args)
		local which = args[1]
		Stop("new command")
		local recipe = which == "old" and OLD or which == "infmap" and INFMAP or nil
		if not recipe then
			gmsky.Log("physground: experiment off (use gmsky_test_physground old|infmap|off)")
			return
		end
		G.test = { recipe = recipe, serial = 0, next = 0, started = CurTime(), ground = {}, flagged = {} }
		if recipe == INFMAP then
			for _, ent in ipairs(ents.GetAll()) do Flag(ent) end
		end
		gmsky.Log(string.format("physground: experiment START, recipe %s: a new physics mesh from Skyrim's triangles within %d x %d x %d units of the player every %.1f s, the old one's physics destroyed first (never two meshes at once); custom collision checks on the player and physics objects: %s",
			which, HALF_XY * 2, HALF_XY * 2, HALF_Z * 2, EVERY, recipe == INFMAP and "yes (as InfMap2)" or "no"))
	end)
end
