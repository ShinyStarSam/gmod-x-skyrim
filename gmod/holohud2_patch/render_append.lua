
---
--- gmsky patch (Gmod x Skyrim project; our code, not HOLOHUD's). tools/build_holohud.py appends
--- this to the end of HOLOHUD 2's lua/holohud2/render.lua in our personal copy, so it can use that
--- file's local variables (MAT_FRAME, MAT_COMPOSITE, polygons, computed, r_3d, r_pp, debug,
--- draw_elements). holohud2/init.lua picks up these two functions after render.lua has run.
---
--- Why (docs/DESIGN.md "HOLOHUD 2, patched"; tests 33-34): Skyrim draws Garry's Mod's HUD from one
--- picture with premultiplied alpha (colour + (1 - alpha) x Skyrim). HOLOHUD's frame layer (the
--- see-through plates) must write its alpha, and its composite layer (glow, text, compass, radar:
--- drawn additively, it adds light) must write none. Unpatched, both write alpha 1 everywhere.
--- The layers, their colours and the curved "3D visor" polygons are exactly HOLOHUD's; only the
--- alpha writes change.
---

-- The plates' frosted glass is fake: HOLOHUD pastes a blurred, fully opaque copy of the screen
-- under each plate (modules/render.lua BlurRect), then a 94/255 black on top. That is why the plate
-- layer held alpha 255 (tests 36-37) and why our own picture showed them opaque. Skyrim can't get a
-- blur of its own world from Garry's Mod, so turn it off; the plates become real 94/255 black.
-- holohud2_r_blur is HOLOHUD's own setting ("Blurs the panels' backgrounds"); it is saved, so it
-- stays off for HOLOHUD outside this project too until set back to 1.
RunConsoleCommand( "holohud2_r_blur", "0" )

local gmsky_RenderHUDBackground = HOLOHUD2.render.RenderHUDBackground
local gmsky_RenderHUD = HOLOHUD2.render.RenderHUD

-- Alpha writes back to what gmsky's capture wants: on while it captures (gmsky.alphaForced),
-- Source's own choice otherwise.
local function gmsky_alpha_restore()
    if gmsky and gmsky.alphaForced then
        render.OverrideAlphaWriteEnable( true, true )
    else
        render.OverrideAlphaWriteEnable( false )
    end
end

-- Diagnostics (test 36): while gmsky.holohudProbe is a name, saves pictures to logs/overlay/:
-- HOLOHUD's two layer textures as they are before drawing, and the screen after each step.
local function gmsky_capture( file, w, h )
    local ok, png = pcall( render.Capture, { format = "png", x = 0, y = 0, w = w, h = h, alpha = true } )
    if ok and png then gmsky.WriteLogFile( file, png ) end
end

local function gmsky_probe( step )
    if not ( gmsky and gmsky.holohudProbe ) then return end
    gmsky_capture( gmsky.holohudProbe .. "_" .. step .. ".png", ScrW(), ScrH() )
end

local function gmsky_probe_targets()
    if not ( gmsky and gmsky.holohudProbe ) then return end
    for name, rt in pairs( { rtframe = RT_FRAME, rtcomposite = RT_COMPOSITE } ) do
        render.PushRenderTarget( rt )
        gmsky_capture( gmsky.holohudProbe .. "_" .. name .. ".png", rt:Width(), rt:Height() )
        render.PopRenderTarget()
    end
end

-- Test 37 plate experiment: our own RGBA picture (exact alpha in test 32).
local gmsky_probe_rt

local function gmsky_plate_experiment( layer )
    if not ( gmsky and gmsky.holohudProbe and gmsky.holohudProbe:find( "_plain$" ) ) then return end
    local w, h = ScrW(), ScrH()
    gmsky_probe_rt = gmsky_probe_rt or GetRenderTargetEx( "gmsky_holohud_probe_" .. w .. "_" .. h, w, h,
        RT_SIZE_LITERAL, MATERIAL_RT_DEPTH_SEPARATE, 0, 0, IMAGE_FORMAT_RGBA8888 )
    local setups = {
        over = function() render.OverrideBlend( true, BLEND_SRC_ALPHA, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD,
            BLEND_ONE, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD ) end,
        material = function() end,
        holohud = function() render.OverrideBlend( true, BLEND_SRC_ALPHA, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD ) end,
    }
    for name, setup in pairs( setups ) do
        for bg, v in pairs( { black = 0, grey = 124 } ) do
            render.PushRenderTarget( gmsky_probe_rt )
            cam.Start2D()
            render.OverrideAlphaWriteEnable( true, true )
            render.Clear( v, v, v, 0, true, true )
            setup()
            layer()
            render.OverrideBlend( false )
            gmsky_capture( gmsky.holohudProbe .. "_plate_" .. name .. "_" .. bg .. ".png", w, h )
            cam.End2D()
            render.PopRenderTarget()
        end
    end
    gmsky_alpha_restore()
end

-- One full-screen layer: on the visor's polygons when the 3D effect is on, else one quad (as
-- HOLOHUD draws it).
local function gmsky_layer( mat )
    if r_3d:GetBool() then
        surface.SetDrawColor( 255, 255, 255, 255 )
        surface.SetMaterial( mat )
        for i = 1, #polygons do surface.DrawPoly( polygons[ i ] ) end
    else
        render.SetMaterial( mat )
        render.DrawScreenQuad()
    end
end

-- The plates: colour blended "over" as HOLOHUD does, alpha written with "over" too
-- (alpha = the plate's alpha over a transparent background).
function HOLOHUD2.render.RenderHUDBackground( settings )
    if not r_pp:GetBool() or debug:GetBool() then return gmsky_RenderHUDBackground( settings ) end
    if not computed then return end
    gmsky_probe_targets()
    gmsky_plate_experiment( function() gmsky_layer( MAT_FRAME ) end )
    gmsky_probe( "1before" )
    render.OverrideAlphaWriteEnable( true, true )
    render.OverrideBlend( true, BLEND_SRC_ALPHA, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD,
        BLEND_ONE, BLEND_ONE_MINUS_SRC_ALPHA, BLENDFUNC_ADD )
    gmsky_layer( MAT_FRAME )
    render.OverrideBlend( false )
    gmsky_alpha_restore()
    gmsky_probe( "2afterframe" )
end

-- The glow layer: added light, so it must leave the picture's alpha as it is; then HOLOHUD's
-- direct-paint elements. Its own texture has alpha 255 almost everywhere (test 38). Switching
-- alpha writes off for it failed (tests 36, 37: the switch's effect depended on what was set
-- before), so instead the blend itself keeps the alpha: colour ONE + ONE (added light, as the
-- $additive material does), alpha ZERO + ONE (= the alpha already there). Test 38 showed the
-- separate alpha factors do take effect (plates written 94, not 94 x 94).
function HOLOHUD2.render.RenderHUD( settings, on_overlay )
    if not r_pp:GetBool() or debug:GetBool() then return gmsky_RenderHUD( settings, on_overlay ) end
    if not computed then return end
    gmsky_alpha_restore()
    gmsky_probe( "3beforecomposite" )
    render.OverrideBlend( true, BLEND_ONE, BLEND_ONE, BLENDFUNC_ADD, BLEND_ZERO, BLEND_ONE, BLENDFUNC_ADD )
    gmsky_layer( MAT_COMPOSITE )
    render.OverrideBlend( false )
    gmsky_alpha_restore()
    gmsky_probe( "4aftercomposite" )
    draw_elements( "PaintOver", "HUDPaintOver", settings, 0, 0, on_overlay )
    gmsky_probe( "5afterpaintover" )
end
