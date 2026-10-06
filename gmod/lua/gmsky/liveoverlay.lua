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
-- Step 2b-3: gmsky_overlay 1 (client console variable; on at every start since test 67, the
-- user's request; `gmsky_overlay 0` turns it off for the session) sends every frame to
-- the overlay picture block for Skyrim: each frame is prepared as tests 30-39 did for one frame
-- (cleared to transparent where the view model starts, or at PreDrawHUD if there is no view
-- model, Source's depth-in-alpha off, alpha writes forced for the HUD; gmsky.alphaForced for the
-- HOLOHUD patch), everything set back in PostRender, and the frame marked prepared; the module's
-- Present hook copies marked frames (capture.rs). The Garry's Mod window then shows only the hands
-- and HUD on black: that is expected (it goes out of sight in step 2d).
--
-- Steps W1-W4 (the world layer: Garry's Mod's props and effects as a second picture, with a depth
-- picture and its camera) were removed in G8 (DESIGN.md): Skyrim draws Garry's Mod's objects itself
-- since G3b (render.lua). What stayed from them: after test 72, the view model and hands get their
-- pixels stamped opaque (alpha 1, colour untouched) after they are drawn: see-through parts of
-- solid models (the RPG, the tool gun's screen) showed Skyrim through them. After test 73 the stamp
-- no longer calls itself (redrawing the view model ran its hook again: a stack overflow) and leaves
-- see-through and glowing sub-materials out (the RPG's laser glow became a black bar).

local gmsky = gmsky
local L = gmsky.liveoverlay or {}
gmsky.liveoverlay = L

L.cvar = L.cvar or CreateConVar("gmsky_overlay", "1", 0,
	"1 = send Garry's Mod's hands, weapon and HUD to Skyrim every frame (step 2b-3)")
-- This frame: cleared yet, alpha forced, which hooks ran (for the one-time order log).
L.frame = L.frame or {}

-- After test 44 (the physgun turned Skyrim's whole screen black): Garry's Mod's halo library
-- (physgun grab outline, freeze effects, Sandbox entity outlines) clears the whole screen to black
-- with alpha 255 before drawing its outline and pastes the scene back over it, so after a halo the
-- picture is opaque everywhere. While the overlay is on, each halo is drawn between a copy of the
-- frame's alpha and a restore of exactly that alpha (colours untouched): the outline's glow is
-- then added light (colour with alpha 0), like HOLOHUD's glow. gmsky_overlay_halofix 0 turns this
-- off (to compare). Counted in the 5 s status line.
L.halofix = L.halofix or CreateConVar("gmsky_overlay_halofix", "1", 0,
	"1 = keep the overlay's transparency when Garry's Mod draws a halo (physgun grab outline)")
L.halos = L.halos or { fixed = 0, unfixed = 0, ms = 0 }

-- Per 5 s status line: where frames were wiped.
L.wipes = L.wipes or {}

-- Blur behind see-through parts (HOLOHUD's plates, the console): done by Skyrim, which blurs its own
-- picture there; this only sets a flag in each picture (DESIGN.md "Blur behind the plates").
L.blur = L.blur or CreateConVar("gmsky_overlay_blur", "1", FCVAR_ARCHIVE,
	"1 = Skyrim blurs its world behind see-through parts of the overlay (HOLOHUD's plates)")

-- A screen-sized picture of our own that keeps alpha, and a material that draws it.
local function AlphaStore()
	local w, h = ScrW(), ScrH()
	if L.store and L.store.w == w and L.store.h == h then return L.store end
	local name = string.format("gmsky_alphastore_%dx%d", w, h)
	-- Point sampling, clamped (1 + 4 + 8): pixel for pixel.
	local rt = GetRenderTargetEx(name, w, h, RT_SIZE_LITERAL, MATERIAL_RT_DEPTH_NONE, 13, 0, IMAGE_FORMAT_RGBA8888)
	local mat = CreateMaterial(name .. "_mat", "UnlitGeneric", {
		["$basetexture"] = rt:GetName(), ["$translucent"] = "1", ["$ignorez"] = "1",
		["$vertexcolor"] = "0", ["$vertexalpha"] = "0",
	})
	L.store = { w = w, h = h, rt = rt, mat = mat }
	return L.store
end

local function WrapHalo()
	if not halo or not halo.Render or halo.Render == L.wrappedRender then return end
	local orig = halo.Render
	L.origRender = orig
	L.wrappedRender = function(entry)
		if not (L.frame.active and L.halofix:GetBool()) then
			if L.frame.active then L.halos.unfixed = L.halos.unfixed + 1 end
			return orig(entry)
		end
		local t0 = SysTime()
		local s = AlphaStore()
		render.CopyRenderTargetToTexture(s.rt)
		orig(entry)
		-- Alpha back from the copy: colour factors ZERO, ONE (colour stays), alpha ONE, ZERO
		-- (alpha replaced by the copy's).
		render.OverrideAlphaWriteEnable(true, true)
		render.OverrideBlend(true, BLEND_ZERO, BLEND_ONE, BLENDFUNC_ADD, BLEND_ONE, BLEND_ZERO, BLENDFUNC_ADD)
		render.SetMaterial(s.mat)
		render.DrawScreenQuad()
		render.OverrideBlend(false)
		render.OverrideAlphaWriteEnable(L.frame.alphaForced and true or false, true)
		L.halos.fixed = L.halos.fixed + 1
		L.halos.ms = L.halos.ms + (SysTime() - t0) * 1000
	end
	halo.Render = L.wrappedRender
	gmsky.Log("live overlay: halo.Render wrapped (keeps the overlay's alpha)")
end

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

-- Step 2d-2: Garry's Mod's window moved just past the screens while the overlay is on (a minimized
-- Source game stops drawing), and put back when it switches off (the user's choice).
L.hidewindow = L.hidewindow or CreateConVar("gmsky_overlay_hidewindow", "1", FCVAR_ARCHIVE,
	"1 = move Garry's Mod's window off-screen while the overlay is on (back when it switches off)")

local function Window(off)
	-- (At start-up nothing was moved: no restore, no log line.)
	if off == (L.windowOff or false) or not gmsky.OverlayWindow then return end
	L.windowOff = off
	gmsky.Log("live overlay: " .. gmsky.OverlayWindow(off))
end

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
		WrapHalo()
	else
		if gmsky.OverlayEnable then gmsky.OverlayEnable(false) end
		if L.windowOff then Window(false) end
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

local function Count(what)
	L.wipes[what] = (L.wipes[what] or 0) + 1
end

-- After test 72: a white material that writes alpha 1; drawn with colour writes off it only marks
-- a model's pixels as solid.
local function StampMaterial()
	L.stampMat = L.stampMat or CreateMaterial("gmsky_alphastamp", "UnlitGeneric", {
		["$basetexture"] = "vgui/white", ["$vertexalpha"] = "0", ["$vertexcolor"] = "0",
	})
	return L.stampMat
end

-- After test 73: a material that draws nothing (for see-through sub-materials while stamping).
local function NoDrawMaterial()
	L.noDrawMat = L.noDrawMat or CreateMaterial("gmsky_nodraw", "UnlitGeneric", { ["$basetexture"] = "vgui/white", ["$no_draw"] = "1" })
	return L.noDrawMat
end

-- MATERIAL_VAR_ADDITIVE (128) and MATERIAL_VAR_TRANSLUCENT (2097152): glows and see-through parts.
local SEE_THROUGH = 128 + 2097152
L.seeThrough = L.seeThrough or {}  -- material path -> true / false

-- The sub-material numbers (0-based) of this entity that are see-through or glowing, or nil.
local function SeeThroughParts(e)
	if not (e.GetMaterials and Material and bit) then return nil end
	local parts
	for i, path in ipairs(e:GetMaterials() or {}) do
		local sub = e.GetSubMaterial and e:GetSubMaterial(i - 1) or ""
		local name = (sub ~= "" and sub) or path
		local st = L.seeThrough[name]
		if st == nil then
			local m = Material(name)
			st = m and not m:IsError() and bit.band(m:GetInt("$flags") or 0, SEE_THROUGH) ~= 0 or false
			L.seeThrough[name] = st
		end
		if st then
			parts = parts or {}
			parts[#parts + 1] = { index = i - 1, sub = sub }
		end
	end
	return parts
end

-- Marks the pixels of these models as solid (alpha 1) without changing their colour; see-through and
-- glowing sub-materials are left out. Never runs inside itself (redrawing the view model runs the
-- view model hooks again).
local function Stamp(list)
	if #list == 0 or not render.OverrideColorWriteEnable or L.stamping then return end
	L.stamping = true
	render.OverrideColorWriteEnable(true, false)
	render.OverrideAlphaWriteEnable(true, true)
	local stamp = StampMaterial()
	for _, e in ipairs(list) do
		local parts = SeeThroughParts(e)
		if parts then
			-- Per sub-material: the solid ones stamped, the see-through ones not drawn.
			local subs = {}
			for i, m in ipairs(e:GetMaterials()) do subs[i] = e:GetSubMaterial(i - 1) or "" end
			for i = 1, #subs do e:SetSubMaterial(i - 1, "!gmsky_alphastamp") end
			for _, part in ipairs(parts) do e:SetSubMaterial(part.index, "!gmsky_nodraw") end
			NoDrawMaterial()
			e:DrawModel()
			for i, sub in ipairs(subs) do e:SetSubMaterial(i - 1, sub ~= "" and sub or nil) end
			L.stampedParts = (L.stampedParts or 0) + #parts
		else
			render.MaterialOverride(stamp)
			e:DrawModel()
			render.MaterialOverride(nil)
		end
	end
	render.OverrideColorWriteEnable(false)
	L.stamped = (L.stamped or 0) + #list
	L.stamping = false
end

hook.Add("PreRender", "gmsky_liveoverlay", function()
	-- The window follows the overlay and gmsky_overlay_hidewindow (both can change any time).
	if L.on ~= nil then Window(L.on and L.hidewindow:GetBool()) end
	local blur = L.blur:GetBool()
	if blur ~= L.blurSent and gmsky.OverlayBlur then
		gmsky.OverlayBlur(blur)
		L.blurSent = blur
		if L.on then gmsky.Log("live overlay: blur behind see-through parts " .. (blur and "ON" or "OFF")) end
	end
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
-- After test 72: the view model and hands are solid (the RPG and the tool gun's screen showed Skyrim
-- through them).
hook.Add("PostDrawViewModel", "gmsky_liveoverlay", function(vm)
	if L.frame.active and IsValid(vm) then
		Stamp({ vm })
		render.OverrideAlphaWriteEnable(false)
	end
end)
hook.Add("PostDrawPlayerHands", "gmsky_liveoverlay", function(hands)
	if L.frame.active and IsValid(hands) then
		Stamp({ hands })
		render.OverrideAlphaWriteEnable(false)
	end
end)

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
	Count("wiped at " .. tostring(f.cleared or "nothing"))
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

-- Step 2c-1: asks Skyrim to save its next drawn frame (before and after our picture, and the
-- picture) into logs/overlay/ for tools\skyrim_shot_report.py.
-- gmsky_overlay_shot [seconds]: after that many seconds (default 0), so the console can be closed
-- first (Garry's Mod's console and menus are in the picture too).
concommand.Add("gmsky_overlay_shot", function(_, _, args)
	local delay = math.max(0, tonumber(args[1] or "0") or 0)
	local function Shoot()
		local line = gmsky.OverlayShot and gmsky.OverlayShot() or "this module build has no OverlayShot"
		gmsky.Log("overlay shot: " .. line)
		print("[gmsky] " .. line)
	end
	if delay > 0 then
		print(string.format("[gmsky] snapshot in %.0f s", delay))
		timer.Simple(delay, Shoot)
	else
		Shoot()
	end
end)

-- "a 3, b 1": counts by name, most first (ties by name), at most `max` names.
local function CountList(t, max)
	local names = {}
	for k in pairs(t) do names[#names + 1] = k end
	table.sort(names, function(a, b) if t[a] ~= t[b] then return t[a] > t[b] end return a < b end)
	local out = {}
	for i = 1, math.min(#names, max) do out[i] = names[i] .. " " .. t[names[i]] end
	if #names > max then out[#out + 1] = "+" .. (#names - max) .. " more classes" end
	return #out > 0 and table.concat(out, ", ") or "none"
end

-- A Lua error inside a timer makes Garry's Mod delete the timer, so errors are caught and logged.
timer.Create("gmsky_liveoverlay_status", 5, 0, function()
	local ok, err = pcall(function()
		local s = gmsky.OverlayStatus and gmsky.OverlayStatus()
		if s then
			local h = L.halos
			gmsky.Log(string.format("%s; mat_queue_mode %s; halos: %d with alpha kept (%.2f ms each), %d without",
				s, QueueMode(), h.fixed, h.fixed > 0 and h.ms / h.fixed or 0, h.unfixed))
			L.halos = { fixed = 0, unfixed = 0, ms = 0 }
			gmsky.Log(string.format("live overlay frames: %s | %d models stamped solid (%d see-through parts left out)",
				CountList(L.wipes, 10), L.stamped or 0, L.stampedParts or 0))
			L.wipes = {}
			L.stamped, L.stampedParts = 0, 0
		end
	end)
	if not ok then gmsky.Log("ERROR in live overlay status: " .. tostring(err)) end
end)
