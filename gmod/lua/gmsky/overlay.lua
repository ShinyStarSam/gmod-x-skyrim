-- gmsky overlay, Phase 2a experiment (DESIGN.md "Phase 2 steps"). Client side only (client.lua
-- includes it).
--
-- Phase 2 draws Garry's Mod's hands, weapon and HUD inside Skyrim's window, as SkyCraft (MIT) does
-- with Minecraft's: only those, on a transparent background, at Skyrim's resolution. Before
-- building that, this measures what Garry's Mod gives us:
--   * which Direct3D 9 it draws with (gmsky.GraphicsInfo, logged when this file loads);
--   * whether the view model (hands + weapon) and the HUD can be had alone;
--   * how to get their transparency: two-background matting (the overlay over black and over
--     white; where they differ, the pixel is see-through).
--
-- Tries so far (MODLOG tests 25, 26):
--   1. vm:DrawModel() / hands:DrawModel() / render.RenderHUD into a render target from PostRender:
--      nothing drawn; render.Capture of the screen there: nothing.
--   2. render.RenderView into a render target from PostRender: only the sky was drawn (0.5 ms,
--      identical pictures for every weapon), although the view-model hooks ran.
-- Now (try 3): Garry's Mod's own, normal frame. Over three consecutive frames: (1) the frame as it
-- is, (2) the screen cleared to black at the first PreDrawViewModel / PreDrawPlayerHands of the
-- frame, so only what is drawn after the world remains, (3) the same with white. Each is captured
-- in PostDrawHUD (after all of the HUD, still inside the frame). The Garry's Mod window flashes for
-- two frames. If it works, step 2b does the same every frame (no second view, no extra cost).
--
-- Test 27: this works for the view model (hands + weapon): solid wherever black and white agree,
-- Source's own alpha there is 13-90 (unusable), the matte is exact. The HUD was missing, also in the
-- plain frame: HUDPaint didn't run, most likely because the console (where the command was typed)
-- was still open. So the capture now waits until Garry's Mod's console / menu is closed, + 0.5 s.
--
-- Test 28: the HUD is there (health panel 18% opaque, its digits 64-86%), but the PNGs' own alpha
-- (Source's) hides it in an image viewer; previews are made from the matte instead. No crosshair in
-- the overlay nor at the screen centre: each frame is now also captured in DrawOverlay (*_late.png)
-- to see whether the crosshair (and chat, menus) are drawn after PostDrawHUD.
--
-- Test 29 (crosshair on): the crosshair is captured; panels drawn after PostDrawHUD (notifications,
-- the tool gun's info: dark see-through boxes on the right) appear only in DrawOverlay, so the real
-- capture point is DrawOverlay. The black/white pair comes from two frames, which won't line up when
-- the hands move; so a fourth frame tests one-frame alpha: cleared to transparent black with
-- render.SetWriteDepthToDestAlpha(false) (Source stores depth in alpha by default), <name>_alpha.png.
--
-- Console: gmsky_test_overlay [name] saves into logs/overlay/ (through the module, never into the
-- game folder): <name>_scene.png, <name>_black.png, <name>_white.png, <name>_alpha.png (and *_late.png), and logs the timings and
-- which hooks ran in each frame.
-- Log: logs/gmod_client.log

local gmsky = gmsky
local O = gmsky.overlay or {}
gmsky.overlay = O

gmsky.Log(gmsky.GraphicsInfo())

-- name, clear colour at the view model (nil = no clear), true = also stop Source writing depth into
-- the alpha channel for the rest of that frame, and where to force alpha writes with "over"
-- blending of the alpha channel (nil = nowhere, "2d" = from PreDrawHUD on, "all" = from the view
-- model on). Test 30: with depth-in-alpha off the 3D view model's alpha is right, the 2D parts
-- write none. Test 31: does forcing it fix the 2D parts (SkyCraft's model: one frame, the game's
-- own alpha)? Black / white stay as the reference to measure against.
local STEPS = {
	{ "scene", nil }, { "black", 0 }, { "white", 255 },
	{ "alpha", 0, true }, { "alpha2d", 0, true, "2d" }, { "alphaall", 0, true, "all" },
}

-- Alpha writes on, and the alpha channel blended as "over" (dest = src + dest * (1 - src alpha)),
-- the colour as usual (src * src alpha + dest * (1 - src alpha)).
local function ForceAlpha(t)
	if t.alphaForced then return end
	t.alphaForced = true
	render.OverrideAlphaWriteEnable(true, true)
	render.OverrideBlend(true, BLEND_SRC_ALPHA, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD,
		BLEND_ONE, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD)
end

-- The running test: tag, step (1..3), this frame's hook calls and whether it cleared, results.
O.test = nil

-- Not before the console / menu is closed and half a second has passed (the HUD isn't drawn
-- while they are open).
local function Ready(t)
	if gui.IsGameUIVisible() or gui.IsConsoleVisible() then
		t.closedAt = nil
		return false
	end
	t.closedAt = t.closedAt or SysTime()
	return SysTime() - t.closedAt > 0.5
end

local function Note(name)
	local t = O.test
	if not t or not t.ready then return end
	local frame = FrameNumber()
	if t.frame ~= frame then
		t.frame, t.calls, t.cleared, t.t0 = frame, {}, false, SysTime()
	end
	t.calls[name] = (t.calls[name] or 0) + 1
	local clear = STEPS[t.step][2]
	if clear and not t.cleared then
		t.cleared = true
		if STEPS[t.step][3] then
			render.SetWriteDepthToDestAlpha(false)
			t.depthAlphaOff = true
		end
		render.Clear(clear, clear, clear, 0, true, true)
		if STEPS[t.step][4] == "all" then ForceAlpha(t) end
	end
end

hook.Add("PreDrawHUD", "gmsky_overlay_test", function()
	local t = O.test
	if t and t.ready and t.cleared and STEPS[t.step] and STEPS[t.step][4] == "2d" then ForceAlpha(t) end
end)

-- After the frame: Source writes depth into alpha again, blending back to normal.
hook.Add("PostRender", "gmsky_overlay_test_alpha", function()
	local t = O.test
	if not t then return end
	if t.depthAlphaOff then
		render.SetWriteDepthToDestAlpha(true)
		t.depthAlphaOff = nil
	end
	if t.alphaForced then
		render.OverrideBlend(false)
		render.OverrideAlphaWriteEnable(false)
		t.alphaForced = nil
	end
end)
hook.Add("PreDrawViewModel", "gmsky_overlay_test", function() Note("PreDrawViewModel") end)
hook.Add("PreDrawPlayerHands", "gmsky_overlay_test", function() Note("PreDrawPlayerHands") end)
hook.Add("HUDPaint", "gmsky_overlay_test", function() Note("HUDPaint") end)

local function Save(name, png)
	if not png or #png == 0 then return "render.Capture gave nothing" end
	local ok, err = gmsky.WriteLogFile(name, png)
	return ok and string.format("saved (%d KB)", math.floor(#png / 1024)) or ("could not save: " .. tostring(err))
end

-- After all of the HUD, still inside the frame: capture this step, go to the next.
-- Arms the test once the console is closed (checked every frame, before the frame is drawn).
hook.Add("PreRender", "gmsky_overlay_test", function()
	local t = O.test
	if t and not t.ready and Ready(t) then
		t.ready = true
		gmsky.Log("overlay test " .. t.tag .. ": console closed, capturing")
	end
end)

local function Capture(file)
	local w, h = ScrW(), ScrH()
	local c0 = SysTime()
	local ok, png = pcall(render.Capture, { format = "png", x = 0, y = 0, w = w, h = h, alpha = true })
	local ms = (SysTime() - c0) * 1000
	return string.format("%s, capture+PNG %.1f ms", ok and Save(file, png) or ("ERROR " .. tostring(png)), ms)
end

-- Next step; logs the whole test after the last one.
local function Advance(t)
	t.step = t.step + 1
	t.frame, t.late = nil, nil
	if t.step > #STEPS then
		local wep = LocalPlayer():GetActiveWeapon()
		gmsky.Log(string.format("overlay test %q: %dx%d, weapon %s | %s", t.tag, ScrW(), ScrH(),
			IsValid(wep) and wep:GetClass() or "none", table.concat(t.results, " | ")))
		O.test = nil
	end
end

-- After all of the HUD, still inside the frame: capture this step. The same frame is captured
-- again in DrawOverlay (the last 2D hook: whatever Garry's Mod draws after the HUD, e.g. the
-- crosshair, chat and menus, if they come later), then the test moves to the next step.
hook.Add("PostDrawHUD", "gmsky_overlay_test", function()
	local t = O.test
	if not t or not t.ready then return end
	if t.late then
		t.results[#t.results + 1] = "late: DrawOverlay didn't run in that frame"
		Advance(t)
		t = O.test
		if not t then return end
	end
	Note("PostDrawHUD")
	local name, clear = STEPS[t.step][1], STEPS[t.step][2]
	local saved = Capture(t.tag .. "_" .. name .. ".png")
	local calls = {}
	for k, v in SortedPairs(t.calls) do calls[#calls + 1] = k .. " x" .. v end
	t.results[#t.results + 1] = string.format("%s: frame %d, %s, cleared at the view model: %s, hooks: %s",
		name, t.frame, saved, clear and tostring(t.cleared) or "-", table.concat(calls, ", "))
	t.late = name
end)

hook.Add("DrawOverlay", "gmsky_overlay_test", function()
	local t = O.test
	if not t or not t.late then return end
	t.results[#t.results + 1] = string.format("%s late (DrawOverlay, frame %d): %s", t.late, FrameNumber(),
		Capture(t.tag .. "_" .. t.late .. "_late.png"))
	Advance(t)
end)

concommand.Add("gmsky_test_overlay", function(_, _, args)
	local tag = (args[1] or "test"):gsub("[^%w_%-]", "")
	if tag == "" then tag = "test" end
	O.test = { tag = tag, step = 1, results = {} }
	gmsky.Log("overlay test " .. tag .. ": capturing three frames once the console is closed")
end)
