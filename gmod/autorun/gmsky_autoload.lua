-- gmsky (Gmod x Skyrim) auto-loader, installed once into garrysmod/lua/autorun/server/ by
-- tools/install_gmod_autoload.ps1. It never needs changing: all of gmsky's own code lives in the
-- project folder and is loaded by require("gmsky"). Delete this file to stop gmsky loading by itself.
--
-- Single-player only. It waits until the player is fully in the game (the first real command from
-- the player's game, the documented way to tell; PlayerInitialSpawn runs too early), because gmsky
-- then also loads its client side.
if not game.SinglePlayer() then return end

hook.Add("SetupMove", "gmsky_autoload", function(ply, mv, cmd)
	if cmd:IsForced() then return end -- not yet a real command from the player's game
	hook.Remove("SetupMove", "gmsky_autoload")
	timer.Simple(0, function() -- outside the hook call, since gmsky adds hooks of its own
		if not gmsky then require("gmsky") end
	end)
end)
