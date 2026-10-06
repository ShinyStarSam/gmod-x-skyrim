-- gmsky physics chunks (step S2, DESIGN.md "S2 in detail"). Runs on both sides (server.lua and
-- client.lua include it).
--
-- With `gmsky_ground physics`, Skyrim's collision triangles become Garry's Mod physics objects:
-- one `gmsky_chunk` entity per block of gmsky_chunk_regions^3 Skyrim regions on a fixed lattice,
-- every triangle cut exactly at the chunk's faces so chunks never overlap. The player, bullets and
-- props all collide with them (one system; the trace field is off in this mode, terrain.lua).
-- Based on InfMap2's chunk recipe (https://codeberg.org/googer/infmap2, lua/entities/inf_chunk.lua;
-- no licence file, so written from DESIGN.md's description, no code copied), in a different order.
--
-- The rule from tests 101-117: a chunk is never built while an earlier mesh of the same chunk is
-- still in the physics engine. A destroyed mesh stays until the next physics step on the server
-- (Ghidra, 2026-10-06), and a new mesh built over it overflowed vphysics.dll's pair-record list
-- (test 116). S2b (test 119): the "static" flag is added right after PhysicsFromMesh, so a new
-- mesh never makes pair records with its neighbours while it is still movable.
-- The module checks the engine's memory, read only (gmsky.PhysAddress / gmsky.PhysAlive).
--
-- The server decides which chunks exist (all of Skyrim's collection window around the player,
-- once every region of a chunk has arrived) and rebuilds one only when its regions' content
-- fingerprint changes (gmsky.ShapesChunkVersion). Each side builds its own physics from the shared
-- Rust store (one module per process). Logs: gmod_server.log / gmod_client.log.

local gmsky = gmsky
local C = gmsky.chunks or {}
gmsky.chunks = C

local RADIUS, BELOW, ABOVE = 5, 3, 2 -- Skyrim's collection window in regions (Collision.cpp kRadius, kBelow, kAbove)
local BUDGET_MS = 3                  -- builds per tick (server) / frame (client) until this much time is spent; at least one
local MAX_TRIS = 127474              -- per mesh (as S1c; not sure it binds physics meshes)
local FALLBACK_TICKS = 10            -- without gmsky.PhysAlive: wait this long after a destroy (test 117 measured 7)

C.chunks = C.chunks or {}            -- server: key -> { ent, ver, empty, tris, big }
C.stats = C.stats or { builds = 0, rebuilds = 0, removed = 0, empties = 0, engineWaits = 0, worst = 0, tris = 0, mostTris = 0, moved = 0 }

local function Key(cx, cy, cz) return cx .. " " .. cy .. " " .. cz end

-- ---- meshes still in the engine ----------------------------------------------------------------
-- address ("0x...") -> { key, name, t0, tick0 }. Per side (each Lua state has its own).
C.dead = C.dead or {}

-- Every physics object's address -> name (to tell a freed and reused address from a lingering
-- object).
local function Owners()
	local owners = {}
	for _, ent in ipairs(ents.GetAll()) do
		if IsValid(ent) then
			for i = 0, math.max(ent:GetPhysicsObjectCount(), 1) - 1 do
				local p = ent:GetPhysicsObjectNum(i)
				if not IsValid(p) and i == 0 then p = ent:GetPhysicsObject() end
				if IsValid(p) then
					local a = gmsky.PhysAddress(p)
					if a then owners[a] = string.format("%s %d", ent:GetClass(), ent:EntIndex()) end
				end
			end
		end
	end
	return owners
end

-- Call just before a mesh's physics goes (PhysicsDestroy, or in OnRemove). `key` groups meshes
-- of the same place; `lo`, `hi` (optional): the mesh's box in Garry's Mod's world, so a new mesh
-- overlapping it waits even under another key (test 118: after a fast travel the chunk numbers
-- mean other places, and new chunks were built over the old ones still in the engine). `why` is
-- for the log.
function C.Destroyed(key, name, phys, why, lo, hi)
	if not IsValid(phys) then return end
	local addr = gmsky.PhysAddress and gmsky.PhysAddress(phys)
	C.dead[addr or ("tick " .. engine.TickCount() .. " " .. name)] = { key = key, name = name, t0 = SysTime(), tick0 = engine.TickCount(),
		noaddr = not addr, lo = lo, hi = hi }
	gmsky.Log(string.format("chunks: %s (%s) leaves the physics engine (%s)%s", name, tostring(addr), why,
		lo and string.format("; world box (%.0f, %.0f, %.0f) - (%.0f, %.0f, %.0f)", lo.x, lo.y, lo.z, hi.x, hi.y, hi.z) or ""))
end

-- Grows a box so that boxes meeting at a face, edge or corner overlap.
local TOUCH = Vector(1, 1, 1)

-- Boxes overlap by more than half a unit (chunks sharing a face don't count).
local function Overlap(alo, ahi, blo, bhi)
	return alo.x < bhi.x - 0.5 and blo.x < ahi.x - 0.5 and alo.y < bhi.y - 0.5 and blo.y < ahi.y - 0.5
		and alo.z < bhi.z - 0.5 and blo.z < ahi.z - 0.5
end

-- nil if no old mesh of `key`, and none overlapping or touching the world box `lo`-`hi` (if given),
-- is still in the engine (each departure logged once); otherwise a line naming those still there.
-- Touching counts too (test 122): a dead mesh has no game data, so the engine's filter can't keep
-- it apart from a new neighbour sharing its face, and the dead one's list overflowed. Gone = no
-- longer looks alive, or its address now belongs to another physics object (freed and reused, by
-- this side or, test 121, by the other side: one heap in single-player; gmsky.PhysAlive tells).
function C.StillInEngine(key, lo, hi)
	local mine = {}
	for addr, d in pairs(C.dead) do
		if d.key == key or (lo and d.lo and Overlap(lo - TOUCH, hi + TOUCH, d.lo, d.hi)) then mine[#mine + 1] = addr end
	end
	if #mine == 0 then return nil end
	local owners = gmsky.PhysAlive and Owners() or {}
	local still = {}
	for _, addr in ipairs(mine) do
		local d = C.dead[addr]
		local ms, ticks = (SysTime() - d.t0) * 1000, engine.TickCount() - d.tick0
		local alive, live, slots, reused = false, 0, 0, false
		if d.noaddr or not gmsky.PhysAlive then
			alive = ticks < FALLBACK_TICKS
		elseif not owners[addr] then
			alive, live, slots, reused = gmsky.PhysAlive(addr)
		end
		if alive then
			still[#still + 1] = string.format("%s (%s, its list %d/%d, %.0f ms / %d ticks ago)", d.name, addr, live, slots, ms, ticks)
		else
			gmsky.Log(string.format("chunks: %s (%s) gone from the engine %.0f ms, %d ticks after it was destroyed (%s)", d.name, addr, ms, ticks,
				owners[addr] and ("its memory now belongs to " .. owners[addr]) or (d.noaddr or not gmsky.PhysAlive) and string.format("no address: waited %d ticks", FALLBACK_TICKS)
				or reused and "its memory now holds a new object with an entity, not on this side: the other side's" or "no longer looks alive"))
			C.dead[addr] = nil
		end
	end
	return #still > 0 and table.concat(still, "; ") or nil
end

-- S2b's switch (shared: the client builds its own chunks in the same order).
C.flagFirst = C.flagFirst or CreateConVar("gmsky_chunks_flagfirst", "1", FCVAR_REPLICATED,
	"1 = chunks get FVPHYSICS_CONSTRAINT_STATIC right after PhysicsFromMesh (S2b); 0 = last, the order of tests 118-119")

-- Server, after every setup call of a chunk build: the new mesh's pair records with other chunks
-- (and the player), its list, and the largest list among the chunks touching it. `nextStep` names
-- the call that comes next, so the last line before a crash names the call it crashed in. Returns
-- the milliseconds it took (left out of the build time).
function C.StepLog(ent, after, flagFirst, nextStep)
	if not SERVER or not gmsky.PhysListPartners then return 0 end
	local t0 = SysTime()
	local phys = ent:GetPhysicsObject()
	if not IsValid(phys) then
		gmsky.Log(string.format("chunks: %s after %s: no physics object; next: %s", ent:ChunkName(), after, nextStep))
		return (SysTime() - t0) * 1000
	end
	local wlo, whi = ent:GetPos() + ent:GetBoxLo(), ent:GetPos() + ent:GetBoxHi()
	local names, physs, touching = {}, {}, {}
	for _, e in ipairs(ents.FindByClass("gmsky_chunk")) do
		local p = e ~= ent and e:GetPhysicsObject()
		if p and IsValid(p) then
			physs[#physs + 1], names[#names + 1] = p, e:ChunkName()
			-- Touching = boxes meet at a face, edge or corner.
			if Overlap(wlo - TOUCH, whi + TOUCH, e:GetPos() + e:GetBoxLo(), e:GetPos() + e:GetBoxHi()) then touching[#touching + 1] = #physs end
		end
	end
	local ply = player.GetAll()[1]
	local plyIdx
	if IsValid(ply) and IsValid(ply:GetPhysicsObject()) then
		physs[#physs + 1], names[#names + 1] = ply:GetPhysicsObject(), "player"
		plyIdx = #physs
	end
	local t, why = gmsky.PhysListPartners(phys, physs)
	local line
	if not t then
		line = "its list unreadable (" .. tostring(why) .. ")"
	else
		local withChunks, most, mostName = 0, 0, "-"
		for i = 1, #physs do
			local r = t[i] or 0
			if i ~= plyIdx then
				withChunks = withChunks + r
				if r > most then most, mostName = r, names[i] end
			end
		end
		local big, bigName = -1, "-"
		for _, i in ipairs(touching) do
			local live = gmsky.PhysList(physs[i])
			if live and live > big then big, bigName = live, names[i] end
		end
		C.stats.worstChunkRecords = math.max(C.stats.worstChunkRecords or 0, withChunks)
		line = string.format("records with other chunks %d (most: %s %d), with the player %d, with other objects %d; its list %d/%d; largest list of the %d touching chunks: %s %d",
			withChunks, mostName, most, plyIdx and t[plyIdx] or 0, t.unlisted, t.live, t.slots, #touching, bigName, math.max(big, 0))
	end
	gmsky.Log(string.format("chunks: %s after %s (flag %s): %s; next: %s", ent:ChunkName(), after, flagFirst and "first" or "last", line, nextStep))
	return (SysTime() - t0) * 1000
end

-- ---- the chunk entity --------------------------------------------------------------------------

local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "CX")         -- the chunk's place on the lattice
	self:NetworkVar("Int", 1, "CY")
	self:NetworkVar("Int", 2, "CZ")
	self:NetworkVar("Int", 3, "N")          -- regions per side
	self:NetworkVar("String", 0, "Version") -- the server's fingerprint of its regions when built
	self:NetworkVar("Vector", 0, "BoxLo")   -- the chunk's box, relative to the anchor
	self:NetworkVar("Vector", 1, "BoxHi")
end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction
end

function ENT:ChunkKey() return Key(self:GetCX(), self:GetCY(), self:GetCZ()) end
function ENT:ChunkName() return string.format("chunk %d %d %d", self:GetCX(), self:GetCY(), self:GetCZ()) end

-- Builds the physics. False (and the reason) if it can't yet: an earlier mesh of this chunk is
-- still in the engine. The server passes its triangles (already read) in `flat`.
function ENT:Build(flat, n)
	local key, name = self:ChunkKey(), self:ChunkName()
	local lo, hi = self:GetBoxLo(), self:GetBoxHi()
	local wlo, whi = self:GetPos() + lo, self:GetPos() + hi
	local still = C.StillInEngine(key, wlo, whi)
	if still then return false, still end
	if CLIENT then
		-- The server never has two chunks in one place, but the client may see a new chunk before
		-- the old one's removal arrives: wait for it too.
		for _, e in ipairs(ents.FindByClass("gmsky_chunk")) do
			if e ~= self and e.gmskyBuilt and IsValid(e:GetPhysicsObject())
				and Overlap(wlo, whi, e:GetPos() + e:GetBoxLo(), e:GetPos() + e:GetBoxHi()) then
				return false, string.format("%s, an older chunk entity here, still has its physics", e:ChunkName())
			end
		end
	end
	local t0 = SysTime()
	if not flat then
		local N = self:GetN()
		local _, _, ver = gmsky.ShapesChunkVersion(self:GetCX() * N, self:GetCY() * N, self:GetCZ() * N,
			self:GetCX() * N + N - 1, self:GetCY() * N + N - 1, self:GetCZ() * N + N - 1)
		if ver ~= self:GetVersion() then
			C.mismatches = (C.mismatches or 0) + 1
			gmsky.Log(string.format("chunks: %s: this side's regions differ from the server's (fingerprint %s here, %s there); built from this side's",
				name, tostring(ver), self:GetVersion()))
		end
		flat, n = gmsky.TrisInBox(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, MAX_TRIS + 1, true)
	end
	if n == 0 or n > MAX_TRIS then
		gmsky.Log(string.format("chunks: %s: %d triangles on this side: not built", name, n))
		self.gmskyBuilt = true
		return true
	end
	local verts = {}
	for i = 1, n * 9, 3 do verts[#verts + 1] = { pos = Vector(flat[i], flat[i + 1], flat[i + 2]) } end
	-- The setup calls. S2b (test 119): with gmsky_chunks_flagfirst 1 the "two static meshes never
	-- collide" flag comes right after PhysicsFromMesh, so the engine's filter already says "no" to
	-- every other chunk while the new mesh is still movable; 0 = the old order (flag last).
	local flagFirst = C.flagFirst:GetBool()
	local steps = {}
	-- `onPhys`: the call is on the physics object (skipped if there is none).
	local function Add(what, onPhys, fn) steps[#steps + 1] = { what, onPhys, fn } end
	local function Flags()
		Add("AddGameFlag(CONSTRAINT_STATIC, NO_SELF_COLLISIONS)", true, function(p)
			p:AddGameFlag(FVPHYSICS_CONSTRAINT_STATIC)
			p:AddGameFlag(FVPHYSICS_NO_SELF_COLLISIONS)
		end)
	end
	if flagFirst then Flags() end
	Add("SetSolid(SOLID_VPHYSICS)", false, function() self:SetSolid(SOLID_VPHYSICS) end)
	Add("SetMoveType(MOVETYPE_NONE)", false, function() self:SetMoveType(MOVETYPE_NONE) end)
	Add("EnableCustomCollisions(true)", false, function() self:EnableCustomCollisions(true) end)
	Add("EnableMotion(false)", true, function(p) p:EnableMotion(false) end)
	Add("SetMass(50000)", true, function(p) p:SetMass(50000) end)
	if not flagFirst then Flags() end
	self:PhysicsDestroy()
	local ok = self:PhysicsFromMesh(verts)
	local logMs = C.StepLog(self, "PhysicsFromMesh", flagFirst, steps[1][1])
	for i, st in ipairs(steps) do
		local p = self:GetPhysicsObject()
		if not st[2] or IsValid(p) then st[3](p) end
		logMs = logMs + C.StepLog(self, st[1], flagFirst, steps[i + 1] and steps[i + 1][1] or "nothing")
	end
	local phys = self:GetPhysicsObject()
	self:DrawShadow(false)
	self:AddSolidFlags(FSOLID_FORCE_WORLD_ALIGNED)
	self:AddFlags(FL_STATICPROP)
	self:AddEFlags(EFL_KEEP_ON_RECREATE_ENTITIES)
	self.gmskyBuilt, self.gmskyTris = true, n
	local ms = (SysTime() - t0) * 1000 - logMs
	local s = C.stats
	s.builds, s.tris = s.builds + 1, s.tris + n
	s.worst, s.mostTris = math.max(s.worst, ms), math.max(s.mostTris, n)
	if not ok or not IsValid(phys) then
		gmsky.Log(string.format("chunks: %s: PhysicsFromMesh FAILED (%s, physics object %s), %d triangles", name, tostring(ok), tostring(IsValid(phys)), n))
	else
		-- The address lets a crash dump's register be matched to a chunk.
		gmsky.Log(string.format("chunks: built %s (%s): %d triangles, %.1f ms (+ %.1f ms of step logging); flag %s; CONSTRAINT_STATIC %s, NO_SELF_COLLISIONS %s, motion %s at the end",
			name, tostring(gmsky.PhysAddress and gmsky.PhysAddress(phys)), n, ms, logMs, flagFirst and "first" or "last",
			tostring(phys:HasGameFlag(FVPHYSICS_CONSTRAINT_STATIC)), tostring(phys:HasGameFlag(FVPHYSICS_NO_SELF_COLLISIONS)), tostring(phys:IsMotionEnabled())))
	end
	return true
end

function ENT:Initialize()
	self:DrawShadow(false)
	-- The server builds in C.Update, right after spawning; the client in Think.
end

function ENT:OnRemove()
	if not self.gmskyRetired then
		C.Destroyed(self:ChunkKey(), self:ChunkName(), self:GetPhysicsObject(), "entity removed", self:GetPos() + self:GetBoxLo(), self:GetPos() + self:GetBoxHi())
	end
end

-- Server: a chunk goes. Its mesh is recorded and its physics destroyed now, before Remove (which
-- finishes at the end of the tick), so a new mesh of the same chunk built in this same tick
-- already finds the old one in C.StillInEngine.
function C.Retire(ent, why)
	if not IsValid(ent) then return end
	C.Destroyed(ent:ChunkKey(), ent:ChunkName(), ent:GetPhysicsObject(), why, ent:GetPos() + ent:GetBoxLo(), ent:GetPos() + ent:GetBoxHi())
	ent.gmskyRetired = true
	ent:PhysicsDestroy()
	ent:Remove()
end

if CLIENT then
	-- Client builds share a per-frame budget; waits for the engine are logged once per chunk.
	function ENT:Think()
		if not self.gmskyBuilt and self:GetN() > 0 then
			local f = FrameNumber()
			if C.frame ~= f then C.frame, C.spent, C.frameBuilds = f, 0, 0 end
			if C.frameBuilds == 0 or C.spent < BUDGET_MS then
				local t0 = SysTime()
				local ok, why = self:Build()
				C.spent, C.frameBuilds = C.spent + (SysTime() - t0) * 1000, C.frameBuilds + 1
				if not ok then
					C.stats.engineWaits = C.stats.engineWaits + 1
					if not self.gmskyWaitLogged then
						self.gmskyWaitLogged = true
						gmsky.Log(string.format("chunks: %s waits on this side: still in the engine: %s", self:ChunkName(), why))
					end
				end
			end
		end
		-- Follow the entity (re-centre): the physics object doesn't move by itself.
		local phys = self:GetPhysicsObject()
		if self.gmskyBuilt and IsValid(phys) then
			local want = self:GetPos()
			if phys:GetPos():DistToSqr(want) > 0.0001 then
				phys:SetPos(want)
				C.stats.moved = C.stats.moved + 1
			end
		end
		self:SetNextClientThink(CurTime())
		return true
	end
end

scripted_ents.Register(ENT, "gmsky_chunk")

-- Chunks can't be picked up or removed with tools.
hook.Add("PhysgunPickup", "gmsky_chunks", function(_, ent) if IsValid(ent) and ent:GetClass() == "gmsky_chunk" then return false end end)
hook.Add("CanTool", "gmsky_chunks", function(_, tr) if IsValid(tr.Entity) and tr.Entity:GetClass() == "gmsky_chunk" then return false end end)

-- Once a second, if anything happened: this side's chunk numbers.
function C.StatusLine()
	local s = C.stats
	local built, waiting = 0, 0
	for _, e in ipairs(ents.FindByClass("gmsky_chunk")) do
		if e.gmskyBuilt then built = built + 1 else waiting = waiting + 1 end
	end
	local dead = 0
	for _ in pairs(C.dead) do dead = dead + 1 end
	local line = string.format("chunks: %d entities (%d built, %d not built yet), %d old meshes still in the engine; flag %s; this second: %d builds (%d triangles, most %d, slowest %.1f ms), %d engine waits, %d physics moved to their entity%s%s",
		built + waiting, built, waiting, dead, C.flagFirst:GetBool() and "first" or "last", s.builds, s.tris, s.mostTris, s.worst, s.engineWaits, s.moved,
		SERVER and string.format(", most records a new chunk had with other chunks %d", s.worstChunkRecords or 0) or "",
		C.mismatches and string.format(", fingerprint mismatches so far %d", C.mismatches) or "")
	s.builds, s.tris, s.mostTris, s.worst, s.engineWaits, s.moved, s.worstChunkRecords = 0, 0, 0, 0, 0, 0, 0
	return line
end

if CLIENT then
	timer.Create("gmsky_chunks_status", 1, 0, function()
		if #ents.FindByClass("gmsky_chunk") == 0 and next(C.dead) == nil then return end
		gmsky.Log(C.StatusLine())
	end)
end

-- ---- the server: which chunks exist ------------------------------------------------------------

if SERVER then
	C.size = C.size or CreateConVar("gmsky_chunk_regions", "2", FCVAR_ARCHIVE,
		"gmsky_ground physics: chunk size in Skyrim regions per side (read when the chunks are cleared)")
	C.N = C.N or math.max(1, math.floor(C.size:GetFloat()))
	C.waitingRegions = 0

	-- Removes every chunk (new origin, a switch of ground mode).
	function C.Clear(why)
		local n = 0
		for _, rec in pairs(C.chunks) do
			if IsValid(rec.ent) then C.Retire(rec.ent, why) n = n + 1 end
		end
		C.chunks = {}
		C.N = math.max(1, math.floor(C.size:GetFloat()))
		if n > 0 then gmsky.Log(string.format("chunks: all %d chunk entities removed (%s); chunk size now %d regions", n, why, C.N)) end
	end

	-- The chunk box (relative to the anchor) of chunk (cx, cy, cz).
	local function Box(bx, by, bz, size, cx, cy, cz)
		local w = C.N * size
		local lo = Vector(bx + cx * w, by + cy * w, bz + cz * w)
		return lo, lo + Vector(w, w, w)
	end

	-- Every chunk overlapping the player's footprint (from 64 below its feet to 72 above) is
	-- built or known empty. p is relative to the anchor.
	function C.ReadyAt(p)
		local bx, by, bz, size = gmsky.ShapesLattice()
		if not bx then return false end
		local w = C.N * size
		local function c(v, b) return math.floor((v - b) / w) end
		for cx = c(p.x - 16, bx), c(p.x + 16, bx) do
			for cy = c(p.y - 16, by), c(p.y + 16, by) do
				for cz = c(p.z - 64, bz), c(p.z + 72, bz) do
					local rec = C.chunks[Key(cx, cy, cz)]
					if not rec or not (rec.empty or rec.big or (IsValid(rec.ent) and rec.ent.gmskyBuilt)) then return false end
				end
			end
		end
		return true
	end

	-- Re-centre: every chunk and its physics to the new anchor in the same tick. The shift is at
	-- least gmsky_recentre_at (8,000) units, further than the chunks reach, so no chunk lands on
	-- one that hasn't moved yet.
	function C.MoveAll(anchor)
		local n, worst = 0, 0
		for _, rec in pairs(C.chunks) do
			if IsValid(rec.ent) then
				rec.ent:SetPos(anchor)
				local phys = rec.ent:GetPhysicsObject()
				if IsValid(phys) then
					phys:SetPos(anchor)
					worst = math.max(worst, phys:GetPos():Distance(anchor))
				end
				n = n + 1
			end
		end
		gmsky.Log(string.format("chunks: re-centre: %d chunks and their physics moved to the new anchor (%.1f, %.1f, %.1f); largest physics position error after the move %.3f units",
			n, anchor.x, anchor.y, anchor.z, worst))
	end

	-- ---- S2e: the stuck detector (logging only) ----
	-- A movement key held, not noclip, not held by terrain.lua, and less than STUCK_MOVE units of
	-- horizontal movement a tick for STUCK_TICKS ticks: one STUCK line with what is around the
	-- player, and one "unstuck" line when it ends.
	local STUCK_MOVE, STUCK_TICKS = 0.5, 20
	C.stuck = C.stuck or { ticks = 0, episodes = 0 }

	local function TraceText(tr)
		if tr.StartSolid then return string.format("starts inside %s", IsValid(tr.Entity) and tr.Entity:GetClass() or tr.HitWorld and "the world" or "?") end
		if not tr.Hit then return "clear" end
		return string.format("hit %s at %.0f%%, normal z %.2f", IsValid(tr.Entity) and tr.Entity:GetClass() or tr.HitWorld and "the world" or "?", tr.Fraction * 100, tr.HitNormal.z)
	end

	-- Skyrim's triangles around the player, sorted into kinds, heights relative to the feet.
	-- The flags (GMSKY_TRI_*): 1 = one of Skyrim's stair ramps, 2 = a stair material (S2i-1;
	-- counted on their own, whatever their kind).
	local function TrisText(ply, a)
		if not gmsky.TrisInfo then return "no gmsky.TrisInfo" end
		local p, mins, maxs = ply:GetPos() - a, ply:OBBMins(), ply:OBBMaxs()
		local lo, hi = p + mins - Vector(24, 24, 8), p + maxs + Vector(24, 24, 8)
		local list, n = gmsky.TrisInfo(lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, 4000)
		local step, head = ply:GetStepSize(), maxs.z
		local k = { ramps = 0, rampLo = math.huge, rampHi = -math.huge, walls = 0, highWalls = 0, floors = 0, steps = 0, ledges = 0, ceil = 0, ceilLo = math.huge,
			stairs = 0, stairLo = math.huge, stairHi = -math.huge, stairFloors = 0, stairWalls = 0 }
		for i = 1, n * 6, 6 do
			local zlo, zhi, nz, flags = list[i] - p.z, list[i + 1] - p.z, list[i + 4], list[i + 5]
			local ramp = bit.band(flags, 1) ~= 0
			if bit.band(flags, 2) ~= 0 then
				k.stairs = k.stairs + 1
				k.stairLo, k.stairHi = math.min(k.stairLo, zlo), math.max(k.stairHi, zhi)
				if nz >= 0.7 then k.stairFloors = k.stairFloors + 1 elseif nz > -0.7 then k.stairWalls = k.stairWalls + 1 end
			end
			if ramp then
				k.ramps = k.ramps + 1
				k.rampLo, k.rampHi = math.min(k.rampLo, zlo), math.max(k.rampHi, zhi)
			elseif nz >= 0.7 then
				k.floors = k.floors + 1
				if zhi > 0.5 and zhi <= step then k.steps = k.steps + 1 elseif zhi > step and zlo < head then k.ledges = k.ledges + 1 end
			elseif nz <= -0.7 then
				k.ceil = k.ceil + 1
				k.ceilLo = math.min(k.ceilLo, zlo)
			else
				k.walls = k.walls + 1
				if zhi > step and zlo < head then k.highWalls = k.highWalls + 1 end
			end
		end
		return string.format("Skyrim triangles within 24 units: %d; stair ramps %d%s; stair material %d%s; walls %d (%d from above step height %.0f to head height %.0f); floors %d (%d between 0.5 and step height above the feet, %d higher but below the head); ceilings %d%s",
			n, k.ramps, k.ramps > 0 and string.format(" (z %.0f to %.0f above the feet)", k.rampLo, k.rampHi) or "",
			k.stairs, k.stairs > 0 and string.format(" (z %.0f to %.0f above the feet; %d flat, %d steep)", k.stairLo, k.stairHi, k.stairFloors, k.stairWalls) or "", k.walls, k.highWalls, step, head,
			k.floors, k.steps, k.ledges, k.ceil, k.ceil > 0 and string.format(" (lowest %.0f above the feet)", k.ceilLo) or "")
	end

	local function StuckReport(ply, a, wish)
		local pos, mins, maxs = ply:GetPos(), ply:OBBMins(), ply:OBBMaxs()
		local function Hull(from, to)
			return util.TraceHull({ start = from, endpos = to, mins = mins, maxs = maxs, filter = ply, mask = MASK_PLAYERSOLID })
		end
		local parts = { "here: " .. TraceText(Hull(pos, pos)) }
		for _, up in ipairs({ 0, ply:GetStepSize(), 36 }) do
			local from = pos + Vector(0, 0, up)
			parts[#parts + 1] = string.format("ahead from %.0f up: %s", up, TraceText(Hull(from, from + wish * 24)))
		end
		local g = ply:GetGroundEntity()
		gmsky.Log(string.format("chunks: STUCK #%d for %d ticks at (%.1f, %.1f, %.1f), relative to the anchor (%.1f, %.1f, %.1f): on ground %s (%s), speed (%.0f, %.0f, %.0f), wish direction (%.2f, %.2f); %s; %s",
			C.stuck.episodes, C.stuck.ticks, pos.x, pos.y, pos.z, pos.x - a.x, pos.y - a.y, pos.z - a.z, tostring(ply:IsOnGround()),
			IsValid(g) and g:GetClass() or g == game.GetWorld() and "the world" or "nothing", ply:GetVelocity().x, ply:GetVelocity().y, ply:GetVelocity().z,
			wish.x, wish.y, table.concat(parts, "; "), TrisText(ply, a)))
	end

	function C.StuckCheck(ply, a)
		local st = C.stuck
		local f = (ply:KeyDown(IN_FORWARD) and 1 or 0) - (ply:KeyDown(IN_BACK) and 1 or 0)
		local r = (ply:KeyDown(IN_MOVERIGHT) and 1 or 0) - (ply:KeyDown(IN_MOVELEFT) and 1 or 0)
		local noclip = ply:GetMoveType() == MOVETYPE_NOCLIP
		local held = gmsky.terrain and gmsky.terrain.holding
		local pos = ply:GetPos()
		local moved = st.last and Vector(pos.x - st.last.x, pos.y - st.last.y, 0):Length() or math.huge
		st.last = pos
		local why
		if f == 0 and r == 0 then why = "keys released"
		elseif noclip then why = "noclip"
		elseif held then why = "held by the ground hold"
		elseif moved >= STUCK_MOVE then why = string.format("moving again (%.1f units this tick)", moved) end
		if why then
			if st.ticks >= STUCK_TICKS then
				gmsky.Log(string.format("chunks: unstuck #%d after %d ticks: %s; now at (%.1f, %.1f, %.1f)", st.episodes, st.ticks, why, pos.x, pos.y, pos.z))
			end
			st.ticks = 0
			return
		end
		st.ticks = st.ticks + 1
		if st.ticks == STUCK_TICKS then
			st.episodes = st.episodes + 1
			local ang = Angle(0, ply:EyeAngles().y, 0)
			local wish = ang:Forward() * f + ang:Right() * r
			wish:Normalize()
			StuckReport(ply, a, wish)
		end
	end

	-- Every tick in physics mode (terrain.lua). a = the anchor.
	function C.Update(ply, a)
		C.StuckCheck(ply, a)
		local bx, by, bz, size = gmsky.ShapesLattice()
		if not bx then return end
		local p = ply:GetPos() - a
		local N = C.N
		local function r(v, b) return math.floor((v - b) / size) end
		local prx, pry, prz = r(p.x, bx), r(p.y, by), r(p.z, bz)
		local function c(region) return math.floor(region / N) end
		local x0, x1 = c(prx - RADIUS), c(prx + RADIUS)
		local y0, y1 = c(pry - RADIUS), c(pry + RADIUS)
		local z0, z1 = c(prz - BELOW), c(prz + ABOVE)
		local todo = {}
		C.waitingRegions = 0
		for cx = x0, x1 do
			for cy = y0, y1 do
				for cz = z0, z1 do
					local key = Key(cx, cy, cz)
					local arrived, total, ver = gmsky.ShapesChunkVersion(cx * N, cy * N, cz * N, cx * N + N - 1, cy * N + N - 1, cz * N + N - 1)
					local rec = C.chunks[key]
					if arrived < total then
						if not rec then C.waitingRegions = C.waitingRegions + 1 end
					elseif not rec or rec.ver ~= ver then
						if rec then
							-- Skyrim's triangles here changed: the old mesh goes, the new one waits
							-- until the engine has let go of it.
							C.stats.rebuilds = C.stats.rebuilds + 1
							gmsky.Log(string.format("chunks: chunk %s changed (fingerprint %s -> %s): rebuilding; player %s",
								key, tostring(rec.ver), ver, rec.empty and "-" or string.format("%.0f units from it", (Vector(cx + 0.5, cy + 0.5, cz + 0.5) * N * size + Vector(bx, by, bz)):Distance(p))))
							C.Retire(rec.ent, "its triangles changed")
							C.chunks[key] = nil
						end
						local lo, hi = Box(bx, by, bz, size, cx, cy, cz)
						local d = math.max(lo.x - p.x, 0, p.x - hi.x) ^ 2 + math.max(lo.y - p.y, 0, p.y - hi.y) ^ 2 + math.max(lo.z - p.z, 0, p.z - hi.z) ^ 2
						todo[#todo + 1] = { key = key, cx = cx, cy = cy, cz = cz, ver = ver, lo = lo, hi = hi, d = d }
					end
				end
			end
		end
		-- Nearest first, until the budget is spent (at least one build).
		table.sort(todo, function(u, v) return u.d < v.d end)
		local t0, built = SysTime(), 0
		for _, t in ipairs(todo) do
			if built > 0 and (SysTime() - t0) * 1000 >= BUDGET_MS then break end
			local still = C.StillInEngine(t.key, a + t.lo, a + t.hi)
			if still then
				C.stats.engineWaits = C.stats.engineWaits + 1
			else
				local flat, n = gmsky.TrisInBox(t.lo.x, t.lo.y, t.lo.z, t.hi.x, t.hi.y, t.hi.z, MAX_TRIS + 1, true)
				if n == 0 then
					C.chunks[t.key] = { ver = t.ver, empty = true }
					C.stats.empties = C.stats.empties + 1
				elseif n > MAX_TRIS then
					C.chunks[t.key] = { ver = t.ver, big = true }
					gmsky.Log(string.format("chunks: chunk %s has %d triangles, more than %d: NOT built", t.key, n, MAX_TRIS))
				else
					local e = ents.Create("gmsky_chunk")
					e:SetCX(t.cx) e:SetCY(t.cy) e:SetCZ(t.cz) e:SetN(N)
					e:SetVersion(t.ver)
					e:SetBoxLo(t.lo) e:SetBoxHi(t.hi)
					e:SetPos(a)
					e:SetAngles(Angle(0, 0, 0))
					e:Spawn()
					e:Build(flat, n)
					C.chunks[t.key] = { ent = e, ver = t.ver, tris = n }
					built = built + 1
				end
			end
		end
		-- Chunks more than one chunk outside the window go.
		for key, rec in pairs(C.chunks) do
			local cx, cy, cz = key:match("^(%-?%d+) (%-?%d+) (%-?%d+)$")
			cx, cy, cz = tonumber(cx), tonumber(cy), tonumber(cz)
			if cx < x0 - 1 or cx > x1 + 1 or cy < y0 - 1 or cy > y1 + 1 or cz < z0 - 1 or cz > z1 + 1 then
				C.Retire(rec.ent, "outside Skyrim's collection window")
				C.chunks[key] = nil
				C.stats.removed = C.stats.removed + 1
			end
		end
		-- The entities stay at the anchor (a re-centre moves them in C.MoveAll; this catches anything else).
		for _, rec in pairs(C.chunks) do
			if IsValid(rec.ent) and rec.ent:GetPos():DistToSqr(a) > 0.0001 then C.MoveAll(a) break end
		end
	end

	-- The server's once-a-second line (terrain.lua's status).
	function C.ServerStatus()
		local empty, big = 0, 0
		for _, rec in pairs(C.chunks) do
			if rec.empty then empty = empty + 1 elseif rec.big then big = big + 1 end
		end
		local s = C.stats
		local line = string.format("%s; empty chunks %d, too big %d, chunks waiting for regions %d; this second: %d rebuilds, %d removed, %d empties found; stuck so far %d",
			C.StatusLine(), empty, big, C.waitingRegions, s.rebuilds, s.removed, s.empties, C.stuck.episodes)
		s.rebuilds, s.removed, s.empties = 0, 0, 0
		return line
	end

	-- S2b's stress test: every 0.5 s one built chunk touching the player's is rebuilt (retired and
	-- built again from the same triangles, as if Skyrim had changed them), next to its live
	-- neighbours. Not the player's own chunk. `gmsky_chunks_stress` toggles; 1 / 0 sets.
	C.stress = C.stress or { on = false, n = 0 }
	concommand.Add("gmsky_chunks_stress", function(ply, _, args)
		if IsValid(ply) and not ply:IsListenServerHost() then return end
		local want = args[1] and args[1] ~= "0" or (not args[1] and not C.stress.on)
		C.stress.on = want
		gmsky.Log(string.format("chunks: stress %s (flag %s)", want and "ON: a chunk next to the player's rebuilt every 0.5 s" or "OFF", C.flagFirst:GetBool() and "first" or "last"))
		print("gmsky_chunks_stress: " .. (want and "on" or "off"))
	end)
	timer.Create("gmsky_chunks_stress", 0.5, 0, function()
		if not C.stress.on then return end
		local ply = player.GetAll()[1]
		local bx, by, bz, size = gmsky.ShapesLattice()
		if not IsValid(ply) or not bx then return end
		local a
		for _, rec in pairs(C.chunks) do if IsValid(rec.ent) then a = rec.ent:GetPos() break end end
		if not a then return end
		local p = ply:GetPos() - a
		local w = C.N * size
		local px, py, pz = math.floor((p.x - bx) / w), math.floor((p.y - by) / w), math.floor((p.z - bz) / w)
		local cands = {}
		for dx = -1, 1 do
			for dy = -1, 1 do
				for dz = -1, 1 do
					local key = Key(px + dx, py + dy, pz + dz)
					local rec = C.chunks[key]
					if (dx ~= 0 or dy ~= 0 or dz ~= 0) and rec and IsValid(rec.ent) and rec.ent.gmskyBuilt then cands[#cands + 1] = key end
				end
			end
		end
		if #cands == 0 then return end
		table.sort(cands)
		C.stress.n = C.stress.n + 1
		local key = cands[(C.stress.n - 1) % #cands + 1]
		local rec = C.chunks[key]
		gmsky.Log(string.format("chunks: stress rebuild %d: chunk %s (%d triangles), player in chunk %s, %d candidates",
			C.stress.n, key, rec.tris or 0, Key(px, py, pz), #cands))
		C.Retire(rec.ent, "stress rebuild")
		C.chunks[key] = nil
	end)

	-- Every 5 s: the biggest pair-record lists (a list growing towards 65,532 is seen early).
	timer.Create("gmsky_chunks_lists", 5, 0, function()
		if next(C.chunks) == nil or not gmsky.physground or not gmsky.physground.ListsLine then return end
		gmsky.Log("chunks: pair-record" .. gmsky.physground.ListsLine(nil):gsub("^; ", " "))
	end)
end
