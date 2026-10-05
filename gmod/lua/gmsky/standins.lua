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
-- Step 3d (the user's choice: a flat x3 for now, head x2, nothing else scaled).
S.damageScale = S.damageScale or CreateConVar("gmsky_damage_scale", "3", FCVAR_ARCHIVE,
	"Skyrim damage = Garry's Mod damage x this (hits on Skyrim's NPCs)")
S.headScale = S.headScale or CreateConVar("gmsky_headshot_scale", "2", FCVAR_ARCHIVE,
	"Extra damage multiplier for head shots on Skyrim's NPCs")
-- Step 3d-2 (after test 63): Skyrim's ragdolls are pushed with Garry's Mod's own damage force.
S.ragdollForce = S.ragdollForce or CreateConVar("gmsky_ragdoll_force", "3", FCVAR_ARCHIVE,
	"Push on Skyrim's corpses = Garry's Mod's damage force x this (1 = as Garry's Mod pushes its own ragdolls; 3 chosen by the user, test 64)")
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
	-- Garry's Mod damage types (DMG_*) -> what kind of hit Skyrim plays (GMSKY_HITK_*).
	local DMG_KIND = { { 64, 3 }, { 128, 2 }, { 4, 2 }, { 2, 1 }, { 0x20000000, 1 } } -- blast, club, slash, bullet, buckshot
	local function KindOf(t)
		for _, k in ipairs(DMG_KIND) do
			if bit.band(t, k[1]) ~= 0 then return k[2] end
		end
		return 0
	end
	local KIND_NAMES = { [0] = "other", "bullet", "melee", "explosion" }

	-- Step 3d: the hit goes to Skyrim (gmsky.SendHit), scaled (gmsky_damage_scale, head x
	-- gmsky_headshot_scale). The body part is the one nearest to where the damage landed;
	-- explosions have none (their damage is placed at the blast).
	function ENT:OnTakeDamage(dmg)
		local id = self:FormId()
		local pos = dmg:GetDamagePosition()
		local group, bone, dist = gmsky.HitPartAt(id, pos.x, pos.y, pos.z)
		local kind = KindOf(dmg:GetDamageType())
		if kind == 3 then group, bone = 0, 255 end
		local gdmg = dmg:GetDamage()
		local sdmg = gdmg * S.damageScale:GetFloat() * (group == 1 and S.headScale:GetFloat() or 1)
		local force = dmg:GetDamageForce()
		local fl = force:Length()
		local dir = fl > 0.001 and force * (1 / fl) or nil
		if not dir then
			local att0 = dmg:GetAttacker()
			local from = IsValid(att0) and (att0.EyePos and att0:EyePos() or att0:GetPos()) or pos
			local d = pos - from
			local l = d:Length()
			dir = l > 0.001 and d * (1 / l) or Vector(1, 0, 0)
		end
		local sent = gmsky.SendHit(id, sdmg, gdmg, pos.x, pos.y, pos.z, dir.x, dir.y, dir.z, fl * S.ragdollForce:GetFloat(), dmg:GetDamageType(),
			group or 0, bone or 255, kind)
		S.sent = (S.sent or 0) + (sent and 1 or 0)
		local a = gmsky.actors and gmsky.actors.byId[id]
		local att, inf = dmg:GetAttacker(), dmg:GetInflictor()
		local weapon = IsValid(att) and att.GetActiveWeapon and att:GetActiveWeapon()
		S.damaged = (S.damaged or 0) + 1
		if (S.damageLines or 0) < 20 then
			S.damageLines = (S.damageLines or 0) + 1
			gmsky.Log(string.format("STAND-IN HIT %s %08X%s: %s %s, %.1f damage (type %d) -> %.1f Skyrim damage %s, by %s (inflictor %s), %s",
				a and a.name or "?", id, bit.band(self.gmskyFlags or 0, 1) ~= 0 and " (dead)" or "",
				KIND_NAMES[kind], group and S.GroupName(group) or "no part", gdmg, dmg:GetDamageType(), sdmg,
				sent and "sent" or "NOT sent (no link / origin)",
				IsValid(weapon) and weapon:GetClass() or (IsValid(att) and att:GetClass() or "?"),
				IsValid(inf) and inf:GetClass() or "?",
				dist and string.format("%.1f units from the part (bone %d)", dist, bone) or "no hitboxes"))
		end
	end
else
	function ENT:Draw() end -- invisible (not SetNoDraw: that would stop it being sent to the client)
end

scripted_ents.Register(ENT, "gmsky_actor")

-- ---- an NPC walking into the player shoves them (after test 58; the user's choice) ---------------
-- A trace that starts inside a hull doesn't hit it (so nobody gets stuck), which let NPCs walk
-- through the player. Now, while the player's box overlaps a living NPC's hull, the player gets a
-- speed out of it along the shorter way, scaled by how deep (out in about 0.08 s, at most 300
-- units/s). Garry's Mod's own movement applies it, so it never pushes the player into a wall, and
-- ground friction stops them once clear. Both sides run it (movement prediction).
S.shoveTime = 0.08
S.shoveMax = 300
S.shoves = S.shoves or { ticks = 0, deepest = 0 }

hook.Add("SetupMove", "gmsky_actor_shove", function(ply, mv)
	if not S.solid:GetBool() or ply:GetMoveType() == MOVETYPE_NOCLIP then return end
	local mins, maxs = ply:GetHull()
	if ply:Crouching() then mins, maxs = ply:GetHullDuck() end
	local o = mv:GetOrigin()
	local dx, dy, depth = gmsky.HitPushOut(o.x, o.y, o.z, maxs.x, maxs.z - mins.z)
	if not dx then return end
	local want = math.min(depth / S.shoveTime, S.shoveMax)
	local v = mv:GetVelocity()
	local along = v.x * dx + v.y * dy
	if along < want then
		v.x = v.x + dx * (want - along)
		v.y = v.y + dy * (want - along)
		mv:SetVelocity(v)
	end
	S.shoves.ticks = S.shoves.ticks + 1
	S.shoves.deepest = math.max(S.shoves.deepest, depth)
end)

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

	-- Step 3c-3: gmsky_hitbox_test (aim at an NPC): Garry's Mod's copy of its hitboxes against
	-- Skyrim's own, ray by ray (DESIGN.md "3c-3 in detail"). The report goes to the log and console.
	concommand.Add("gmsky_hitbox_test", function(ply)
		ply = IsValid(ply) and ply or player.GetAll()[1]
		if not IsValid(ply) then return end
		local eye = ply:EyePos()
		local tr = util.TraceLine({ start = eye, endpos = eye + ply:GetAimVector() * 8192, filter = ply, mask = MASK_SHOT })
		local e = tr.Entity
		if not IsValid(e) or e:GetClass() ~= "gmsky_actor" then
			print("[gmsky] hitbox test: aim at a Skyrim NPC first")
			return
		end
		local f = ply:GetAimVector()
		local id, why = gmsky.HitTestStart(e:FormId(), f.x, f.y, f.z)
		local a = gmsky.actors and gmsky.actors.byId[e:FormId()]
		local line = id and string.format("hitbox test started on %s %08X", a and a.name or "?", e:FormId())
			or ("hitbox test not started: " .. tostring(why))
		gmsky.Log(line)
		print("[gmsky] " .. line)
	end)

	hook.Add("Tick", "gmsky_hitbox_test", function()
		local text = gmsky.HitTestReport()
		if text then
			gmsky.Log(text)
			print("[gmsky] " .. text)
		end
	end)

	-- Once a second (server.lua's status line).
	function S.Status(ply)
		local n = 0
		for _ in pairs(S.ents) do n = n + 1 end
		local s, c = S.stats, S.counts
		local line = ""
		if n > 0 or c.removed > 0 then
			line = string.format(" | stand-ins %d (+%d -%d, table %.0f ms old), traces %d lines %d boxes %d hits %.2f ms, shoved %d ticks (deepest %.1f), damage taken %d (sent to Skyrim %d), aim: %s",
				n, c.made, c.removed, S.age or -1, s.rays, s.boxes, s.hits, s.ms, S.shoves.ticks, S.shoves.deepest, S.damaged or 0, S.sent or 0, ply and AimTarget(ply) or "-")
		end
		S.stats = { rays = 0, boxes = 0, hits = 0, ms = 0 }
		S.counts = { made = 0, removed = 0 }
		S.shoves = { ticks = 0, deepest = 0 }
		S.damaged, S.damageLines, S.sent = 0, 0, 0
		return line
	end
end

gmsky.Log("standins.lua loaded")
