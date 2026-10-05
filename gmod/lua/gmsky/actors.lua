-- gmsky server side, Phase 3 step 3b: Skyrim's actors near the player (people, creatures), read
-- every tick from Skyrim's table (nearest first, positions already in Garry's Mod's own world
-- units). Log only for now; step 3c gives each one an invisible stand-in to hit.
-- DESIGN.md "Phase 3 steps".

local gmsky = gmsky
local SCALE = 64 / 120.5 -- Garry's Mod units per Skyrim unit (gmsky_protocol.h kGmodPerSkyrim)

local A = {
	seq = 0,
	list = {},      -- the newest table: { id, flags, x, y, z, yaw, radius, height, hp, level, name }
	byId = {},
	frame = 0,
	stats = { reads = 0, appeared = 0, left = 0 },
	logged = 0,     -- appear / leave lines this second (capped)
}
gmsky.actors = A

local FLAGS = { { 0x1, "hostile" }, { 0x8, "fighting" }, { 0x4, "essential" }, { 0x2, "dead" } }
function A.FlagText(f)
	local out = {}
	for _, p in ipairs(FLAGS) do
		if bit.band(f, p[1]) ~= 0 then out[#out + 1] = p[2] end
	end
	return table.concat(out, " ")
end

local function Describe(a)
	return string.format("%s %08X lvl %d", a.name ~= "" and a.name or "(no name)", a.id, a.level)
end

local function Note(line)
	if A.logged < 10 then gmsky.Log(line) end
	A.logged = A.logged + 1
end

-- Every tick (server.lua): take the newest table if Skyrim wrote one.
function A.Update()
	local seq, frame, list = gmsky.ReadActors(A.seq)
	if seq == false then return end
	if seq == nil then
		-- No table for the current origin (none yet, or the origin just changed).
		if #A.list > 0 then
			Note(string.format("actors: table dropped (%d actors), no table for this origin yet", #A.list))
			A.list, A.byId = {}, {}
		end
		return
	end
	A.seq, A.frame = seq, frame
	local byId = {}
	for _, a in ipairs(list) do
		byId[a.id] = a
		if not A.byId[a.id] then
			A.stats.appeared = A.stats.appeared + 1
			Note("actor near: " .. Describe(a) .. (a.flags ~= 0 and (" (" .. A.FlagText(a.flags) .. ")") or ""))
		end
	end
	for id, a in pairs(A.byId) do
		if not byId[id] then
			A.stats.left = A.stats.left + 1
			Note("actor gone: " .. Describe(a))
		end
	end
	A.list, A.byId = list, byId
	A.stats.reads = A.stats.reads + 1
end

-- Once a second (server.lua's status timer): the count and the nearest three, with their distance
-- in Garry's Mod units and in Skyrim units (to compare with Skyrim's ACTORS line).
function A.Status(ply)
	local s = A.stats
	local line = ""
	if #A.list > 0 or s.left > 0 then
		local near = {}
		if ply then
			local p = ply:GetPos()
			for i = 1, math.min(3, #A.list) do
				local a = A.list[i]
				local d = math.sqrt((a.x - p.x) ^ 2 + (a.y - p.y) ^ 2 + (a.z - p.z) ^ 2)
				near[#near + 1] = string.format("%s at %.0f (%.0f Skyrim) hp %.0f%%%s", Describe(a), d, d / SCALE, a.hp * 100,
					a.flags ~= 0 and (" " .. A.FlagText(a.flags)) or "")
			end
		end
		line = string.format(" | actors %d (tables read %d, +%d -%d)%s", #A.list, s.reads, s.appeared, s.left,
			#near > 0 and (": " .. table.concat(near, "; ")) or "")
		if A.logged > 10 then line = line .. string.format(" (%d appear/leave lines not shown)", A.logged - 10) end
	end
	A.stats = { reads = 0, appeared = 0, left = 0 }
	A.logged = 0
	return line
end

gmsky.Log("actors.lua loaded")
