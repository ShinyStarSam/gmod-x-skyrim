-- gmsky, both sides, step S2j: the player's box sized from Skyrim's player (DESIGN.md S2j).
-- Garry's Mod's player is a 32 x 32 box; Skyrim's is a round capsule, narrower once scaled, so the
-- box didn't fit through places Skyrim's player does (test 136). Skyrim sends its capsule radius
-- in Garry's Mod units (gmsky.PlayerRadius). Source's movement only knows an upright box.
-- Test 137: the first choice, rule B (corners inside the capsule, half width = radius / 1.41 =
-- 7.5) let the eye get closer to walls than Skyrim's own camera ever does, and walls were cut by
-- the camera's near distance ("I see through certain walls"). Now the box is as wide as Skyrim's
-- player (half width = its radius, 10.6): the eye keeps Skyrim's own distance from walls; only
-- the box's corners stick out past Skyrim's body. Heights stay as they are (72 standing, 36
-- crouched).
-- Both sides set the same box (the client predicts movement with it).

local gmsky = gmsky

local Z = gmsky.size or {}
gmsky.size = Z

Z.on = Z.on or CreateConVar("gmsky_size_from_skyrim", "1", FCVAR_REPLICATED,
	"1 = the player's box is as wide as Skyrim's player (S2j); 0 = Garry's Mod's own 32-unit box")
local DEFAULT = 16  -- Garry's Mod's own half width
local MIN, MAX = 4, 24

-- The half width wanted now: Skyrim's player's radius, or Garry's Mod's own.
function Z.Wanted()
	local r = gmsky.PlayerRadius and gmsky.PlayerRadius()
	if not Z.on:GetBool() or not r then return DEFAULT, r end
	return math.Clamp(math.Round(r, 1), MIN, MAX), r
end

-- Sets ply's standing and crouched boxes to half width h, keeping their heights. True if changed.
function Z.Apply(ply, h)
	local lo, hi = ply:GetHull()
	if math.abs(hi.x - h) < 0.05 then return false end
	local dlo, dhi = ply:GetHullDuck()
	ply:SetHull(Vector(-h, -h, lo.z), Vector(h, h, hi.z))
	ply:SetHullDuck(Vector(-h, -h, dlo.z), Vector(h, h, dhi.z))
	gmsky.Log(string.format("size: player box half width %.1f -> %.1f (%s); standing %.0f tall, crouched %.0f",
		hi.x, h, Z.lastRadius and string.format("Skyrim's player radius %.2f", Z.lastRadius) or "Garry's Mod's own",
		hi.z - lo.z, dhi.z - dlo.z))
	return true
end

-- Twice a second: every player on the server, the local player on the client.
hook.Add("Think", "gmsky_size", function()
	local now = CurTime()
	if Z.next and now < Z.next then return end
	Z.next = now + 0.5
	local h, r = Z.Wanted()
	Z.lastRadius = Z.on:GetBool() and r or nil
	local players = SERVER and player.GetAll() or { LocalPlayer() }
	for _, ply in ipairs(players) do
		if IsValid(ply) then Z.Apply(ply, h) end
	end
end)
