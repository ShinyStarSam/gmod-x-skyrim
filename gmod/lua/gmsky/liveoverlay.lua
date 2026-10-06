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
-- Step W1 (DESIGN.md "Garry's Mod's world objects and effects in Skyrim"): with gmsky_overlay_world
-- 1 (the default) the frame is wiped earlier, when Source starts drawing entities in the main view
-- (PreDrawOpaqueRenderables), so gm_flatmap's floor and sky are gone but grenades, rockets,
-- explosions, tracers, beams and props drawn after it stay in the picture. The view model's wipe is
-- the fallback for frames where that didn't happen. Test 70: Garry's Mod says "drawing the sky" on
-- every main view of a map with a 2D sky (the wiki's note), so only the 3D sky is skipped now.
--
-- Step W2 (SkyCraft's way: Skyrim hides them behind its own depth buffer, W3): where the view
-- model, hands or HUD start, the frame so far is the world layer: gmsky.OverlayGrabWorld() copies
-- it, then it is wiped and the hands / HUD are drawn on transparent as before. The module sends HUD
-- picture, world layer and depth together (capture.rs).
--
-- Step W4 (after test 71): the depth is drawn inside the main view, right after its see-through
-- pass (same camera): each Garry's Mod entity with a model is drawn again with ent:DrawModel()
-- under Source's DepthWrite material ($color_depth: red = distance / its far constant) into a
-- half-size RGBA16F target (render.RenderView ignored the override in test 71). A small square at a
-- known distance in the top-left corner lets the module work out the far constant itself. The
-- camera goes along (gmsky.OverlayView) so Skyrim can re-project the world layer onto its own
-- camera. Alpha writes are off in the see-through pass (effects add light; test 71's black boxes).
-- World layer and depth are only sent while something can show (an entity, or a shot / effect in
-- the last second); otherwise only the hands / HUD picture (cheap).
--
-- After test 72: the module copies the pictures at the end of the frame (Source holds drawing back,
-- so copies made when Lua asked were empty), so the world layer is first copied into its own
-- picture (render.CopyRenderTargetToTexture). Solid models get their pixels stamped opaque (alpha 1,
-- colour untouched) after the see-through pass, and the view model and hands after they are drawn:
-- see-through parts of solid models (some props, the RPG, the tool gun's screen) showed Skyrim
-- through them.
--
-- After test 73: Source's DepthWrite drew nothing here (every depth picture empty), so the depth
-- picture is drawn with fog instead: models black under white linear fog from 0 to FOG_END, so a
-- pixel's grey level grows with its distance. Calibration squares at known distances let the module
-- work out the scale and whether the fog measures along the view or along the ray (capture.rs). The
-- stamp no longer calls itself (redrawing the view model ran its hook again: thousands of stamps a
-- frame, a stack overflow, ~12 frames/s), and leaves see-through and glowing sub-materials out (the
-- RPG's laser glow became a black bar).

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

-- Step W1: keep Garry's Mod's world objects and effects (wiped at the entity pass, not the view model).
-- Off by default since G3b: Skyrim draws Garry's Mod's objects itself (render.lua); 1 brings the
-- picture world layer back (objects then show twice).
L.world = L.world or CreateConVar("gmsky_overlay_world", "0", 0,
	"1 = keep Garry's Mod's world objects and effects (grenades, explosions, props) in the overlay")
-- Per 5 s status line: where frames were wiped, and entity passes not used (why).
L.wipes = L.wipes or {}

-- Step W2: the world layer's depth for Skyrim's depth test.
L.depth = L.depth or CreateConVar("gmsky_overlay_depth", "1", 0,
	"1 = send the depth of Garry's Mod's world objects so Skyrim hides them behind its walls")
-- After test 73: the fog's end and the calibration squares' distances (Garry's Mod units); each
-- square covers 8 x 8 depth pixels (capture.rs: A at (4, 4) and C at (4, h/2) at D1, B at (12, 4)
-- at D2, D at (20, 4) plain white).
L.FOG_END = 20000
L.D1, L.D2 = 500, 4000
L.SQ = 8

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
		if L.frame.noAlpha then
			render.OverrideAlphaWriteEnable(true, false) -- still in the see-through pass (step W4)
		else
			render.OverrideAlphaWriteEnable(L.frame.alphaForced and true or false, true)
		end
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

local function Target()
	return render.GetRenderTarget and render.GetRenderTarget() or nil
end

-- Step W1: Source has drawn the map (gm_flatmap's floor, its sky) and is about to draw entities.
-- Only the main view counts: not the 3D sky, not depth-only passes (shadows), and not a view drawn
-- into another picture (a reflection, a camera screen), which has a different render target than
-- the frame had at PreRender. Must return nothing (true would stop Source drawing the entities).
-- The DepthWrite material that draws every object as its distance (step W2).
local function DepthMaterial()
	-- After test 73: black, fogged white with distance.
	L.depthMat = L.depthMat or CreateMaterial("gmsky_depthfog", "UnlitGeneric", {
		["$basetexture"] = "vgui/white", ["$color"] = "[0 0 0]", ["$vertexalpha"] = "0", ["$vertexcolor"] = "0",
	})
	return L.depthMat
end

hook.Add("PreDrawOpaqueRenderables", "gmsky_liveoverlay", function(depth, skybox, sky3d)
	local f = L.frame
	if not f.active or f.cleared or not L.world:GetBool() then return end
	-- (skybox is true on every main view of a 2D-sky map, test 70: not a reason to skip.)
	if depth or sky3d then
		Count(depth and "depth pass skipped" or "3D sky pass skipped")
		return
	end
	if Target() ~= f.target then
		Count("other picture skipped")
		return
	end
	ClearOnce("entities")
end)

-- The main view's own passes after the entity wipe: not depth-only passes, not the 3D sky, not
-- another picture.
local function MainPass(f, depth, sky3d)
	return f.active and f.cleared == "entities" and not depth and not sky3d and Target() == f.target
end

-- Step W4: something Garry's Mod draws in the world happened recently (shots, effects, new entities).
function L.Poke() L.lastActivity = RealTime and RealTime() or 0 end
hook.Add("EntityFireBullets", "gmsky_liveoverlay", function() L.Poke() end)
hook.Add("OnEntityCreated", "gmsky_liveoverlay", function(e)
	if IsValid(e) and not e:GetClass():find("^gmsky_") then L.Poke() end
end)
if util and util.Effect and not L.effectWrapped then
	L.effectWrapped = true
	local orig = util.Effect
	util.Effect = function(...) L.Poke() return orig(...) end
end

-- Entities whose model is drawn in the world (not the player, hands, view model, held weapons,
-- our own invisible ground and stand-ins); `any` = also something without a model (an effect).
local SKIP_DRAW = { player = true, gmod_hands = true, predicted_viewmodel = true, viewmodel = true, worldspawn = true }
local function WorldEntities()
	local list, any = {}, false
	if not (ents and ents.GetAll and LocalPlayer) then return list, any end
	local ply = LocalPlayer()
	for _, e in ipairs(ents.GetAll()) do
		if IsValid(e) and not e:IsDormant() and not e:GetNoDraw() then
			local c = e:GetClass()
			if not SKIP_DRAW[c] and not c:find("^gmsky_") and e ~= ply and not (e:IsWeapon() and IsValid(e:GetOwner())) then
				any = true
				local m = e:GetModel()
				if m and m ~= "" then list[#list + 1] = e end
			end
		end
	end
	return list, any
end

-- Step W4: alpha writes off for the see-through pass, so effects only add light.
hook.Add("PreDrawTranslucentRenderables", "gmsky_liveoverlay", function(depth, skybox, sky3d)
	local f = L.frame
	if f.noAlphaDone or not MainPass(f, depth, sky3d) then return end
	f.noAlphaDone, f.noAlpha = true, true
	render.OverrideAlphaWriteEnable(true, false)
end)

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

-- Step W4: the camera, and the depth of the world objects, drawn in the main view.
local function DepthStep(f)
	local t0 = SysTime()
	local list, any = WorldEntities()
	f.worldActive = any or (RealTime and L.lastActivity and RealTime() - L.lastActivity < 1) or false
	if not f.worldActive then
		Count("nothing to show")
		return
	end
	Stamp(list)
	render.OverrideAlphaWriteEnable(true, false) -- back to the see-through pass's setting
	local v = render.GetViewSetup and render.GetViewSetup()
	if v and gmsky.OverlayView then
		gmsky.OverlayView(v.origin.x, v.origin.y, v.origin.z, v.angles.p, v.angles.y, v.angles.r, v.fov, v.aspect)
	end
	if not (v and L.depth:GetBool() and gmsky.OverlayGrabDepth) then return end
	local w, h = math.max(16, math.floor(ScrW() / 2)), math.max(16, math.floor(ScrH() / 2))
	if not (L.depthRt and L.depthRt.w == w and L.depthRt.h == h) then
		-- Point sampling, clamped (1 + 4 + 8); its own depth buffer; four half floats.
		local name = string.format("gmsky_depth_%dx%d", w, h)
		L.depthRt = { w = w, h = h, rt = GetRenderTargetEx(name, w, h, RT_SIZE_LITERAL, MATERIAL_RT_DEPTH_SEPARATE, 13, 0,
			IMAGE_FORMAT_RGBA16161616F) }
	end
	local mat = DepthMaterial()
	render.OverrideAlphaWriteEnable(false) -- the material's alpha 1 marks a real depth
	render.PushRenderTarget(L.depthRt.rt)
	render.Clear(0, 0, 0, 0, true, true)
	-- After test 73: white linear fog from 0 to FOG_END (the old fog put back afterwards).
	local fogMode = render.GetFogMode and render.GetFogMode()
	local fogStart, fogEnd = 0, 0
	if render.GetFogDistances then fogStart, fogEnd = render.GetFogDistances() end
	local fr, fg, fb = 0, 0, 0
	if render.GetFogColor then fr, fg, fb = render.GetFogColor() end
	render.FogMode(MATERIAL_FOG_LINEAR)
	render.FogStart(0)
	render.FogEnd(L.FOG_END)
	render.FogColor(255, 255, 255)
	render.FogMaxDensity(1)
	-- The calibration squares (drawn first; a model in front of one spoils that frame's
	-- calibration, and the module keeps the last good one).
	local fwd, right, up = v.angles:Forward(), v.angles:Right(), v.angles:Up()
	local tx = math.tan(math.rad(v.fov / 2))
	local ty = tx / v.aspect
	local function Square(px, py, d, m)
		local nx, ny = -1 + 2 * px / w, 1 - 2 * py / h
		local pos = v.origin + (fwd + right * (nx * tx) + up * (ny * ty)) * d
		local size = L.SQ * 2 * d * tx / w
		render.SetMaterial(m)
		render.DrawQuadEasy(pos, -fwd, size, size, Color(255, 255, 255), 0)
	end
	Square(4, 4, L.D1, mat)
	Square(12, 4, L.D2, mat)
	Square(4, math.floor(h / 2), L.D1, mat)
	Square(20, 4, L.D1, StampMaterial())
	render.MaterialOverride(mat)
	for _, e in ipairs(list) do e:DrawModel() end
	render.MaterialOverride(nil)
	render.FogMode(fogMode or MATERIAL_FOG_NONE)
	render.FogStart(fogStart or 0)
	render.FogEnd(fogEnd or 0)
	render.FogColor(fr or 0, fg or 0, fb or 0)
	gmsky.OverlayGrabDepth(L.D1, L.D2)
	render.PopRenderTarget()
	render.OverrideAlphaWriteEnable(true, false) -- back to the see-through pass's setting
	Count("depth drawn")
	L.depthMs = (L.depthMs or 0) + (SysTime() - t0) * 1000
	L.depthEnts = (L.depthEnts or 0) + #list
end

hook.Add("PostDrawTranslucentRenderables", "gmsky_liveoverlay", function(depth, skybox, sky3d)
	local f = L.frame
	if f.depthDone or not MainPass(f, depth, sky3d) then return end
	f.depthDone = true
	DepthStep(f)
end)

-- Step W2: the world is done (the view model, hands or HUD start): copy it as the world layer (if
-- anything can show) and wipe it, once per frame, only if the frame was wiped at the entity pass.
local function SplitLayers()
	local f = L.frame
	if not f.active or f.split or f.cleared ~= "entities" then return end
	f.split = true
	if f.worldActive and gmsky.OverlayGrabWorld then
		local w, h = ScrW(), ScrH()
		if not (L.worldRt and L.worldRt.w == w and L.worldRt.h == h) then
			-- Point sampling, clamped (1 + 4 + 8); no depth; 8 bits per channel with alpha.
			L.worldRt = { w = w, h = h, rt = GetRenderTargetEx(string.format("gmsky_world_%dx%d", w, h), w, h, RT_SIZE_LITERAL,
				MATERIAL_RT_DEPTH_NONE, 13, 0, IMAGE_FORMAT_BGRA8888) }
		end
		render.CopyRenderTargetToTexture(L.worldRt.rt)
		render.PushRenderTarget(L.worldRt.rt)
		gmsky.OverlayGrabWorld()
		render.PopRenderTarget()
	end
	if f.noAlpha then
		f.noAlpha = false
		render.OverrideAlphaWriteEnable(false)
	end
	render.Clear(0, 0, 0, 0, false, false)
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
	L.frame = { active = L.on, target = L.on and Target() or nil }
end)

hook.Add("PreDrawViewModel", "gmsky_liveoverlay", function() SplitLayers() ClearOnce("view model") end)
hook.Add("PreDrawPlayerHands", "gmsky_liveoverlay", function() SplitLayers() ClearOnce("hands") end)
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
	SplitLayers()
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
	if f.noAlpha then
		f.noAlpha = false
		render.OverrideAlphaWriteEnable(false)
	end
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

-- Garry's Mod things within 4,000 units of the eye that the picture can now show, by class (not
-- the player, the hands, held weapons, or our own invisible ground and stand-ins).
local SKIP = { player = true, gmod_hands = true, predicted_viewmodel = true, viewmodel = true, worldspawn = true }
local function NearThings()
	if not (ents and ents.FindInSphere and LocalPlayer) then return "?" end
	local ply = LocalPlayer()
	if not IsValid(ply) then return "?" end
	local byClass = {}
	for _, e in ipairs(ents.FindInSphere(ply:EyePos(), 4000)) do
		local c = e:GetClass()
		if not SKIP[c] and not c:find("^gmsky_") and not e:GetNoDraw() and not (e:IsWeapon() and IsValid(e:GetOwner())) then
			byClass[c] = (byClass[c] or 0) + 1
		end
	end
	return CountList(byClass, 8)
end

-- Step W2 calibration: Garry's Mod's own trace to the object under the crosshair, measured along
-- the view, next to the depth picture's distance at the screen centre. Equal = far constant right.
local function Calibration()
	if not (util and util.TraceLine and LocalPlayer and gmsky.OverlayDepthCentre) then return "calibration: ?" end
	local ply = LocalPlayer()
	if not IsValid(ply) then return "calibration: ?" end
	local eye, aim = ply:EyePos(), ply:GetAimVector()
	local tr = util.TraceLine({ start = eye, endpos = eye + aim * 16384, filter = ply })
	local e = tr.Entity
	local depth = gmsky.OverlayDepthCentre()
	local what
	if IsValid(e) and not e:GetClass():find("^gmsky_") then
		local along = (tr.HitPos - eye):Dot(aim)
		what = string.format("crosshair on %s at %.1f along the view", e:GetClass(), along)
		if depth then what = what .. string.format(", depth picture %.1f (ratio %.3f)", depth, depth / math.max(along, 0.001)) end
	else
		what = "crosshair on no Garry's Mod object"
		if depth then what = what .. string.format(", depth picture %.1f", depth) end
	end
	return "calibration: " .. what
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
			local drawn = L.wipes["depth drawn"] or 0
			gmsky.Log(string.format("live overlay world (gmsky_overlay_world %s, depth %s): frames %s | depth %.3f ms avg, %.1f entities avg | %d models stamped solid (%d see-through parts left out) | near: %s | %s",
				L.world:GetString(), L.depth:GetString(), CountList(L.wipes, 10),
				drawn > 0 and (L.depthMs or 0) / drawn or 0, drawn > 0 and (L.depthEnts or 0) / drawn or 0, L.stamped or 0, L.stampedParts or 0,
				NearThings(), Calibration()))
			L.wipes = {}
			L.depthMs, L.depthEnts, L.stamped, L.stampedParts = 0, 0, 0, 0
		end
	end)
	if not ok then gmsky.Log("ERROR in live overlay status: " .. tostring(err)) end
end)
