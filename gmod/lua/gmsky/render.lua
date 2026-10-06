-- gmsky render: Garry's Mod's objects drawn by Skyrim itself (geometry route step G3b, DESIGN.md
-- §12 "Garry's Mod's things drawn inside Skyrim"). Client side only (client.lua includes it).
--
-- Every Garry's Mod frame, the objects with a model near the player (props, ragdolls, NPCs,
-- rockets, grenades; not the player's own body, weapons, view model and hands, nor our own gmsky_*
-- entities) are sent to Skyrim: each model once (util.GetModelMeshes: triangles, texture
-- coordinates, normals, bone weights, bind poses), each texture once (its .vtf from Garry's Mod's
-- own file system, decoded by the module, vtf.rs), and every frame each object's bone matrices
-- (gmsky.RenderFrame; the module turns them into "rest pose -> world" and subtracts the anchor).
-- Skyrim draws them inside its own frame with its own camera (World.cpp), so they sit still in its
-- world and hide behind its walls. As SkyCraft (MIT) does with Minecraft's entities, except that
-- SkyCraft sends posed triangles every frame and we send each model once plus bone matrices.
--
-- Console (client): gmsky_render 0/1 (on by default).
-- Log (every 5 s): logs/gmod_client.log, lines starting "render:" and "render out:".
--
-- Step G6 (after tests 79-80, a held prop trailed the view): what the player holds with the physgun,
-- the gravity gun or +use is marked by the server (NW2 bool "gmsky_held"), sent with a HELD flag, and
-- each frame carries the view Garry's Mod drew it with; Skyrim then keeps a held object where it was
-- in front of Garry's Mod's camera, in front of its own camera of this frame (as SkyCraft keeps the
-- player's body on the feet its camera follows). gmsky_render_held 0 (client) sends no flag.

local gmsky = gmsky

if SERVER then
	-- Pick-ups and drops: physgun, gravity gun, +use.
	local function Held(ent, on)
		if IsValid(ent) then ent:SetNW2Bool("gmsky_held", on) end
	end
	hook.Add("OnPhysgunPickup", "gmsky_render_held", function(ply, ent) Held(ent, true) end)
	hook.Add("PhysgunDrop", "gmsky_render_held", function(ply, ent) Held(ent, false) end)
	hook.Add("GravGunOnPickedUp", "gmsky_render_held", function(ply, ent) Held(ent, true) end)
	hook.Add("GravGunOnDropped", "gmsky_render_held", function(ply, ent) Held(ent, false) end)
	hook.Add("OnPlayerPhysicsPickup", "gmsky_render_held", function(ply, ent) Held(ent, true) end)
	hook.Add("OnPlayerPhysicsDrop", "gmsky_render_held", function(ply, ent) Held(ent, false) end)
	gmsky.Log("render.lua loaded (server: held objects marked)")
	return
end
local R = gmsky.render or {}
gmsky.render = R

R.cvar = R.cvar or CreateConVar("gmsky_render", "1", 0,
	"1 = send Garry's Mod's objects to Skyrim, which draws them in its own world (geometry route)")
R.held = R.held or CreateConVar("gmsky_render_held", "1", 0,
	"1 = Skyrim keeps what you hold (physgun, gravity gun, +use) in front of its own camera (step G6)")
-- G5b bundle: switches to compare in game (sent with every frame; Skyrim reads them live).
R.shadows = R.shadows or CreateConVar("gmsky_render_shadows", "1", 0, "1 = shadows on Garry's Mod's objects and from them (G5b)")
R.normals = R.normals or CreateConVar("gmsky_render_normals", "1", 0, "1 = Garry's Mod's normal maps (surface bumps) in Skyrim")
R.normalFlip = R.normalFlip or CreateConVar("gmsky_render_normal_flip", "0", 0, "1 = read normal maps' green channel the other way")
R.phong = R.phong or CreateConVar("gmsky_render_phong", "1", 0, "1 = Garry's Mod's phong shine in Skyrim")
R.RANGE = 8000             -- Garry's Mod units from the eye
R.NEW_MODELS_PER_FRAME = 1  -- reading a model costs a few ms

R.models = R.models or {}     -- "model|skin" -> { id, bones, failed, why }
R.materials = R.materials or {}  -- material name -> { tex, flags, aref }
R.textures = R.textures or {}  -- texture name -> id (0 = could not be sent)
R.nextModel = R.nextModel or 1
R.nextTexture = R.nextTexture or 1
R.stats = R.stats or {}

local function ResetStats()
	R.stats = { frames = 0, objects = 0, held = 0, skipped = 0, failed = 0, newModels = 0, modelMs = 0,
		modelMsMax = 0, frameMs = 0, frameMsMax = 0, newTextures = 0, textureFails = 0 }
end
if not R.stats.frames then ResetStats() end

-- Source material flags ($flags) -> GMSKY_RND_MAT_*.
local MAT_FLAGS = {
	{ 0x200000, 0x1 },  -- translucent
	{ 0x80, 0x2 },      -- additive
	{ 0x100, 0x4 },     -- alpha test
	{ 0x40, 0x8 },      -- self-illuminated
	{ 0x2000, 0x10 },   -- no culling
}

-- A texture's id in Skyrim, sending it the first time (0 = none / could not be sent).
-- linear: a normal map (not a colour), read as is by Skyrim.
local function TextureId(name, linear)
	local key = linear and (name .. "|linear") or name
	local id = R.textures[key]
	if id then return id end
	id = 0
	local data = file.Read("materials/" .. name .. ".vtf", "GAME")
	if not data then
		gmsky.Log("render: texture " .. name .. ": no file materials/" .. name .. ".vtf")
		R.stats.textureFails = R.stats.textureFails + 1
	else
		local tid = R.nextTexture
		local ok, line = gmsky.RenderTexture(tid, data, linear and true or false)
		if ok then
			R.nextTexture = tid + 1
			id = tid
			R.stats.newTextures = R.stats.newTextures + 1
			gmsky.Log("render: " .. name .. ": " .. line)
		else
			R.stats.textureFails = R.stats.textureFails + 1
			gmsky.Log("render: texture " .. name .. " not sent: " .. tostring(line))
		end
	end
	R.textures[key] = id
	return id
end

-- What Skyrim needs from a material: texture id, flags, alpha reference, normal map id, phong
-- exponent and boost (G5b bundle: $bumpmap unless $ssbump; $phong, $phongexponent (5),
-- $phongboost (1), $basemapalphaphongmask).
local function MaterialInfo(name)
	local info = R.materials[name]
	if info then return info end
	info = { tex = 0, flags = 0, aref = 0, bump = 0, phongExp = 0, phongBoost = 0 }
	local mat = Material(name)
	if mat and not mat:IsError() then
		local f = mat:GetInt("$flags") or 0
		for _, pair in ipairs(MAT_FLAGS) do
			if bit.band(f, pair[1]) ~= 0 then info.flags = info.flags + pair[2] end
		end
		info.aref = mat:GetFloat("$alphatestreference") or 0
		if bit.band(info.flags, 0x4) ~= 0 and info.aref <= 0 then info.aref = 0.5 end
		local tex = mat:GetTexture("$basetexture")
		if tex and not tex:IsError() then
			info.tex = TextureId(tex:GetName())
		end
		if (mat:GetInt("$ssbump") or 0) == 0 then
			local bump = mat:GetTexture("$bumpmap")
			if bump and not bump:IsError() then
				info.bump = TextureId(bump:GetName(), true)
			end
		end
		if (mat:GetInt("$phong") or 0) ~= 0 then
			info.flags = info.flags + 0x20
			local e, b = mat:GetFloat("$phongexponent"), mat:GetFloat("$phongboost")
			info.phongExp = (e and e > 0) and e or 5
			info.phongBoost = (b and b > 0) and b or 1
			if (mat:GetInt("$basemapalphaphongmask") or 0) ~= 0 then info.flags = info.flags + 0x40 end
		end
	else
		gmsky.Log("render: material " .. tostring(name) .. " not found (drawn white)")
	end
	R.materials[name] = info
	return info
end

-- Reads a model and sends it. Returns the cache entry (failed = true if it can't be drawn).
local function Prepare(key, model, skin)
	local t0 = SysTime()
	local entry = { id = R.nextModel, bones = 0 }
	R.models[key] = entry
	local meshes, binds = util.GetModelMeshes(model, 0, 0, skin)
	if not meshes or #meshes == 0 or not binds or not binds[0] then
		entry.failed, entry.why = true, "util.GetModelMeshes gave nothing"
	else
		local bones = #binds + 1  -- bind poses are numbered from 0
		local verts, parts, bindNums = {}, {}, {}
		local nv, np, corners = 0, 0, 0
		for _, m in ipairs(meshes) do
			local tris = m.triangles
			if tris and #tris >= 3 then
				local first = corners
				for _, v in ipairs(tris) do
					local p, n, w = v.pos, v.normal, v.weights
					verts[nv + 1], verts[nv + 2], verts[nv + 3] = p.x, p.y, p.z
					if n then
						verts[nv + 4], verts[nv + 5], verts[nv + 6] = n.x, n.y, n.z
					else
						verts[nv + 4], verts[nv + 5], verts[nv + 6] = 0, 0, 1
					end
					-- Tangent + handedness: userdata (tangent x, y, z, handedness), else tangent.
					local ud, t = v.userdata, v.tangent
					if ud and ud[1] then
						verts[nv + 7], verts[nv + 8], verts[nv + 9], verts[nv + 10] = ud[1], ud[2], ud[3], ud[4] or 1
					elseif t then
						verts[nv + 7], verts[nv + 8], verts[nv + 9], verts[nv + 10] = t.x, t.y, t.z, 1
					else
						verts[nv + 7], verts[nv + 8], verts[nv + 9], verts[nv + 10] = 0, 0, 0, 1
					end
					verts[nv + 11], verts[nv + 12] = v.u or 0, v.v or 0
					for k = 1, 4 do
						local bw = w and w[k]
						verts[nv + 11 + 2 * k] = bw and bw.bone or 0
						verts[nv + 12 + 2 * k] = bw and bw.weight or 0
					end
					nv = nv + 20
					corners = corners + 1
				end
				local info = MaterialInfo(m.material)
				parts[np + 1], parts[np + 2], parts[np + 3], parts[np + 4], parts[np + 5] =
					first, corners - first, info.tex, info.flags, info.aref
				parts[np + 6], parts[np + 7], parts[np + 8] = info.bump, info.phongExp, info.phongBoost
				np = np + 8
			end
		end
		for b = 0, bones - 1 do
			local mtx = binds[b] and binds[b].matrix
			local k = b * 12
			if mtx then
				bindNums[k + 1], bindNums[k + 2], bindNums[k + 3], bindNums[k + 4],
				bindNums[k + 5], bindNums[k + 6], bindNums[k + 7], bindNums[k + 8],
				bindNums[k + 9], bindNums[k + 10], bindNums[k + 11], bindNums[k + 12] = mtx:Unpack()
			else
				bindNums[k + 1], bindNums[k + 2], bindNums[k + 3], bindNums[k + 4] = 1, 0, 0, 0
				bindNums[k + 5], bindNums[k + 6], bindNums[k + 7], bindNums[k + 8] = 0, 1, 0, 0
				bindNums[k + 9], bindNums[k + 10], bindNums[k + 11], bindNums[k + 12] = 0, 0, 1, 0
			end
		end
		if corners == 0 then
			entry.failed, entry.why = true, "no triangles"
		else
			local ok, line = gmsky.RenderModel(entry.id, bones, verts, parts, bindNums)
			if ok then
				entry.bones = bones
				R.nextModel = R.nextModel + 1
				gmsky.Log(string.format("render: %s skin %d: %s (%.1f ms)", model, skin, line, (SysTime() - t0) * 1000))
			else
				entry.failed, entry.why = true, tostring(line)
			end
		end
	end
	if entry.failed then
		R.stats.failed = R.stats.failed + 1
		gmsky.Log(string.format("render: %s skin %d can't be drawn: %s", model, skin, entry.why))
	end
	local ms = (SysTime() - t0) * 1000
	R.stats.newModels = R.stats.newModels + 1
	R.stats.modelMs = R.stats.modelMs + ms
	R.stats.modelMsMax = math.max(R.stats.modelMsMax, ms)
	return entry
end

-- A number for each entity, stable while it exists: its entity index, or one of our own for
-- client-side entities (index -1, such as ragdolls of dead NPCs).
R.ownNumbers = R.ownNumbers or setmetatable({}, { __mode = "k" })
R.nextOwn = R.nextOwn or 1000000
function R.EntityNumber(e)
	local i = e:EntIndex()
	if i and i > 0 then return i end
	local own = R.ownNumbers[e]
	if not own then
		own = R.nextOwn
		R.nextOwn = own + 1
		R.ownNumbers[e] = own
	end
	return own
end

local SKIP_CLASS = { viewmodel = true, gmod_hands = true, predicted_viewmodel = true, worldspawn = true }

-- Should this entity be drawn in Skyrim?
function R.Wanted(e, me, eye)
	if not IsValid(e) or e == me or e:IsDormant() or e:GetNoDraw() then return false end
	local class = e:GetClass()
	if SKIP_CLASS[class] or string.sub(class, 1, 6) == "gmsky_" then return false end
	local model = e:GetModel()
	if not model or model == "" or string.sub(model, 1, 1) == "*" then return false end
	if e:IsWeapon() and e:GetOwner() == me then return false end
	if e:GetColor().a == 0 then return false end
	return e:GetPos():DistToSqr(eye) < R.RANGE * R.RANGE
end

local lastFrame = -1
function R.Frame()
	if not R.cvar:GetBool() or not gmsky.RenderFrame then return end
	local fn = FrameNumber()
	if fn == lastFrame then return end
	lastFrame = fn
	local t0 = SysTime()
	local me = LocalPlayer()
	if not IsValid(me) then return end
	local eye = me:EyePos()
	local flat, n, objects, budget = {}, 0, 0, R.NEW_MODELS_PER_FRAME
	local heldOn = R.held:GetBool()
	local s = R.stats
	for _, e in ipairs(ents.GetAll()) do
		if R.Wanted(e, me, eye) then
			local model, skin = e:GetModel(), e:GetSkin() or 0
			local key = model .. "|" .. skin
			local entry = R.models[key]
			if not entry and budget > 0 then
				budget = budget - 1
				entry = Prepare(key, model, skin)
			end
			if entry and not entry.failed then
				e:SetupBones()
				local c = e:GetColor()
				local held = heldOn and e:GetNW2Bool("gmsky_held", false)
				flat[n + 1], flat[n + 2], flat[n + 3] = entry.id, c.r + c.g * 256 + c.b * 65536 + c.a * 16777216, entry.bones
				flat[n + 4], flat[n + 5] = R.EntityNumber(e), held and 1 or 0
				n = n + 5
				if held then s.held = s.held + 1 end
				for b = 0, entry.bones - 1 do
					local m = e:GetBoneMatrix(b) or e:GetWorldTransformMatrix()
					flat[n + 1], flat[n + 2], flat[n + 3], flat[n + 4],
					flat[n + 5], flat[n + 6], flat[n + 7], flat[n + 8],
					flat[n + 9], flat[n + 10], flat[n + 11], flat[n + 12] = m:Unpack()
					n = n + 12
				end
				objects = objects + 1
			elseif not entry or entry.failed then
				R.stats.skipped = R.stats.skipped + 1
			end
		end
	end
	-- The camera Garry's Mod draws this frame with (for held objects, step G6).
	local v = render.GetViewSetup and render.GetViewSetup()
	local switches = (R.normalFlip:GetBool() and 0x2 or 0) + (R.normals:GetBool() and 0 or 0x4)
		+ (R.phong:GetBool() and 0 or 0x8) + (R.shadows:GetBool() and 0 or 0x10)
	if v and v.origin and v.angles then
		gmsky.RenderFrame(fn, flat, v.origin.x, v.origin.y, v.origin.z, v.angles.p, v.angles.y, v.angles.r, switches)
	else
		gmsky.RenderFrame(fn, flat)
	end
	local ms = (SysTime() - t0) * 1000
	s.frames, s.objects = s.frames + 1, s.objects + objects
	s.frameMs, s.frameMsMax = s.frameMs + ms, math.max(s.frameMsMax, ms)
end

hook.Add("PostDrawOpaqueRenderables", "gmsky_render", function(depth, skybox, sky3d)
	-- skybox is true on every main view of a 2D-sky map such as gm_flatmap (test 70; test 78 sent
	-- nothing because of it): only the depth pass and the 3D sky are skipped.
	if depth or sky3d then return end
	local ok, err = pcall(R.Frame)
	if not ok then
		R.errors = (R.errors or 0) + 1
		if R.errors <= 5 then gmsky.Log("render: ERROR " .. tostring(err)) end
	end
end)

function R.Status()
	local s = R.stats
	local known, failed = 0, 0
	for _, e in pairs(R.models) do
		if e.failed then failed = failed + 1 else known = known + 1 end
	end
	local textures = 0
	for _, id in pairs(R.textures) do if id > 0 then textures = textures + 1 end end
	local line = string.format("render: %s; %d frames, %.1f objects a frame, %.1f held (gmsky_render_held %s), %d skipped (model not ready / can't be drawn); "
		.. "models %d ok, %d can't be drawn (%d new, %.1f ms avg, %.1f max); textures %d (%d new, %d failed); Lua %.2f ms a frame avg, %.2f max",
		R.cvar:GetBool() and "on" or "off (gmsky_render 0)", s.frames, s.frames > 0 and s.objects / s.frames or 0,
		s.frames > 0 and s.held / s.frames or 0, R.held:GetString(), s.skipped,
		known, failed, s.newModels, s.newModels > 0 and s.modelMs / s.newModels or 0, s.modelMsMax, textures, s.newTextures, s.textureFails,
		s.frames > 0 and s.frameMs / s.frames or 0, s.frameMsMax)
	ResetStats()
	return line
end

timer.Create("gmsky_render_status", 5, 0, function()
	local ok, err = pcall(function()
		gmsky.Log(R.Status())
		if gmsky.RenderStatus then gmsky.Log(gmsky.RenderStatus()) end
	end)
	if not ok then gmsky.Log("render: ERROR in status: " .. tostring(err)) end
end)

gmsky.Log("render.lua loaded (gmsky_render " .. R.cvar:GetString() .. ")")
