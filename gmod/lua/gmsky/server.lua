-- gmsky server side. Run by the gmsky module after `require("gmsky")` in server Lua.
-- Edit freely; `lua_run gmsky.Reload()` re-runs this file without restarting.
-- Log: logs/gmod_server.log

local gmsky = gmsky

local function GetPlayer()
	-- Single-player: the only player.
	local ply = player.GetAll()[1]
	if IsValid(ply) then return ply end
end

-- Every server tick: send the player's state to Skyrim.
hook.Add("Tick", "gmsky_player_state", function()
	local ply = GetPlayer()
	if not ply then return end
	local pos = ply:GetPos()
	local ang = ply:EyeAngles()
	local vel = ply:GetVelocity()
	local flags = 0
	if ply:IsOnGround() then flags = flags + 1 end
	if ply:Crouching() then flags = flags + 2 end
	if ply:Alive() then flags = flags + 4 end
	if ply:GetMoveType() == MOVETYPE_NOCLIP then flags = flags + 8 end
	gmsky.WritePlayerState(engine.TickCount(), pos.x, pos.y, pos.z, ang.p, ang.y, ang.r,
		ply:GetViewOffset().z, vel.x, vel.y, vel.z, flags, ply:GetFOV())
end)

-- Once a second: write what we're sending to the log, so it can be checked from numbers.
local lastWrites = 0
timer.Create("gmsky_status", 1, 0, function()
	local connected, epoch, skyrimFrame, writes, err = gmsky.Status()
	local ply = GetPlayer()
	local where = "no player"
	if ply then
		local p, a = ply:GetPos(), ply:EyeAngles()
		where = string.format("pos=(%.1f, %.1f, %.1f) yaw=%.1f pitch=%.1f ground=%s",
			p.x, p.y, p.z, a.y, a.p, tostring(ply:IsOnGround()))
	end
	gmsky.Log(string.format("tick=%d %s skyrim=%s epoch=%d skyrim_frame=%d writes/s=%d%s",
		engine.TickCount(), where, connected and "CONNECTED" or "not connected", epoch, skyrimFrame,
		writes - lastWrites, err and (" error=" .. err) or ""))
	lastWrites = writes
end)

-- Load the client side too: new players when they spawn, and anyone already in game now.
local LOAD_CLIENT = 'require("gmsky")'
hook.Add("PlayerInitialSpawn", "gmsky_load_client", function(ply)
	ply:SendLua(LOAD_CLIENT)
end)
for _, ply in ipairs(player.GetAll()) do
	ply:SendLua(LOAD_CLIENT)
end

gmsky.Log("server.lua loaded")
print("[gmsky] server side loaded, logging to " .. gmsky.Root() .. "\\logs\\gmod_server.log")
