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
		ply:EyePos().z - pos.z, vel.x, vel.y, vel.z, flags, ply:GetFOV())
end)

-- Input typed into Skyrim's window (Phase 1 step 1). While Skyrim routes input here, it replaces
-- the player's own command: buttons, movement and view angles. The mouse is turned into view
-- angles here, with Garry's Mod's own formula (degrees = counts * 0.022 * sensitivity).
local sensitivity = CreateConVar("gmsky_sensitivity", "3", FCVAR_ARCHIVE,
	"Mouse sensitivity for input from Skyrim's window (same scale as Garry's Mod's sensitivity)")
local IN_MAP = { -- GMSKY_IN_* bit -> Garry's Mod IN_* button
	{ 0x1, IN_FORWARD }, { 0x2, IN_BACK }, { 0x4, IN_MOVELEFT }, { 0x8, IN_MOVERIGHT },
	{ 0x10, IN_JUMP }, { 0x20, IN_DUCK }, { 0x40, IN_SPEED }, { 0x80, IN_WALK },
	{ 0x100, IN_ATTACK }, { 0x200, IN_ATTACK2 }, { 0x400, IN_RELOAD },
}
local FULL = 10000 -- "full stick"; Garry's Mod clamps it to the player's speed
local view -- the view angles we drive while input comes from Skyrim
local inputStats = { cmds = 0, dx = 0, dy = 0, buttons = 0 }

hook.Add("StartCommand", "gmsky_input", function(ply, cmd)
	if ply ~= GetPlayer() then return end
	local active, buttons, dx, dy = gmsky.ReadInput()
	if not active then
		if view then gmsky.Log("input from Skyrim: OFF") end
		view = nil
		return
	end
	if not view then
		view = ply:EyeAngles()
		view.r = 0
		gmsky.Log("input from Skyrim: ON")
	end
	local k = 0.022 * sensitivity:GetFloat()
	view.y = math.NormalizeAngle(view.y - dx * k)
	view.p = math.Clamp(view.p + dy * k, -89, 89)

	local b = 0
	for _, m in ipairs(IN_MAP) do
		if bit.band(buttons, m[1]) ~= 0 then b = bit.bor(b, m[2]) end
	end
	local fwd, side = 0, 0
	if bit.band(buttons, 0x1) ~= 0 then fwd = fwd + FULL end
	if bit.band(buttons, 0x2) ~= 0 then fwd = fwd - FULL end
	if bit.band(buttons, 0x8) ~= 0 then side = side + FULL end
	if bit.band(buttons, 0x4) ~= 0 then side = side - FULL end

	cmd:ClearButtons()
	cmd:ClearMovement()
	cmd:SetButtons(b)
	cmd:SetForwardMove(fwd)
	cmd:SetSideMove(side)
	cmd:SetViewAngles(view)

	inputStats.cmds = inputStats.cmds + 1
	inputStats.dx = inputStats.dx + dx
	inputStats.dy = inputStats.dy + dy
	inputStats.buttons = buttons
end)

-- Once a second: write what we're sending to the log, so it can be checked from numbers.
local lastWrites = 0
timer.Create("gmsky_status", 1, 0, function()
	local connected, epoch, skyrimFrame, writes, err = gmsky.Status()
	local ply = GetPlayer()
	local where = "no player"
	if ply then
		local p, a = ply:GetPos(), ply:EyeAngles()
		where = string.format("pos=(%.1f, %.1f, %.1f) yaw=%.1f pitch=%.1f eye=%.1f ground=%s",
			p.x, p.y, p.z, a.y, a.p, ply:EyePos().z - p.z, tostring(ply:IsOnGround()))
		-- Convention check: if yaw 0 = +X and yaw grows counter-clockwise, then walking forward
		-- moves along atan2(vy, vx) and facing-minus-moving is about 0.
		local v = ply:GetVelocity()
		local speed = math.sqrt(v.x * v.x + v.y * v.y)
		if speed > 30 then
			local moveYaw = math.deg(math.atan2(v.y, v.x))
			local diff = (a.y - moveYaw + 180) % 360 - 180
			where = where .. string.format(" moving yaw=%.1f speed=%.0f facing-minus-moving=%.1f", moveYaw, speed, diff)
		end
	end
	local input = view and string.format(" input=ON cmds/s=%d buttons=0x%x mouse=(%d, %d)",
		inputStats.cmds, inputStats.buttons, inputStats.dx, inputStats.dy) or " input=off"
	gmsky.Log(string.format("tick=%d %s skyrim=%s epoch=%d skyrim_frame=%d writes/s=%d%s%s",
		engine.TickCount(), where, connected and "CONNECTED" or "not connected", epoch, skyrimFrame,
		writes - lastWrites, input, err and (" error=" .. err) or ""))
	lastWrites = writes
	inputStats.cmds, inputStats.dx, inputStats.dy = 0, 0, 0
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
