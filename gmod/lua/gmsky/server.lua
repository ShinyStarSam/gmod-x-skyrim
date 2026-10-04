-- gmsky server side. Run by the gmsky module after `require("gmsky")` in server Lua.
-- Edit freely; `lua_run gmsky.Reload()` re-runs this file without restarting.
-- Log: logs/gmod_server.log

local gmsky = gmsky

local function GetPlayer()
	-- Single-player: the only player.
	local ply = player.GetAll()[1]
	if IsValid(ply) then return ply end
end

-- Collision (Phase 1 step 3), shared with the client side.
gmsky.Include("terrain")
local T = gmsky.terrain
-- Collision field (re-plan step A): triangles answered from Lua, no physics objects.
gmsky.Include("field")
if GetPlayer() and T then T.home = T.MeasureRoom(GetPlayer():GetPos() + Vector(0, 0, 40)) end
local anchored = 0 -- the origin epoch Garry's Mod is anchored to

local sensitivity = CreateConVar("gmsky_sensitivity", "3", FCVAR_ARCHIVE,
	"Mouse sensitivity for input from Skyrim's window (same scale as Garry's Mod's sensitivity)")
local predict = CreateConVar("gmsky_predict", "1", FCVAR_ARCHIVE,
	"1 = Skyrim draws mouse movement Garry's Mod hasn't applied yet (no round-trip delay), 0 = off")
local smooth = CreateConVar("gmsky_smooth", "1", FCVAR_ARCHIVE,
	"1 = Skyrim blends between Garry's Mod's ticks (even steps), 0 = newest tick each frame")
local driveCamera = CreateConVar("gmsky_camera", "1", FCVAR_ARCHIVE,
	"1 = Skyrim turns its camera straight to the view (no camera easing / frame delay), 0 = Skyrim's own camera")

-- Every server tick: send the player's state to Skyrim.
hook.Add("Tick", "gmsky_player_state", function()
	local ply = GetPlayer()
	if not ply then return end
	-- A new origin from Skyrim: move to the middle of the map first, so the anchor is there (3c).
	if T then T.BeforeAnchor(ply, gmsky.OriginEpoch(), anchored) end
	local pos = ply:GetPos()
	local ang = ply:EyeAngles()
	local vel = ply:GetVelocity()
	local flags = 0
	if ply:IsOnGround() then flags = flags + 1 end
	if ply:Crouching() then flags = flags + 2 end
	if ply:Alive() then flags = flags + 4 end
	if ply:GetMoveType() == MOVETYPE_NOCLIP then flags = flags + 8 end
	if driveCamera:GetBool() then flags = flags + 0x20 end
	if smooth:GetBool() then flags = flags + 0x40 end
	-- Degrees per Skyrim mouse count, so Skyrim can draw the mouse movement still on its way here.
	local degPerCount = predict:GetBool() and 0.022 * sensitivity:GetFloat() or 0
	anchored = gmsky.WritePlayerState(engine.TickCount(), pos.x, pos.y, pos.z, ang.p, ang.y, ang.r,
		ply:EyePos().z - pos.z, vel.x, vel.y, vel.z, flags, ply:GetFOV(), degPerCount,
		engine.TickInterval() * 1000) or 0
	if T then T.UpdateGround(ply, anchored, (gmsky.Status())) end
end)

-- Input typed into Skyrim's window (Phase 1 step 1). While Skyrim routes input here, it replaces
-- the player's own command: buttons, movement and view angles. The mouse is turned into view
-- angles here, with Garry's Mod's own formula (degrees = counts * 0.022 * sensitivity).
local IN_MAP = { -- GMSKY_IN_* bit -> Garry's Mod IN_* button
	{ 0x1, IN_FORWARD }, { 0x2, IN_BACK }, { 0x4, IN_MOVELEFT }, { 0x8, IN_MOVERIGHT },
	{ 0x10, IN_JUMP }, { 0x20, IN_DUCK }, { 0x40, IN_SPEED }, { 0x80, IN_WALK },
	{ 0x100, IN_ATTACK }, { 0x200, IN_ATTACK2 }, { 0x400, IN_RELOAD },
}
local FULL = 10000 -- "full stick"; Garry's Mod clamps it to the player's speed
local HOLD_TIME = 0.5 -- seconds we wait for the Garry's Mod window to take over our view
local view -- the view angles we drive while input comes from Skyrim
-- After input stops (a Skyrim menu opened, alt-tab): keep our last view until the Garry's Mod
-- window's own view has caught up with it, so the player doesn't snap to where that window looked.
local hold
local inputStats = { cmds = 0, dx = 0, dy = 0, buttons = 0 }

-- V in Skyrim's window (step 3e): what Garry's Mod's own `noclip` command does. The gamemode's
-- PlayerNoClip hook decides whether it's allowed (Sandbox: always in single-player).
local function ToggleNoclip(ply)
	local want = ply:GetMoveType() ~= MOVETYPE_NOCLIP
	local allowed = hook.Run("PlayerNoClip", ply, want)
	if allowed then ply:SetMoveType(want and MOVETYPE_NOCLIP or MOVETYPE_WALK) end
	local p = ply:GetPos()
	gmsky.Log(string.format("noclip %s (V in Skyrim) at (%.0f, %.0f, %.0f): %s", want and "ON" or "OFF", p.x, p.y, p.z,
		allowed and "done" or "refused by the PlayerNoClip hook"))
end

local function AngleGap(a, b)
	return math.max(math.abs(math.NormalizeAngle(a.y - b.y)), math.abs(a.p - b.p))
end

hook.Add("StartCommand", "gmsky_input", function(ply, cmd)
	if ply ~= GetPlayer() then return end
	local active, buttons, dx, dy, _, noclipPresses = gmsky.ReadInput()
	if not active then
		if view then
			-- Tell the Garry's Mod window our view (it only had its own, stale one until now).
			ply:SetEyeAngles(view)
			hold = { ang = view, untilTime = CurTime() + HOLD_TIME, ticks = 0 }
			gmsky.Log(string.format("input from Skyrim: OFF, holding yaw=%.1f pitch=%.1f", view.y, view.p))
			view = nil
		end
		if hold then
			local own = cmd:GetViewAngles()
			local gap = AngleGap(own, hold.ang)
			if gap < 1 or CurTime() > hold.untilTime then
				gmsky.Log(string.format("view handed to the Garry's Mod window after %d held ticks "
					.. "(its view yaw=%.1f pitch=%.1f, %.1f degrees from ours)", hold.ticks, own.y, own.p, gap))
				hold = nil
			else
				cmd:SetViewAngles(hold.ang)
				hold.ticks = hold.ticks + 1
			end
		end
		return
	end
	hold = nil
	if not view then
		view = ply:EyeAngles()
		view.r = 0
		gmsky.Log("input from Skyrim: ON")
	end
	for _ = 1, noclipPresses do ToggleNoclip(ply) end
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
	-- Keep the Garry's Mod window's own view in step with ours, so letting go never snaps.
	ply:SetEyeAngles(view)

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
		where = string.format("pos=(%.1f, %.1f, %.1f) yaw=%.1f pitch=%.1f eye=%.1f ground=%s%s",
			p.x, p.y, p.z, a.y, a.p, ply:EyePos().z - p.z, tostring(ply:IsOnGround()),
			ply:GetMoveType() == MOVETYPE_NOCLIP and " NOCLIP" or "")
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
	local terrain = T and (T.Status(ply) .. (ply and T.GroundStatus(ply) or "")) or ""
	if gmsky.field then terrain = terrain .. gmsky.field.Status(ply) end
	gmsky.Log(string.format("tick=%d %s skyrim=%s epoch=%d skyrim_frame=%d writes/s=%d%s%s%s",
		engine.TickCount(), where, connected and "CONNECTED" or "not connected", epoch, skyrimFrame,
		writes - lastWrites, input, err and (" error=" .. err) or "", terrain))
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
