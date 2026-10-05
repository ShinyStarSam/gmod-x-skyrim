-- gmsky live overlay, Phase 2 step 2b (DESIGN.md "Step 2b in detail"). Client side only
-- (client.lua includes it).
--
-- Step 2b-1: the module finds Garry's Mod's Direct3D 9 device and hooks its Present (the call that
-- hands each finished frame to Windows; module file present.rs). For now the hook only counts:
-- frames, which thread draws, and the back buffer's format, size and multisampling. Later steps
-- copy the picture there into shared memory for Skyrim.
--
-- Console (client): gmsky_overlay_probe   installs the hook (once; harmless to repeat).
-- Log (every 5 s once installed): logs/gmod_client.log, lines starting "present hook:".
--
-- Step 2b-3: gmsky_overlay 1 (client console variable, off at every start) sends every frame to
-- the overlay picture block for Skyrim: each frame is prepared as tests 30-39 did for one frame
-- (cleared to transparent where the view model starts, or at PreDrawHUD if there is no view
-- model, Source's depth-in-alpha off, alpha writes forced for the HUD; gmsky.alphaForced for the
-- HOLOHUD patch), everything set back in PostRender, and the frame marked prepared; the module's
-- Present hook copies marked frames (capture.rs). The Garry's Mod window then shows only the hands
-- and HUD on black: that is expected (it goes out of sight in step 2d).

local gmsky = gmsky
local L = gmsky.liveoverlay or {}
gmsky.liveoverlay = L

L.cvar = L.cvar or CreateConVar("gmsky_overlay", "0", 0,
	"1 = send Garry's Mod's hands, weapon and HUD to Skyrim every frame (step 2b-3)")
-- This frame: cleared yet, alpha forced, which hooks ran (for the one-time order log).
L.frame = L.frame or {}

local function QueueMode()
	local cv = GetConVar("mat_queue_mode")
	return cv and cv:GetString() or "?"
end

concommand.Add("gmsky_overlay_probe", function()
	if not gmsky.OverlayHook then
		gmsky.Log("overlay probe: this module build has no OverlayHook")
		return
	end
	local t0 = SysTime()
	local line = gmsky.OverlayHook()
	gmsky.Log(string.format("overlay probe (%.1f ms, mat_queue_mode %s, screen %dx%d): %s",
		(SysTime() - t0) * 1000, QueueMode(), ScrW(), ScrH(), line))
	print("[gmsky] " .. line)
end)

-- On / off, checked once per frame before drawing.
local function Apply(on)
	if on == L.on then return end
	local was = L.on
	L.on = on
	if on then
		local line = gmsky.OverlayHook and gmsky.OverlayHook() or "this module build has no OverlayHook"
		gmsky.Log(string.format("live overlay ON (mat_queue_mode %s, screen %dx%d): %s", QueueMode(), ScrW(), ScrH(), line))
		if gmsky.OverlayEnable then gmsky.OverlayEnable(true) end
		L.orderLogged = false
	else
		if gmsky.OverlayEnable then gmsky.OverlayEnable(false) end
		-- (Not logged at start-up, when it was never on.)
		if was ~= nil then gmsky.Log("live overlay OFF") end
	end
end

local function Note(name)
	local f = L.frame
	f.order = f.order or {}
	f.order[#f.order + 1] = name
end

-- Wipes everything drawn so far this frame (the world) to transparent, once per frame.
local function ClearOnce(where)
	local f = L.frame
	if not f.active or f.cleared then return end
	f.cleared = where
	render.SetWriteDepthToDestAlpha(false)
	render.Clear(0, 0, 0, 0, true, true)
end

hook.Add("PreRender", "gmsky_liveoverlay", function()
	-- Once per switch-on: the order of the last frame's final hooks, and where it was cleared.
	local prev = L.frame
	if L.on and not L.orderLogged and prev.cleared then
		L.orderLogged = true
		gmsky.Log("live overlay first frame: cleared at " .. tostring(prev.cleared) .. ", hook order "
			.. table.concat(prev.order or {}, ", "))
	end
	Apply(L.cvar:GetBool())
	L.frame = { active = L.on }
end)

hook.Add("PreDrawViewModel", "gmsky_liveoverlay", function() ClearOnce("view model") end)
hook.Add("PreDrawPlayerHands", "gmsky_liveoverlay", function() ClearOnce("hands") end)

hook.Add("PreDrawHUD", "gmsky_liveoverlay", function()
	local f = L.frame
	if not f.active then return end
	ClearOnce("HUD (no view model this frame)")
	if not f.alphaForced then
		f.alphaForced = true
		render.OverrideAlphaWriteEnable(true, true)
		gmsky.alphaForced = true -- read by the HOLOHUD patch (gmod/holohud2_patch)
	end
end)

hook.Add("DrawOverlay", "gmsky_liveoverlay", function()
	if L.on then Note("DrawOverlay") end
end)

-- After the frame: everything back to normal, and the frame marked for the Present hook.
hook.Add("PostRender", "gmsky_liveoverlay", function()
	local f = L.frame
	if not f.active then return end
	Note("PostRender")
	if f.cleared then
		render.SetWriteDepthToDestAlpha(true)
		if gmsky.OverlayPrepared then gmsky.OverlayPrepared() end
	end
	if f.alphaForced then
		render.OverrideAlphaWriteEnable(false)
		gmsky.alphaForced = false
	end
	f.active = false
end)

-- A Lua error inside a timer makes Garry's Mod delete the timer, so errors are caught and logged.
timer.Create("gmsky_liveoverlay_status", 5, 0, function()
	local ok, err = pcall(function()
		local s = gmsky.OverlayStatus and gmsky.OverlayStatus()
		if s then gmsky.Log(s .. "; mat_queue_mode " .. QueueMode()) end
	end)
	if not ok then gmsky.Log("ERROR in live overlay status: " .. tostring(err)) end
end)
