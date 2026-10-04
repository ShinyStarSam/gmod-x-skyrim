-- gmsky client side. Loaded when the server sends `require("gmsky")` to the client.
-- Phase 0 only proves the client side loads and runs; later phases use it for camera, HUD and drawing.
-- Log: logs/gmod_client.log

local gmsky = gmsky

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

timer.Create("gmsky_client_status", 5, 0, function()
	gmsky.Log(string.format("client frames in last 5 s: %d (%.0f fps) fps_max_nofocus=%s",
		frames, frames / 5, NoFocusCap()))
	frames = 0
end)

gmsky.Log("client.lua loaded")
print("[gmsky] client side loaded")
