-- gmsky server side, Phase 3 step 3e: Skyrim's NPCs hurt the Garry's Mod player.
-- Skyrim keeps its own player at full health while Garry's Mod drives and puts every hit into a
-- queue (skse/src/Combat.cpp); here each one becomes Garry's Mod damage on the player, from the
-- attacker's stand-in (so armour, pain sounds and the damage direction work as for a Garry's Mod
-- hit). Adapted from SkyCraft's SkyCombat.hurtPlayer (MIT). DESIGN.md "3e in detail".

local gmsky = gmsky

local H = gmsky.hurt or {}
gmsky.hurt = H
H.scale = H.scale or CreateConVar("gmsky_hurt_scale", "1", FCVAR_ARCHIVE,
	"Garry's Mod damage = Skyrim damage x this (Skyrim's NPCs hurting you)")
H.shove = 300 -- units/s away from the attacker on a power attack
H.stats = H.stats or { events = 0, skyrim = 0, gmod = 0, lost = 0, dead = 0 }
H.lines = H.lines or 0

-- GMSKY_HURTK_* -> Garry's Mod damage type and a name for the logs.
local KIND_DMG = { [0] = 4, 2, 8, 0 } -- melee DMG_SLASH, projectile DMG_BULLET, magic DMG_BURN, other DMG_GENERIC
local KIND_NAMES = { [0] = "melee", "projectile", "magic", "other" }

local function Apply(ply, e)
	local S, A = gmsky.standins, gmsky.actors
	local att = S and S.ents and S.ents[e.attacker]
	local info = A and A.byId[e.attacker]
	local centre = ply:WorldSpaceCenter()
	local from
	if IsValid(att) then
		from = att:GetPos()
	elseif info then
		from = Vector(info.x, info.y, info.z + (info.height or 0) * 0.6)
	end
	local dir = from and (centre - from) or Vector(0, 0, 0)
	dir.z = 0
	local len = dir:Length()
	dir = len > 0.001 and dir * (1 / len) or nil
	local amount = e.damage * H.scale:GetFloat()
	local source = IsValid(att) and att or game.GetWorld()
	local dmg = DamageInfo()
	dmg:SetDamage(amount)
	dmg:SetDamageType(KIND_DMG[e.kind] or 0)
	dmg:SetAttacker(source)
	dmg:SetInflictor(source)
	dmg:SetDamagePosition(from or centre)
	dmg:SetDamageForce(dir or Vector(0, 0, 0))
	local before = ply:Health()
	ply:TakeDamageInfo(dmg)
	local shoved = bit.band(e.flags, 2) ~= 0 and dir ~= nil
	if shoved then ply:SetVelocity(dir * H.shove) end
	H.lastHurt = CurTime() -- step 3f: no calm spot right after a hit
	local st = H.stats
	st.events, st.skyrim, st.gmod = st.events + 1, st.skyrim + e.damage, st.gmod + amount
	if H.lines < 20 then
		H.lines = H.lines + 1
		gmsky.Log(string.format("HURT by %s %08X: %.1f Skyrim -> %.1f Garry's Mod damage, %s%s%s, health %d -> %d, armour %d%s",
			info and info.name or "?", e.attacker, e.damage, amount, KIND_NAMES[e.kind] or "?",
			bit.band(e.flags, 2) ~= 0 and ", power attack" or "", bit.band(e.flags, 1) ~= 0 and ", blocked in Skyrim" or "",
			before, ply:Health(), ply:Armor(),
			IsValid(att) and "" or (from and " (no stand-in: from the world, placed at the actor)" or " (attacker unknown)")))
	end
end

-- Every tick (server.lua).
function H.Update(ply)
	local list, lost = gmsky.ReadHurts()
	if not list then return end
	H.stats.lost = H.stats.lost + (lost or 0)
	for _, e in ipairs(list) do
		if IsValid(ply) and ply:Alive() then
			Apply(ply, e)
		else
			H.stats.dead = H.stats.dead + 1 -- dead in Garry's Mod: step 3f
		end
	end
end

-- Once a second (server.lua's status timer).
function H.Status()
	local st = H.stats
	local line = ""
	if st.events > 0 or st.lost > 0 or st.dead > 0 then
		line = string.format(" | hurt %d times (%.1f Skyrim -> %.1f Garry's Mod damage)%s%s", st.events, st.skyrim, st.gmod,
			st.lost > 0 and string.format(", %d lost", st.lost) or "",
			st.dead > 0 and string.format(", %d while dead", st.dead) or "")
	end
	H.stats = { events = 0, skyrim = 0, gmod = 0, lost = 0, dead = 0 }
	H.lines = 0
	return line
end

gmsky.Log("hurt.lua loaded")
