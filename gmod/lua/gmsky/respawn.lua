-- gmsky server side, Phase 3 step 3f: dying works like Garry's Mod (the user, after test 61): you
-- respawn, no Skyrim save reload. You come back at the last calm spot: where you last stood while
-- nobody was fighting you (Skyrim's GMSKY_SF_IN_COMBAT off, no hurt in the last 3 s). Skyrim keeps
-- its player essential at the body while you're dead and follows you to the calm spot.
-- DESIGN.md "3f in detail".

local gmsky = gmsky

local R = gmsky.respawn or {}
gmsky.respawn = R
R.every = 0.5     -- seconds between calm spot updates
R.hurtQuiet = 3   -- no calm spot this long after a hurt from Skyrim
R.calm = R.calm   -- { rel = Vector (relative to the anchor), epoch, yaw, time }
R.nextAt = R.nextAt or 0
R.stats = R.stats or { updates = 0 }

local function Rel(v, ax, ay, az) return Vector(v.x - ax, v.y - ay, v.z - az) end
-- The origin epoch Garry's Mod is anchored to (what positions relative to the anchor belong to).
local function Anchored() local _, e = gmsky.Status() return e or 0 end

-- Every tick (server.lua), after the player state is written.
function R.Update(ply, anchored)
	local now = CurTime()
	-- The tick after a respawn: if something put the player back at a spawn point, move them again.
	if R.pending then
		local p = ply:GetPos()
		local off = math.sqrt((p.x - R.pending.x) ^ 2 + (p.y - R.pending.y) ^ 2 + (p.z - R.pending.z) ^ 2)
		if off > 64 then
			ply:SetPos(R.pending)
			gmsky.Log(string.format("RESPAWN: the player was %.0f units from the spot a tick later; moved again", off))
		end
		R.pending = nil
	end
	local flags = gmsky.SkyrimFlags()
	local fighting = flags ~= nil and bit.band(flags, 1) ~= 0
	if fighting ~= R.fighting then
		R.fighting = fighting
		if fighting then
			gmsky.Log(R.calm and string.format("respawn: a fight started; calm spot kept from %.1f s ago", now - R.calm.time)
				or "respawn: a fight started; no calm spot yet")
		else
			gmsky.Log("respawn: calm again")
		end
	end
	if now < R.nextAt then return end
	R.nextAt = now + R.every
	local T = gmsky.terrain
	local hurt = gmsky.hurt and gmsky.hurt.lastHurt
	if not ply:Alive() or not ply:IsOnGround() or ply:GetMoveType() == MOVETYPE_NOCLIP or (T and T.holding)
		or flags == nil or fighting or (anchored or 0) == 0 or (hurt and now - hurt < R.hurtQuiet) then
		return
	end
	local ax, ay, az = gmsky.Anchor()
	if not ax then return end
	R.calm = { rel = Rel(ply:GetPos(), ax, ay, az), epoch = anchored, yaw = ply:EyeAngles().y, time = now }
	R.stats.updates = R.stats.updates + 1
end

hook.Add("PlayerDeath", "gmsky_respawn", function(ply, inflictor, attacker)
	local now = CurTime()
	local ax, ay, az = gmsky.Anchor()
	local pos = ply:GetPos()
	R.died = { rel = ax and Rel(pos, ax, ay, az), epoch = Anchored(), yaw = ply:EyeAngles().y, time = now }
	local away = R.calm and R.died.rel and (R.calm.rel - R.died.rel):Length()
	local info = IsValid(attacker) and attacker.FormId and gmsky.actors and gmsky.actors.byId[attacker:FormId()]
	gmsky.Log(string.format("DIED (killed by %s) at (%.0f, %.0f, %.0f); %s", info and info.name or (IsValid(attacker) and tostring(attacker:GetClass()) or "?"),
		pos.x, pos.y, pos.z,
		R.calm and string.format("last calm spot %.1f s ago, %.0f units away", now - R.calm.time, away or -1) or "no calm spot"))
end)

-- Garry's Mod's own respawn (full health, Sandbox's weapons), then moved to the calm spot. The
-- hook returns nothing, so the gamemode's PlayerSpawn still runs.
hook.Add("PlayerSpawn", "gmsky_respawn", function(ply)
	local died = R.died
	if not died then return end -- the first spawn: the terrain code places the player
	R.died = nil
	local epoch = Anchored()
	local ax, ay, az = gmsky.Anchor()
	local spot, why
	if R.calm and R.calm.epoch == epoch and ax then
		spot, why = R.calm, string.format("the last calm spot (%.1f s before dying)", died.time - R.calm.time)
	elseif died.rel and died.epoch == epoch and ax then
		spot, why = died, "where you died (no calm spot for this origin)"
	end
	if not spot then
		gmsky.Log("RESPAWN: no calm spot and no death spot for this origin: Garry's Mod's own spawn point")
		return
	end
	local target = Vector(ax + spot.rel.x, ay + spot.rel.y, az + spot.rel.z)
	ply:SetPos(target)
	ply:SetEyeAngles(Angle(0, spot.yaw, 0))
	R.pending = target
	gmsky.Log(string.format("RESPAWN at %s: (%.0f, %.0f, %.0f), %.0f units from where you died", why, target.x, target.y, target.z,
		died.rel and (spot.rel - died.rel):Length() or -1))
end)

-- Once a second (server.lua's status timer).
function R.Status()
	local line = ""
	if R.calm then
		line = string.format(" | calm spot %.1f s old%s", CurTime() - R.calm.time, R.fighting and " (in a fight)" or "")
	end
	R.stats = { updates = 0 }
	return line
end

gmsky.Log("respawn.lua loaded")
