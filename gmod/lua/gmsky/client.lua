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

timer.Create("gmsky_client_status", 5, 0, function()
	gmsky.Log(string.format("client frames in last 5 s: %d (%.0f fps)", frames, frames / 5))
	frames = 0
end)

-- Garry's Mod sleeps between frames while its window isn't in front, which makes its ticks
-- arrive in bursts (Phase 0 finding). Skyrim's window is in front now, so turn that off.
RunConsoleCommand("engine_no_focus_sleep", "0")
timer.Simple(1, function()
	local cv = GetConVar("engine_no_focus_sleep")
	gmsky.Log("engine_no_focus_sleep is now " .. (cv and cv:GetString() or "(no such setting)"))
end)

gmsky.Log("client.lua loaded")
print("[gmsky] client side loaded")
