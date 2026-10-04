-- gmsky client side. Loaded when the server sends `require("gmsky")` to the client.
-- Phase 0 only proves the client side loads and runs; later phases use it for camera, HUD and drawing.
-- Log: logs/gmod_client.log

local gmsky = gmsky

local frames = 0
hook.Add("Think", "gmsky_count_frames", function()
	frames = frames + 1
end)

timer.Create("gmsky_client_status", 5, 0, function()
	gmsky.Log(string.format("client frames in last 5 s: %d (%.0f fps)", frames, frames / 5))
	frames = 0
end)

gmsky.Log("client.lua loaded")
print("[gmsky] client side loaded")
