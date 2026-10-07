-- gmsky: Skyrim's loose objects as Garry's Mod physics objects (step S2k-K1, DESIGN.md "S2k").
-- Runs on both sides (server.lua and client.lua include it).
--
-- Skyrim sends its loose bodies near the player (plates, food, baskets, dropped weapons, moveable
-- statics; Loose.cpp) through the collision ring: the shape once (convex pieces), then every move,
-- then "gone". The module keeps them (loose.rs). The server makes one invisible `gmsky_loose`
-- entity per body, its physics from the pieces (PhysicsInitMultiConvex), frozen, and moves it
-- wherever Skyrim says. The client builds its own physics for each entity from the same store (one
-- module per process), for movement prediction, as chunks.lua does. Skyrim still draws and owns
-- the real objects: nothing here moves them (that is K2).
-- Only in physics ground mode (gmsky_ground physics). Logs: gmod_server.log / gmod_client.log.

local gmsky = gmsky
local L = gmsky.loose or {}
gmsky.loose = L

local BUDGET_MS = 3        -- builds per tick (server) / frame (client) until this much time is spent ...
local MIN_BUILDS = 5       -- ... but at least this many
local ENTITY_LIMIT = 7600  -- no new copies while more entities than this exist (the wiki gives 8,192; not sure)
local STRIDE = 18          -- numbers per object in gmsky.LooseChanges' list
local FLAG_KEYFRAMED, FLAG_JOINED = 0x2, 0x4

L.stats = L.stats or { built = 0, failed = 0, moved = 0, removed = 0, limit = 0, worst = 0, pieces = 0 }

-- The body's axes (x, y, z, 3 numbers each) -> angles. Both games are Z-up with the same axes.
local function Angles(r)
	local m = Matrix({ { r[1], r[4], r[7], 0 }, { r[2], r[5], r[8], 0 }, { r[3], r[6], r[9], 0 }, { 0, 0, 0, 1 } })
	return m:GetAngles()
end

-- Object n's pieces as PhysicsInitMultiConvex wants them (lists of Vectors), or nil.
local function Pieces(n)
	local flat = gmsky.LooseShape(n)
	if not flat then return end
	local meshes = {}
	for _, f in ipairs(flat) do
		local pts = {}
		for i = 1, #f, 3 do pts[#pts + 1] = Vector(f[i], f[i + 1], f[i + 2]) end
		meshes[#meshes + 1] = pts
	end
	return meshes
end

-- ---- the entity ---------------------------------------------------------------------------------

local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "N")       -- the object's number in the module's store
	self:NetworkVar("Float", 0, "Mass")
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction
end

-- Builds the physics from the store; frozen at once (motion off right after the shape is made,
-- before any other setup call: S2b's rule for new physics objects). Returns ok, pieces.
function ENT:Build()
	local meshes = Pieces(self:GetN())
	if not meshes or #meshes == 0 then return false, 0 end
	self:PhysicsDestroy()
	local ok = self:PhysicsInitMultiConvex(meshes)
	local phys = self:GetPhysicsObject()
	if not ok or not IsValid(phys) then return false, #meshes end
	phys:EnableMotion(false)
	self:SetSolid(SOLID_VPHYSICS)
	self:SetMoveType(MOVETYPE_VPHYSICS)
	self:EnableCustomCollisions(true) -- traces (bullets, the player) use the real shape
	phys:SetMass(math.Clamp(self:GetMass(), 1, 50000))
	phys:SetPos(self:GetPos())
	phys:SetAngles(self:GetAngles())
	self:DrawShadow(false)
	self.gmskyBuilt = true
	return true, #meshes
end

function ENT:Initialize()
	self:DrawShadow(false)
	if CLIENT then L.clientEnts[self] = true end
end

function ENT:OnRemove()
	if CLIENT then L.clientEnts[self] = nil end
end

if CLIENT then
	function ENT:Draw() end -- Skyrim draws the real object
end

scripted_ents.Register(ENT, "gmsky_loose")

-- Skyrim owns these objects until K2: nothing in Garry's Mod may pick them up or change them.
local function Ours(ent) return IsValid(ent) and ent:GetClass() == "gmsky_loose" end
hook.Add("PhysgunPickup", "gmsky_loose", function(_, ent) if Ours(ent) then return false end end)
hook.Add("CanTool", "gmsky_loose", function(_, tr) if Ours(tr.Entity) then return false end end)
hook.Add("GravGunPickupAllowed", "gmsky_loose", function(_, ent) if Ours(ent) then return false end end)
hook.Add("GravGunPunt", "gmsky_loose", function(_, ent) if Ours(ent) then return false end end)
hook.Add("CanPlayerUnfreeze", "gmsky_loose", function(_, ent) if Ours(ent) then return false end end)

-- ---- the server ---------------------------------------------------------------------------------

if SERVER then
	L.objs = L.objs or {}      -- n -> { rel = Vector (relative to the anchor), ang, mass, flags, ent }
	L.gen, L.ver = L.gen or -1, L.ver or 0

	local function Remove(rec)
		if IsValid(rec.ent) then
			rec.ent:Remove()
			L.stats.removed = L.stats.removed + 1
		end
		rec.ent = nil
	end

	-- A copy and its frozen physics object to a pose.
	function L.Place(e, pos, ang)
		e:SetPos(pos)
		e:SetAngles(ang)
		local phys = e:GetPhysicsObject()
		if IsValid(phys) then
			phys:SetPos(pos)
			phys:SetAngles(ang)
		end
	end

	-- Removes every copy (new origin, a switch of ground mode).
	function L.Clear(why)
		local n = 0
		for _, rec in pairs(L.objs) do
			if IsValid(rec.ent) then n = n + 1 end
			Remove(rec)
		end
		L.objs = {}
		L.gen, L.ver = -1, 0
		if n > 0 then gmsky.Log(string.format("loose: all %d copies removed (%s)", n, why)) end
	end

	-- Re-centre: every copy to the new anchor, in the same tick as the chunks.
	function L.MoveAll(anchor)
		local n = 0
		for _, rec in pairs(L.objs) do
			if IsValid(rec.ent) then
				L.Place(rec.ent, anchor + rec.rel, rec.ang)
				n = n + 1
			end
		end
		if n > 0 then gmsky.Log(string.format("loose: re-centre: %d copies moved to the new anchor", n)) end
	end

	local function Build(rec, n, a)
		if ents.GetCount() > ENTITY_LIMIT then
			L.stats.limit = L.stats.limit + 1
			return false
		end
		local e = ents.Create("gmsky_loose")
		if not IsValid(e) then
			L.stats.failed = L.stats.failed + 1
			return false
		end
		e:SetPos(a + rec.rel)
		e:SetAngles(rec.ang)
		e:SetN(n)
		e:SetMass(rec.mass)
		e:Spawn()
		local ok, pieces = e:Build()
		if not ok then
			L.stats.failed = L.stats.failed + 1
			if (L.failLogged or 0) < 20 then
				L.failLogged = (L.failLogged or 0) + 1
				gmsky.Log(string.format("loose: object %d (FormID %08X): physics FAILED from %d pieces", n, rec.formId, pieces))
			end
			e:Remove()
			rec.failed = true
			return true
		end
		rec.ent = e
		L.stats.built, L.stats.pieces = L.stats.built + 1, L.stats.pieces + pieces
		return true
	end

	function L.Update(ply, a)
		local gen, ver, resync, list, gone = gmsky.LooseChanges(L.gen, L.ver)
		if not gen then return end
		if resync or gen ~= L.gen then
			for _, rec in pairs(L.objs) do Remove(rec) end
			L.objs = {}
		end
		L.gen, L.ver = gen, ver
		for _, n in ipairs(gone) do
			local rec = L.objs[n]
			if rec then
				Remove(rec)
				L.objs[n] = nil
			end
		end
		for i = 1, #list, STRIDE do
			local n = list[i]
			local rec = L.objs[n]
			local rel = Vector(list[i + 6], list[i + 7], list[i + 8])
			local ang = Angles({ list[i + 9], list[i + 10], list[i + 11], list[i + 12], list[i + 13], list[i + 14], list[i + 15], list[i + 16], list[i + 17] })
			if not rec then
				rec = { flags = list[i + 1], formId = list[i + 4], mass = list[i + 5] }
				L.objs[n] = rec
			elseif IsValid(rec.ent) then
				L.Place(rec.ent, a + rel, ang)
				L.stats.moved = L.stats.moved + 1
			end
			rec.rel, rec.ang, rec.flags = rel, ang, list[i + 1]
		end
		-- Build the waiting ones, nearest the player first.
		local waiting = {}
		local p = ply:GetPos() - a
		for n, rec in pairs(L.objs) do
			if not rec.ent and not rec.failed then waiting[#waiting + 1] = { n = n, d = rec.rel:DistToSqr(p) } end
		end
		L.waiting = #waiting
		if #waiting == 0 then return end
		table.sort(waiting, function(x, y) return x.d < y.d end)
		local t0, built = SysTime(), 0
		for _, w in ipairs(waiting) do
			if built >= MIN_BUILDS and (SysTime() - t0) * 1000 > BUDGET_MS then break end
			local t1 = SysTime()
			if not Build(L.objs[w.n], w.n, a) then break end
			L.stats.worst = math.max(L.stats.worst, (SysTime() - t1) * 1000)
			built = built + 1
		end
		L.waiting = #waiting - built
	end

	hook.Add("Tick", "gmsky_loose", function()
		local T = gmsky.terrain
		local ply = player.GetAll()[1]
		local a = T and T.Anchor and T.Anchor()
		if not IsValid(ply) or not a or not T.PhysicsMode() then
			if next(L.objs) ~= nil then L.Clear("not in physics ground mode, or not anchored") end
			return
		end
		L.Update(ply, a)
	end)

	timer.Create("gmsky_loose_status", 1, 0, function()
		if not gmsky.LooseStatus then return end
		local known, copies, keyframed, joined, failed = 0, 0, 0, 0, 0
		for _, rec in pairs(L.objs) do
			known = known + 1
			if IsValid(rec.ent) then copies = copies + 1 end
			if rec.failed then failed = failed + 1 end
			if bit.band(rec.flags or 0, FLAG_KEYFRAMED) ~= 0 then keyframed = keyframed + 1 end
			if bit.band(rec.flags or 0, FLAG_JOINED) ~= 0 then joined = joined + 1 end
		end
		local _, store = gmsky.LooseStatus(true)
		if known == 0 and L.stats.removed == 0 and not store:find("in: [1-9]") then return end
		local s = L.stats
		gmsky.Log(string.format("loose: %d objects known, %d copies (%d keyframed, %d joined, %d failed for good), %d waiting | this second: %d built (%d pieces, slowest %.2f ms), %d failed, %d moved, %d removed, %d refused at the entity limit (%d) | entities in the game %d, edicts %d | %s",
			known, copies, keyframed, joined, failed, L.waiting or 0, s.built, s.pieces, s.worst, s.failed, s.moved, s.removed, s.limit, ENTITY_LIMIT,
			ents.GetCount(), ents.GetEdictCount(), store))
		L.stats = { built = 0, failed = 0, moved = 0, removed = 0, limit = 0, worst = 0, pieces = 0 }
	end)
end

-- ---- the client ---------------------------------------------------------------------------------

if CLIENT then
	L.clientEnts = L.clientEnts or {}
	L.cstats = L.cstats or { built = 0, failed = 0, followed = 0, worst = 0 }
	local nextFollow = 0

	hook.Add("Think", "gmsky_loose", function()
		local t0, built = SysTime(), 0
		local follow = CurTime() >= nextFollow
		if follow then nextFollow = CurTime() + 0.1 end
		for e in pairs(L.clientEnts) do
			if not IsValid(e) then
				L.clientEnts[e] = nil
			elseif not e.gmskyBuilt then
				if e:GetN() > 0 and not e.gmskyFailed and (built < MIN_BUILDS or (SysTime() - t0) * 1000 < BUDGET_MS) then
					local t1 = SysTime()
					local ok = e:Build()
					built = built + 1
					L.cstats.worst = math.max(L.cstats.worst, (SysTime() - t1) * 1000)
					if ok then L.cstats.built = L.cstats.built + 1 else
						-- The store may not have it (yet, or any more): tried again in a second.
						L.cstats.failed = L.cstats.failed + 1
						e.gmskyFailed = true
						timer.Simple(1, function() if IsValid(e) then e.gmskyFailed = nil end end)
					end
				end
			elseif follow then
				-- Follow the entity: the physics object doesn't move by itself on this side.
				local phys = e:GetPhysicsObject()
				local pos, ang = e:GetPos(), e:GetAngles()
				if IsValid(phys) and (pos ~= e.gmskyPos or ang ~= e.gmskyAng) then
					if e.gmskyPos then L.cstats.followed = L.cstats.followed + 1 end
					phys:SetPos(pos)
					phys:SetAngles(ang)
					e.gmskyPos, e.gmskyAng = pos, ang
				end
			end
		end
	end)

	timer.Create("gmsky_loose_status", 5, 0, function()
		local n, built = 0, 0
		for e in pairs(L.clientEnts) do
			if IsValid(e) then
				n = n + 1
				if e.gmskyBuilt then built = built + 1 end
			end
		end
		local c = L.cstats
		if n == 0 and c.built == 0 then return end
		gmsky.Log(string.format("loose (client): %d copies, %d with physics | last 5 s: %d built (slowest %.2f ms), %d tries failed, %d followed a move",
			n, built, c.built, c.worst, c.failed, c.followed))
		L.cstats = { built = 0, failed = 0, followed = 0, worst = 0 }
	end)
end
