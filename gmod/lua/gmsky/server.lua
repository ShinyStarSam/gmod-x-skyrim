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
-- Phase 3 step 3b: Skyrim's actors near the player (log only).
gmsky.Include("actors")
-- Phase 3 step 3c-2: invisible stand-ins with Skyrim's hitboxes.
gmsky.Include("standins")
-- Phase 3 step 3e: Skyrim's NPCs hurt the player.
gmsky.Include("hurt")
-- Phase 3 step 3f: dying respawns you at the last calm spot.
gmsky.Include("respawn")
-- Phase 4 step 4b: swimming in Skyrim's water (both sides).
gmsky.Include("water")
-- Geometry route step G6: marks what the player holds, so Skyrim places it relative to its camera.
gmsky.Include("render")
-- Step S0 (one collision system): physics objects against Skyrim's ground (log only).
gmsky.Include("props")
-- Step S2 (one collision system): Skyrim's collision as physics chunks (gmsky_ground physics).
gmsky.Include("chunks")
-- Step S1 (one collision system): physics ground experiment (gmsky_test_physground).
gmsky.Include("physground")
if GetPlayer() and T then T.home = T.MeasureRoom(GetPlayer():GetPos() + Vector(0, 0, 40)) end
local anchored = 0 -- the origin epoch Garry's Mod is anchored to
local inputOn = false -- input comes from Skyrim's window right now
local menuCursor = false -- a menu cursor is up (or Q held): the mouse never turns the view

-- Jump log (after test 22: a few single-frame jumps of 100-260 units, cause not known). A tick
-- where the player moved more than 40 units further than its speed explains is logged with what
-- was going on. Positions are relative to the anchor, so a re-centre is not a jump.
local jump = { logged = 0, total = 0 }
local function CheckJump(ply, pos, vel, tick)
	local ax, ay, az = gmsky.Anchor()
	if not ax then jump.last = nil return end
	local rx, ry, rz = pos.x - ax, pos.y - ay, pos.z - az
	local l = jump.last
	if l and tick == l.tick + 1 then
		local dt = engine.TickInterval()
		local mx, my, mz = rx - l.x, ry - l.y, rz - l.z
		local ex, ey, ez = (l.vx + vel.x) / 2 * dt, (l.vy + vel.y) / 2 * dt, (l.vz + vel.z) / 2 * dt
		local off = math.sqrt((mx - ex) ^ 2 + (my - ey) ^ 2 + (mz - ez) ^ 2)
		if off > 40 then
			jump.total = jump.total + 1
			if jump.logged < 5 then
				jump.logged = jump.logged + 1
				local ground = ply:GetGroundEntity()
				gmsky.Log(string.format("JUMP tick %d: moved (%.1f, %.1f, %.1f), speed explains (%.1f, %.1f, %.1f), %.1f off; speed (%.0f, %.0f, %.0f) -> (%.0f, %.0f, %.0f); %s, %s, on %s, input from Skyrim %s, terrain %s",
					tick, mx, my, mz, ex, ey, ez, off, l.vx, l.vy, l.vz, vel.x, vel.y, vel.z,
					ply:GetMoveType() == MOVETYPE_NOCLIP and "noclip" or "walking", ply:IsOnGround() and "on ground" or "in air",
					IsValid(ground) and ground:GetClass() or (ground == game.GetWorld() and "the map" or "nothing"),
					l.input == inputOn and (inputOn and "on" or "off") or ("switched " .. (inputOn and "on" or "off")),
					T and string.format("holding %s, lifts this second %d, re-centres %d", tostring(T.holding), T.stats and T.stats.lifts or 0, T.recentres or 0) or "-"))
			end
		end
	end
	jump.last = { tick = tick, x = rx, y = ry, z = rz, vx = vel.x, vy = vel.y, vz = vel.z, input = inputOn }
end

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
	-- Far from the middle of the map: move the player and the anchor back together (step C).
	if T then T.Recentre(ply, anchored) end
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
	-- Step 3a: a menu cursor is up; Skyrim must not turn its camera ahead with the mouse.
	if menuCursor then flags = flags + 0x100 end
	-- Step D: ask Skyrim for its collision shapes (gmsky_ground shapes) or the ray grid.
	if T and T.ShapesMode() then flags = flags + 0x80 end
	-- Degrees per Skyrim mouse count, so Skyrim can draw the mouse movement still on its way here.
	local degPerCount = predict:GetBool() and 0.022 * sensitivity:GetFloat() or 0
	CheckJump(ply, pos, vel, engine.TickCount())
	-- Camera (step 4): view punch (hard landings, later recoil) goes to Skyrim's camera; the FOV
	-- is 0 until something sets it, then the player's own fov_desired is the one in use.
	local punch = ply:GetViewPunchAngles()
	local fov = ply:GetFOV()
	if not fov or fov <= 0 then fov = ply:GetInfoNum("fov_desired", 75) end
	anchored = gmsky.WritePlayerState(engine.TickCount(), pos.x, pos.y, pos.z, ang.p, ang.y, ang.r,
		ply:EyePos().z - pos.z, vel.x, vel.y, vel.z, flags, fov, degPerCount,
		engine.TickInterval() * 1000, punch.p, punch.y, punch.r) or 0
	if T then T.UpdateGround(ply, anchored, (gmsky.Status())) end
	if gmsky.actors then gmsky.actors.Update() end
	if gmsky.standins then gmsky.standins.Update() end
	if gmsky.hurt then gmsky.hurt.Update(ply) end
	if gmsky.respawn then gmsky.respawn.Update(ply, anchored) end
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

-- Phase 3 step 3a: weapon choice and Garry's Mod's menus are done on the client side (binds.lua),
-- through Garry's Mod's own key binds. Here: attack comes from the player's own command while
-- the client handles binds (a click a weapon menu took never fires), no view turning while a
-- menu cursor is up, and the physics gun gets the wheel while it holds something.
local weapons = { physgunNotches = 0, fromClient = nil }

local function NameOf(w)
	return IsValid(w) and w:GetClass() or "nothing"
end

local function AngleGap(a, b)
	return math.max(math.abs(math.NormalizeAngle(a.y - b.y)), math.abs(a.p - b.p))
end

hook.Add("StartCommand", "gmsky_input", function(ply, cmd)
	if ply ~= GetPlayer() then return end
	local active, buttons, dx, dy, dw, noclipPresses = gmsky.ReadInput()
	if not active then
		menuCursor = false
		if view then
			-- Tell the Garry's Mod window our view (it only had its own, stale one until now).
			ply:SetEyeAngles(view)
			hold = { ang = view, untilTime = CurTime() + HOLD_TIME, ticks = 0 }
			gmsky.Log(string.format("input from Skyrim: OFF, holding yaw=%.1f pitch=%.1f", view.y, view.p))
			view = nil
			inputOn = false
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
		inputOn = true
		view.r = 0
		gmsky.Log("input from Skyrim: ON")
	end
	for _ = 1, noclipPresses do ToggleNoclip(ply) end
	local fromClient, cursor = gmsky.ClientInput()
	-- Q (Sandbox's spawn menu) held: a cursor is up from the first tick, before the client sees it.
	cursor = cursor or bit.band(buttons, 0x800) ~= 0
	if cursor ~= menuCursor then
		menuCursor = cursor
		gmsky.Log(cursor and "menu cursor up: the mouse doesn't turn the view" or "menu cursor gone: the mouse turns the view again")
	end
	if fromClient ~= weapons.fromClient then
		weapons.fromClient = fromClient
		gmsky.Log(fromClient and "attack from Garry's Mod's own binds (client side handles clicks, wheel, 1-6, Q)"
			or "attack from Skyrim's buttons (client side not handling binds)")
	end
	if not cursor then
		local k = 0.022 * sensitivity:GetFloat()
		view.y = math.NormalizeAngle(view.y - dx * k)
		view.p = math.Clamp(view.p + dy * k, -89, 89)
	end

	local b = 0
	for _, m in ipairs(IN_MAP) do
		if bit.band(buttons, m[1]) ~= 0 then b = bit.bor(b, m[2]) end
	end
	local ATTACKS = bit.bor(IN_ATTACK, IN_ATTACK2)
	if fromClient then
		b = bit.bor(bit.band(b, bit.bnot(ATTACKS)), bit.band(cmd:GetButtons(), ATTACKS))
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
	-- The physics gun holding something takes the wheel (pushes / pulls the prop); the client
	-- doesn't turn the wheel into weapon binds then.
	local wep = ply:GetActiveWeapon()
	if NameOf(wep) ~= weapons.held then
		if weapons.held then gmsky.Log(string.format("weapon now %s (was %s)", NameOf(wep), weapons.held)) end
		weapons.held = NameOf(wep)
	end
	if dw ~= 0 and not cursor and IsValid(wep) and wep:GetClass() == "weapon_physgun" and bit.band(b, IN_ATTACK) ~= 0 then
		cmd:SetMouseWheel(dw)
		weapons.physgunNotches = weapons.physgunNotches + math.abs(dw)
	end
	-- Keep the Garry's Mod window's own view in step with ours, so letting go never snaps.
	ply:SetEyeAngles(view)

	inputStats.cmds = inputStats.cmds + 1
	inputStats.dx = inputStats.dx + dx
	inputStats.dy = inputStats.dy + dy
	inputStats.buttons = buttons
end)

-- After test 126 (the user): when the Skyrim we were linked to closes, Garry's Mod closes too
-- (its window gets the same "close" as its close button). The module watches Skyrim's process.
-- Test 127: Garry's Mod ignored the close message (and Lua may not run `exit`: Garry's Mod blocks
-- it), so if it is still running CLOSE_WAIT seconds later, the module ends its process.
local CLOSE_WAIT = 5
local closeWithSkyrim = CreateConVar("gmsky_close_with_skyrim", "1", FCVAR_ARCHIVE,
	"1 = Garry's Mod closes when the Skyrim it was linked to closes")
timer.Create("gmsky_close_with_skyrim", 1, 0, function()
	local gone = gmsky.SkyrimExited()
	if not gone then return end
	if not closeWithSkyrim:GetBool() then
		gmsky.Log(string.format("Skyrim (process %d) has closed; not closing Garry's Mod (gmsky_close_with_skyrim 0)", gone))
		return
	end
	gmsky.Log(string.format("Skyrim (process %d) has closed: closing Garry's Mod: %s", gone, gmsky.CloseGame()))
	timer.Simple(CLOSE_WAIT, function()
		gmsky.Log(string.format("Garry's Mod is still running %d s after the close message: ending its process now (Garry's Mod's own shutdown is skipped)", CLOSE_WAIT))
		gmsky.EndProcess()
	end)
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
	local input = view and string.format(" input=ON cmds/s=%d buttons=0x%x mouse=(%d, %d) weapon=%s",
		inputStats.cmds, inputStats.buttons, inputStats.dx, inputStats.dy, ply and NameOf(ply:GetActiveWeapon()) or "-") or " input=off"
	local terrain = T and (T.Status(ply) .. (ply and T.GroundStatus(ply) or "")) or ""
	if gmsky.field then terrain = terrain .. gmsky.field.Status(ply) end
	if gmsky.actors then terrain = terrain .. gmsky.actors.Status(ply) end
	if gmsky.standins then terrain = terrain .. gmsky.standins.Status(ply) end
	if gmsky.hurt then terrain = terrain .. gmsky.hurt.Status() end
	if gmsky.respawn then terrain = terrain .. gmsky.respawn.Status() end
	if gmsky.water then terrain = terrain .. gmsky.water.Status() end
	gmsky.Log(string.format("tick=%d %s skyrim=%s epoch=%d skyrim_frame=%d writes/s=%d%s%s%s",
		engine.TickCount(), where, connected and "CONNECTED" or "not connected", epoch, skyrimFrame,
		writes - lastWrites, input, err and (" error=" .. err) or "", terrain) .. (jump.total > 0 and string.format(" | jumps %d", jump.total) or ""))
	lastWrites = writes
	jump.logged, jump.total = 0, 0
	inputStats.cmds, inputStats.dx, inputStats.dy = 0, 0, 0
	if weapons.physgunNotches > 0 then
		gmsky.Log(string.format("physics gun: %d wheel notches this second", weapons.physgunNotches))
		weapons.physgunNotches = 0
	end
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
