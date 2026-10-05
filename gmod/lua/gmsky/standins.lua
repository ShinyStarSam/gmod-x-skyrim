-- gmsky, both sides, Phase 3 step 3c-2: an invisible stand-in entity (`gmsky_actor`) for every
-- Skyrim actor whose hitboxes Skyrim sends (the nearest 24 within 3,000 Skyrim units, living and
-- dead). It has no physics object (tests 12-17: physics objects made near the player crashed
-- vphysics.dll); its traces are answered by the module from Skyrim's own per-bone body shapes
-- (gmod/module/src/hitboxes.rs): a line (bullets) against the body parts, a moving box (player
-- movement) against one fixed upright box at the NPC's feet, like Garry's Mod's own NPCs (after
-- test 57: a capsule around the animated parts felt weird); corpses don't block. Damage it takes is logged with the body
-- part (head, chest, ...); it isn't passed to Skyrim yet (step 3d).
-- DESIGN.md "Hitboxes: how bullets hit Skyrim's NPCs", "3c-2 in detail".

local gmsky = gmsky
local MARGIN = 32 -- bounds slack: the client's copy of the entity trails the server's

local S = gmsky.standins or {}
gmsky.standins = S
S.solid = S.solid or CreateConVar("gmsky_actor_solid", "1", FCVAR_REPLICATED,
	"1 = you bump into Skyrim's NPCs, 0 = you walk through them (bullets hit them either way)")
S.gear = S.gear or CreateConVar("gmsky_hitbox_gear", "0", FCVAR_REPLICATED,
	"1 = a carried bow, weapon or shield counts as a hit on the NPC, 0 = shots pass by it")
S.stats = S.stats or { rays = 0, boxes = 0, hits = 0, ms = 0 }

local GROUPS = { [0] = "generic", "head", "chest", "stomach", "left arm", "right arm", "left leg", "right leg", [10] = "gear" }
function S.GroupName(g) return GROUPS[g] or ("group " .. tostring(g)) end

-- FormIds are unsigned 32-bit; NetworkVar Int is signed.
local function ToSigned(id) return id >= 0x80000000 and id - 0x100000000 or id end
local function ToUnsigned(n) return n < 0 and n + 0x100000000 or n end

-- ---- the entity ---------------------------------------------------------------------------------

local ENT = {}
ENT.Type = "anim"
ENT.Base = "base_anim"
ENT.Spawnable = false

function ENT:SetupDataTables()
	self:NetworkVar("Int", 0, "FormIdSigned")
	self:NetworkVar("Vector", 0, "BoundsLo") -- collision bounds around the entity
	self:NetworkVar("Vector", 1, "BoundsHi")
end

function ENT:FormId() return ToUnsigned(self:GetFormIdSigned()) end

function ENT:UpdateTransmitState()
	return TRANSMIT_ALWAYS -- the client needs it for movement prediction and bullet effects
end

function ENT:SetupCollision()
	self:SetSolid(SOLID_OBB)
	self:SetMoveType(MOVETYPE_NONE)
	self:EnableCustomCollisions(true)
	self:SetCollisionBounds(self:GetBoundsLo(), self:GetBoundsHi())
	self:DrawShadow(false)
	self.gmskyLo, self.gmskyHi = self:GetBoundsLo(), self:GetBoundsHi()
end

function ENT:Initialize()
	self:SetupCollision()
end

function ENT:Think()
	if not CLIENT then return end
	-- The client follows the server's bounds (the shapes come from the module on both sides).
	local lo, hi = self:GetBoundsLo(), self:GetBoundsHi()
	if lo ~= self.gmskyLo or hi ~= self.gmskyHi then self:SetupCollision() end
	self:SetNextClientThink(CurTime())
	return true
end

function ENT:TestCollision(startpos, delta, isbox, extents, mask)
	local t0 = SysTime()
	local ex, ey, ez = 0, 0, 0
	if isbox then
		if not S.solid:GetBool() then return end
		ex, ey, ez = extents.x, extents.y, extents.z
	end
	-- The module wants the box's centre: startpos is its bottom (test 19, gmsky_field_feet 1).
	local cz = startpos.z
	if isbox and gmsky.field and gmsky.field.feet:GetBool() then cz = cz + ez end
	local frac, nx, ny, nz, group = gmsky.HitTrace(self:FormId(), startpos.x, startpos.y, cz,
		delta.x, delta.y, delta.z, ex, ey, ez, S.gear:GetBool())
	local s = S.stats
	if isbox then s.boxes = s.boxes + 1 else s.rays = s.rays + 1 end
	s.ms = s.ms + (SysTime() - t0) * 1000
	if not frac then return end
	s.hits = s.hits + 1
	return { HitPos = startpos + delta * frac, Fraction = frac, Normal = Vector(nx, ny, nz) }
end

if SERVER then
	-- Logged, not passed to Skyrim yet (step 3d). The body part is the one nearest to where the
	-- damage landed.
	function ENT:OnTakeDamage(dmg)
		local id = self:FormId()
		local pos = dmg:GetDamagePosition()
		local group, bone, dist = gmsky.HitPartAt(id, pos.x, pos.y, pos.z)
		local a = gmsky.actors and gmsky.actors.byId[id]
		local att, inf = dmg:GetAttacker(), dmg:GetInflictor()
		local weapon = IsValid(att) and att.GetActiveWeapon and att:GetActiveWeapon()
		S.damaged = (S.damaged or 0) + 1
		if (S.damageLines or 0) < 20 then
			S.damageLines = (S.damageLines or 0) + 1
			gmsky.Log(string.format("STAND-IN HIT %s %08X%s: %s, %.1f damage (type %d) by %s (inflictor %s), %s",
				a and a.name or "?", id, bit.band(self.gmskyFlags or 0, 1) ~= 0 and " (dead)" or "",
				group and S.GroupName(group) or "no part", dmg:GetDamage(), dmg:GetDamageType(),
				IsValid(weapon) and weapon:GetClass() or (IsValid(att) and att:GetClass() or "?"),
				IsValid(inf) and inf:GetClass() or "?",
				dist and string.format("%.1f units from the part (bone %d)", dist, bone) or "no hitboxes"))
		end
	end
else
	function ENT:Draw() end -- invisible (not SetNoDraw: that would stop it being sent to the client)
end

scripted_ents.Register(ENT, "gmsky_actor")

-- ---- the server keeps one per actor -------------------------------------------------------------

if SERVER then
	S.ents = S.ents or {}
	S.seq = S.seq or 0
	S.counts = S.counts or { made = 0, removed = 0 }

	local function Remove(id)
		local e = S.ents[id]
		if IsValid(e) then e:Remove() end
		S.ents[id] = nil
		S.counts.removed = S.counts.removed + 1
	end

	-- Every tick (server.lua).
	function S.Update()
		local seq, age, list = gmsky.HitboxActors(S.seq)
		if seq == false then return end
		if seq == nil then
			for id in pairs(S.ents) do Remove(id) end
			return
		end
		S.seq, S.age = seq, age
		local seen = {}
		for _, a in ipairs(list) do
			seen[a.id] = true
			local lo, hi = Vector(a.lox, a.loy, a.loz), Vector(a.hix, a.hiy, a.hiz)
			local centre = (lo + hi) * 0.5
			local e = S.ents[a.id]
			if not IsValid(e) then
				e = ents.Create("gmsky_actor")
				e:SetFormIdSigned(ToSigned(a.id))
				e:SetPos(centre)
				e:SetBoundsLo(lo - centre - Vector(MARGIN, MARGIN, MARGIN))
				e:SetBoundsHi(hi - centre + Vector(MARGIN, MARGIN, MARGIN))
				e:Spawn()
				S.ents[a.id] = e
				S.counts.made = S.counts.made + 1
				local info = gmsky.actors and gmsky.actors.byId[a.id]
				if S.counts.made <= 40 then
					gmsky.Log(string.format("stand-in for %s %08X: %d parts%s", info and info.name or "?", a.id, a.parts,
						bit.band(a.flags, 1) ~= 0 and " (dead)" or ""))
				end
			else
				e:SetPos(centre)
				e:SetBoundsLo(lo - centre - Vector(MARGIN, MARGIN, MARGIN))
				e:SetBoundsHi(hi - centre + Vector(MARGIN, MARGIN, MARGIN))
				e:SetupCollision()
			end
			e.gmskyFlags = a.flags
			e.gmskyParts = a.parts
		end
		for id in pairs(S.ents) do
			if not seen[id] then Remove(id) end
		end
	end

	-- What's under Garry's Mod's crosshair: the stand-in and body part a bullet would hit.
	local function AimTarget(ply)
		local eye = ply:EyePos()
		local tr = util.TraceLine({ start = eye, endpos = eye + ply:GetAimVector() * 8192, filter = ply, mask = MASK_SHOT })
		local e = tr.Entity
		if not IsValid(e) or e:GetClass() ~= "gmsky_actor" then
			return tr.Hit and "nothing (" .. (IsValid(e) and e:GetClass() or "the world") .. ")" or "nothing"
		end
		local id = e:FormId()
		local group = gmsky.HitPartAt(id, tr.HitPos.x, tr.HitPos.y, tr.HitPos.z)
		local a = gmsky.actors and gmsky.actors.byId[id]
		return string.format("%s %08X %s at %.0f", a and a.name or "?", id, group and S.GroupName(group) or "?", tr.HitPos:Distance(eye))
	end

	-- Once a second (server.lua's status line).
	function S.Status(ply)
		local n = 0
		for _ in pairs(S.ents) do n = n + 1 end
		local s, c = S.stats, S.counts
		local line = ""
		if n > 0 or c.removed > 0 then
			line = string.format(" | stand-ins %d (+%d -%d, table %.0f ms old), traces %d lines %d boxes %d hits %.2f ms, damage taken %d, aim: %s",
				n, c.made, c.removed, S.age or -1, s.rays, s.boxes, s.hits, s.ms, S.damaged or 0, ply and AimTarget(ply) or "-")
		end
		S.stats = { rays = 0, boxes = 0, hits = 0, ms = 0 }
		S.counts = { made = 0, removed = 0 }
		S.damaged, S.damageLines = 0, 0
		return line
	end
end

gmsky.Log("standins.lua loaded")
