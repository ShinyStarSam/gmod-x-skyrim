-- gmsky 2D alpha test, Phase 2a after test 31 (DESIGN.md "2a after test 31"). Client side only
-- (client.lua includes it).
--
-- Tests 30-31: in Garry's Mod's own frame the 3D view model writes a right alpha (transparency),
-- the 2D parts (HUD, crosshair, panels) write none, and forcing it from Lua (alpha writes +
-- "over" blending of the alpha channel) didn't help. This finds out why, with shapes whose right
-- alpha we know in advance (no black/white reference needed):
--   * three consecutive frames (variants): "plain" (nothing forced), "write" (alpha writes forced,
--     VRMod's way, no blending override), "over" (test 31's: alpha writes + "over" blending);
--     each cleared to transparent black at PreDrawHUD (depth-in-alpha off), so only 2D drawing
--     remains, and captured in DrawOverlay;
--   * patches 40 x 40, white, alpha 255 / 128 / 46 (46 ~ the health panel's 0.18), drawn in
--     HUDPaint with surface.DrawRect, surface.DrawTexturedRect, draw.RoundedBox (VGUI's surface)
--     and render.DrawQuad (not VGUI's surface); and the same three in a Derma panel (Lua UI),
--     painted by the engine's VGUI pass;
--   * once, VRMod's way (see DESIGN.md): a second Derma panel painted
--     manually into our own render target with alpha writes forced, <name>_rt.png;
--   * with mat_queue_mode 0 and the module's gmsky.D3DState (idea 1), Direct3D 9's blending
--     settings read at several points of each frame (read only).
--
-- Test 33 (HOLOHUD 2, DESIGN.md "HUD decision after test 32"): a fourth frame, "grey": like "write"
-- but cleared to mid-grey. Over grey, Garry's Mod's real result is known; the "write" frame
-- (colour + alpha over black) must predict it as colour + (1 - alpha) x grey (in linear light).
-- That tells whether the HUD's additive glow writes alpha it shouldn't.
--
-- Console: gmsky_test_alpha2d [name] [nopatches]; then close the console. "nopatches" leaves out
-- the test patches and panels (so they don't cover HUD elements). Saves <name>_plain.png,
-- <name>_write.png, <name>_over.png, <name>_grey.png, <name>_rt.png and <name>_patches.txt (the
-- layout) into logs/overlay/. Report: python tools\alpha2d_report.py
-- Log: logs/gmod_client.log

local gmsky = gmsky
local A = gmsky.alpha2d or {}
gmsky.alpha2d = A

local VARIANTS = { "plain", "write", "over", "grey" }
-- The "grey" frame's clear value (sRGB 124 = about 0.2 in linear light; the report measures it).
local GREY = 124
local ALPHAS = { 255, 128, 46 }
local SIZE, GAP = 40, 10
local ROW_HUD, ROW_DERMA, X0 = 60, 120, 40
local RT_W, RT_H = 256, 64

-- The patch layout (screen pixels; the rt patches in the render target's pixels).
local function Layout()
	local list, x = {}, X0
	for _, kind in ipairs({ "rect", "tex", "rbox", "quad" }) do
		for _, a in ipairs(ALPHAS) do
			list[#list + 1] = { name = kind .. a, kind = kind, a = a, x = x, y = ROW_HUD, where = "screen" }
			x = x + SIZE + GAP
		end
	end
	for i, a in ipairs(ALPHAS) do
		list[#list + 1] = { name = "derma" .. a, kind = "derma", a = a, x = X0 + (i - 1) * (SIZE + GAP), y = ROW_DERMA, where = "screen" }
	end
	for i, a in ipairs(ALPHAS) do
		list[#list + 1] = { name = "rt" .. a, kind = "rt", a = a, x = 10 + (i - 1) * (SIZE + GAP), y = 10, where = "rt" }
	end
	return list
end

-- Direct3D 9's settings now (idea 1), only when Source draws on this thread (mat_queue_mode 0).
local function D3D(t, label)
	if not t.d3d then return end
	t.results[#t.results + 1] = VARIANTS[t.step] .. " " .. label .. ": " .. gmsky.D3DState(label)
end

local function QueueMode()
	local cv = GetConVar("mat_queue_mode")
	return cv and cv:GetInt() or "?"
end

-- Three rects in a panel's own coordinates (the Derma patches).
local function PaintRects(t)
	for i, a in ipairs(ALPHAS) do
		surface.SetDrawColor(255, 255, 255, a)
		surface.DrawRect((i - 1) * (SIZE + GAP), 0, SIZE, SIZE)
	end
end

local function MakePanels(t)
	local p = vgui.Create("DPanel")
	p:SetPos(X0, ROW_DERMA)
	p:SetSize(3 * SIZE + 2 * GAP, SIZE)
	p:SetMouseInputEnabled(false)
	p:SetKeyboardInputEnabled(false)
	p.Paint = function()
		PaintRects(t)
		if t.inFrame and not t.dermaRead then
			t.dermaRead = true
			t.calls.DermaPaint = true
			D3D(t, "Derma Paint")
		end
	end
	t.derma = p
	-- VRMod's way: painted only by us, into our own render target.
	local m = vgui.Create("DPanel")
	m:SetPos(0, 0)
	m:SetSize(RT_W, RT_H)
	m:SetPaintedManually(true)
	m.Paint = function() PaintRects(t) end
	t.manual = m
	t.rt = GetRenderTargetEx("gmsky_alpha2d_rt", RT_W, RT_H, RT_SIZE_LITERAL, MATERIAL_RT_DEPTH_SEPARATE, 0, 0, IMAGE_FORMAT_RGBA8888)
end

local function Save(name, png)
	if not png or #png == 0 then return name .. ": render.Capture gave nothing" end
	local ok, err = gmsky.WriteLogFile(name, png)
	return name .. ": " .. (ok and string.format("saved (%d KB)", math.floor(#png / 1024)) or ("could not save: " .. tostring(err)))
end

local function Capture(name, w, h)
	local ok, png = pcall(render.Capture, { format = "png", x = 0, y = 0, w = w, h = h, alpha = true })
	return ok and Save(name, png) or (name .. ": ERROR " .. tostring(png))
end

-- VRMod's settings (lua/vrmod/ui/cl_ui.lua): own render target, alpha writes forced, cleared to
-- transparent, depth-in-alpha off, panel painted manually, everything set back.
local function CaptureRT(t)
	render.PushRenderTarget(t.rt)
	local ok, res = pcall(function()
		cam.Start2D()
		render.OverrideAlphaWriteEnable(true, true)
		render.Clear(0, 0, 0, 0, true, true)
		render.SetWriteDepthToDestAlpha(false)
		t.manual:PaintManual()
		render.SetWriteDepthToDestAlpha(true)
		render.OverrideAlphaWriteEnable(false)
		local r = Capture(t.tag .. "_rt.png", RT_W, RT_H)
		cam.End2D()
		return r
	end)
	render.PopRenderTarget()
	return ok and res or ("rt: ERROR " .. tostring(res))
end

-- Not before the console / menu is closed and half a second has passed (no HUD while open).
local function Ready(t)
	if gui.IsGameUIVisible() or gui.IsConsoleVisible() then
		t.closedAt = nil
		return false
	end
	t.closedAt = t.closedAt or SysTime()
	return SysTime() - t.closedAt > 0.5
end

local function Finish(t)
	if IsValid(t.derma) then t.derma:Remove() end
	if IsValid(t.manual) then t.manual:Remove() end
	local lines = {}
	for _, p in ipairs(t.nopatches and {} or Layout()) do
		lines[#lines + 1] = string.format("%s %s %d %d %d %d %d", p.name, p.where, p.x, p.y, SIZE, SIZE, p.a)
	end
	gmsky.WriteLogFile(t.tag .. "_patches.txt", table.concat(lines, "\n") .. "\n")
	local wep = LocalPlayer():GetActiveWeapon()
	gmsky.Log(string.format("alpha2d test %q: %dx%d, weapon %s, mat_queue_mode %s | %s", t.tag, ScrW(), ScrH(),
		IsValid(wep) and wep:GetClass() or "none", tostring(QueueMode()), table.concat(t.results, " | ")))
	A.test = nil
end

hook.Add("PreRender", "gmsky_alpha2d", function()
	local t = A.test
	if t and not t.ready and Ready(t) then
		t.ready = true
		gmsky.Log("alpha2d test " .. t.tag .. ": console closed, capturing")
	end
end)

-- First 2D hook of the frame: wipe everything drawn so far, then this frame's variant.
hook.Add("PreDrawHUD", "gmsky_alpha2d", function()
	local t = A.test
	if not t or not t.ready or t.inFrame then return end
	t.inFrame, t.frame, t.calls, t.dermaRead = true, FrameNumber(), { PreDrawHUD = true }, false
	render.SetWriteDepthToDestAlpha(false)
	t.depthAlphaOff = true
	local v = VARIANTS[t.step]
	-- Test 36: the patched HOLOHUD saves its layers and the screen after each of its steps.
	gmsky.holohudProbe = (v == "plain" or v == "write") and (t.tag .. "_" .. v) or nil
	local bg = v == "grey" and GREY or 0
	render.Clear(bg, bg, bg, 0, true, true)
	if v ~= "plain" then
		render.OverrideAlphaWriteEnable(true, true)
		t.alphaForced = true
		gmsky.alphaForced = true -- read by the HOLOHUD patch (gmod/holohud2_patch)
	end
	if v == "over" then
		render.OverrideBlend(true, BLEND_SRC_ALPHA, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD,
			BLEND_ONE, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD)
		t.blendForced = true
	end
	D3D(t, "PreDrawHUD")
end)

local white

hook.Add("HUDPaint", "gmsky_alpha2d", function()
	local t = A.test
	if not t or not t.inFrame then return end
	t.calls.HUDPaint = true
	if t.nopatches then return end
	for _, p in ipairs(Layout()) do
		if p.kind == "rect" then
			surface.SetDrawColor(255, 255, 255, p.a)
			surface.DrawRect(p.x, p.y, SIZE, SIZE)
			if p.a == 255 then D3D(t, "HUDPaint after rect255") end
		elseif p.kind == "tex" then
			white = white or Material("vgui/white")
			surface.SetMaterial(white)
			surface.SetDrawColor(255, 255, 255, p.a)
			surface.DrawTexturedRect(p.x, p.y, SIZE, SIZE)
		elseif p.kind == "rbox" then
			draw.RoundedBox(8, p.x, p.y, SIZE, SIZE, Color(255, 255, 255, p.a))
		elseif p.kind == "quad" then
			render.SetColorMaterialIgnoreZ()
			render.DrawQuad(Vector(p.x, p.y, 0), Vector(p.x + SIZE, p.y, 0), Vector(p.x + SIZE, p.y + SIZE, 0),
				Vector(p.x, p.y + SIZE, 0), Color(255, 255, 255, p.a))
		end
	end
	D3D(t, "HUDPaint end")
end)

-- Last 2D hook: capture this frame (and once VRMod's render target), next variant.
hook.Add("DrawOverlay", "gmsky_alpha2d", function()
	local t = A.test
	if not t or not t.inFrame then return end
	D3D(t, "DrawOverlay")
	local v = VARIANTS[t.step]
	local saved = Capture(t.tag .. "_" .. v .. ".png", ScrW(), ScrH())
	local calls = {}
	for k in SortedPairs(t.calls) do calls[#calls + 1] = k end
	t.results[#t.results + 1] = string.format("%s: frame %d, %s, hooks: %s", v, t.frame, saved, table.concat(calls, ", "))
	if v == "plain" and not t.nopatches then t.results[#t.results + 1] = CaptureRT(t) end
	t.inFrame = false
	t.step = t.step + 1
	if t.step > #VARIANTS then Finish(t) end
end)

-- After the frame: everything back to normal.
hook.Add("PostRender", "gmsky_alpha2d", function()
	local t = A.test
	if not t then return end
	gmsky.holohudProbe = nil
	if t.depthAlphaOff then render.SetWriteDepthToDestAlpha(true) t.depthAlphaOff = nil end
	if t.blendForced then render.OverrideBlend(false) t.blendForced = nil end
	if t.alphaForced then render.OverrideAlphaWriteEnable(false) t.alphaForced = nil gmsky.alphaForced = false end
end)

concommand.Add("gmsky_test_alpha2d", function(_, _, args)
	local tag = (args[1] or "a2d"):gsub("[^%w_%-]", "")
	if tag == "" then tag = "a2d" end
	if A.test then
		if IsValid(A.test.derma) then A.test.derma:Remove() end
		if IsValid(A.test.manual) then A.test.manual:Remove() end
	end
	local t = { tag = tag, step = 1, results = {}, calls = {}, nopatches = args[2] == "nopatches" }
	local qm = QueueMode()
	if not gmsky.D3DState then
		t.results[#t.results + 1] = "Direct3D reads: not in this module build"
	elseif qm ~= 0 then
		t.results[#t.results + 1] = "Direct3D reads skipped: mat_queue_mode " .. tostring(qm)
			.. " (Source draws on another thread; type mat_queue_mode 0 first)"
	else
		t.d3d = true
	end
	if not t.nopatches then MakePanels(t) end
	A.test = t
	gmsky.Log("alpha2d test " .. tag .. ": mat_queue_mode " .. tostring(qm) .. ", capturing " .. #VARIANTS
		.. " frames once the console is closed" .. (t.nopatches and " (no test patches)" or ""))
end)
