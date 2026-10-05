-- gmsky client side. Loaded when the server sends `require("gmsky")` to the client.
-- Phase 0 only proves the client side loads and runs; later phases use it for camera, HUD and drawing.
-- Log: logs/gmod_client.log

local gmsky = gmsky

-- Collision (Phase 1 step 3): the client builds the same collision, for movement prediction.
gmsky.Include("terrain")
gmsky.Include("field")
-- Phase 3 step 3c-2: the NPC stand-ins (their traces, for movement prediction and bullet effects).
gmsky.Include("standins")
-- Phase 2a: the overlay experiment (gmsky_test_overlay).
gmsky.Include("overlay")
-- Phase 2a after test 31: why 2D drawing writes no alpha (gmsky_test_alpha2d).
gmsky.Include("overlay2d")
-- Phase 2 step 2b: the live overlay (gmsky_overlay_probe).
gmsky.Include("liveoverlay")
-- Phase 3 step 3a: Skyrim's clicks, wheel, 1-6 and Q as Garry's Mod's own binds; menu cursor.
gmsky.Include("binds")

local frames = 0
hook.Add("Think", "gmsky_count_frames", function()
	frames = frames + 1
	-- Keeps the link alive while single-player is paused (the server's Tick stops then).
	gmsky.Heartbeat()
end)

-- Garry's Mod caps its frame rate while its window isn't in front (exactly 20 fps in test 4),
-- which makes its ticks arrive in bursts and the view choppy. This 64-bit build has no
-- `engine_no_focus_sleep`; the cap is `fps_max_nofocus`, set by tools/launch_gmod.ps1
-- (`+fps_max_nofocus 300`) or by typing it in the console. Logged here so the numbers show it.
local function NoFocusCap()
	local cv = GetConVar("fps_max_nofocus")
	return cv and cv:GetString() or "(no such setting)"
end

-- A Lua error inside a timer makes Garry's Mod delete the timer (this one stopped mid-session in
-- the first test with piece reuse on), so errors are caught and logged here instead.
timer.Create("gmsky_client_status", 5, 0, function()
	local ok, err = pcall(function()
		gmsky.Log(string.format("client frames in last 5 s: %d (%.0f fps) fps_max_nofocus=%s%s%s",
			frames, frames / 5, NoFocusCap(), gmsky.terrain and gmsky.terrain.Status(LocalPlayer()) or "",
			gmsky.field and gmsky.field.Status(LocalPlayer()) or ""))
	end)
	if not ok then gmsky.Log("ERROR in client status: " .. tostring(err)) end
	frames = 0
end)

gmsky.Log("client.lua loaded")
print("[gmsky] client side loaded")
