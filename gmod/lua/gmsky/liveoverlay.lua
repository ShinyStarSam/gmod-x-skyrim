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

local gmsky = gmsky
local L = gmsky.liveoverlay or {}
gmsky.liveoverlay = L

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

-- A Lua error inside a timer makes Garry's Mod delete the timer, so errors are caught and logged.
timer.Create("gmsky_liveoverlay_status", 5, 0, function()
	local ok, err = pcall(function()
		local s = gmsky.OverlayStatus and gmsky.OverlayStatus()
		if s then gmsky.Log(s .. "; mat_queue_mode " .. QueueMode()) end
	end)
	if not ok then gmsky.Log("ERROR in live overlay status: " .. tostring(err)) end
end)
