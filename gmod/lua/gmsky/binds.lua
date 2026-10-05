-- gmsky client side, Phase 3 step 3a: Skyrim's clicks, mouse wheel, keys 1-6 and Q become Garry's
-- Mod's own key binds, the way a real key press does: first the PlayerBindPress hook (HOLOHUD 2's
-- weapon menu listens there), then, if no hook took it, the bind runs as a console command. So the
-- weapon menu opens and a click picks from it, and Q opens Sandbox's spawn menu.
-- While a menu cursor is up (spawn menu, any menu with a mouse), Skyrim's mouse moves and clicks
-- that cursor instead (gui.Internal*, as VRMod drives menus); a cursor arrow is drawn into the
-- overlay, since the real Windows cursor isn't in the picture. DESIGN.md "Phase 3 steps", 3a.

local gmsky = gmsky

local cursorSpeed = CreateConVar("gmsky_cursor_speed", "1", FCVAR_ARCHIVE,
	"Garry's Mod menu cursor: pixels per Skyrim mouse count")

-- GMSKY_IN_* bits in GmskyInput.buttons (protocol/gmsky_protocol.h)
local SKY_ATTACK, SKY_ATTACK2, SKY_MENU = 0x100, 0x200, 0x800
local SLOT_MASK, SLOT_SHIFT = 0xF0000, 16

-- Garry's Mod's key codes (the BUTTON_CODE enums), passed to PlayerBindPress like the engine does.
local KEY_Q_ = KEY_Q or 27
local MOUSE_LEFT_, MOUSE_RIGHT_ = MOUSE_LEFT or 107, MOUSE_RIGHT or 108
local WHEEL_UP_, WHEEL_DOWN_ = MOUSE_WHEEL_UP or 112, MOUSE_WHEEL_DOWN or 113
local function SlotKey(n) return (KEY_1 or 2) + n - 1 end

local B = {
	on = false,        -- input comes from Skyrim right now
	prev = 0,          -- Skyrim's buttons last frame
	plus = {},         -- "+binds" we ran, so each gets its "-" on release
	cursor = false,    -- a Garry's Mod menu cursor is up and Skyrim's mouse drives it
	x = nil, y = nil,  -- that cursor, in pixels
	vguiDown = {},     -- mouse buttons pressed into menus
	stats = { run = 0, taken = 0, clicks = 0, moves = 0 },
}
gmsky.binds = B

-- One key press, as the engine does it. Returns true if a hook took it.
local function Press(bind, code)
	local ply = LocalPlayer()
	if not IsValid(ply) then return true end
	if hook.Run("PlayerBindPress", ply, bind, true, code) then
		B.stats.taken = B.stats.taken + 1
		gmsky.Log(string.format("bind %s: taken by a PlayerBindPress hook", bind))
		return true
	end
	ply:ConCommand(bind)
	B.stats.run = B.stats.run + 1
	if bind:sub(1, 1) == "+" then B.plus[bind] = true end
	gmsky.Log(string.format("bind %s: run", bind))
	return false
end

local function Release(bind)
	if not B.plus[bind] then return end
	B.plus[bind] = nil
	local ply = LocalPlayer()
	if IsValid(ply) then ply:ConCommand("-" .. bind:sub(2)) end
end

local function ReleaseAll()
	for bind in pairs(B.plus) do Release(bind) end
end

local function Edge(buttons, bit_)
	local now, before = bit.band(buttons, bit_) ~= 0, bit.band(B.prev, bit_) ~= 0
	return now and not before, before and not now, now
end

local function SetCursor(on)
	if on == B.cursor then return end
	B.cursor = on
	if on then
		if not B.x then B.x, B.y = math.floor(ScrW() / 2), math.floor(ScrH() / 2) end
		gui.InternalCursorMoved(B.x, B.y)
		-- A click held into the world before the menu opened ends here.
		Release("+attack")
		Release("+attack2")
	else
		for code in pairs(B.vguiDown) do gui.InternalMouseReleased(code) end
		B.vguiDown = {}
	end
	gmsky.Log(string.format("menu cursor %s at (%d, %d)", on and "ON (Skyrim's mouse drives it)" or "OFF", B.x or -1, B.y or -1))
end

local function MenuMouse(code, pressed, released)
	if pressed then
		gui.InternalMousePressed(code)
		B.vguiDown[code] = true
		B.stats.clicks = B.stats.clicks + 1
	elseif released and B.vguiDown[code] then
		gui.InternalMouseReleased(code)
		B.vguiDown[code] = nil
	end
end

hook.Add("Think", "gmsky_binds", function()
	local active, buttons, dx, dy, dw = gmsky.ReadInput()
	if not active then
		if B.on then
			ReleaseAll()
			SetCursor(false)
			gmsky.SetClientInput(nil)
			gmsky.Log("binds: input from Skyrim OFF, held binds released")
		end
		B.on, B.prev = false, 0
		return
	end
	if not B.on then
		B.on = true
		gmsky.Log("binds: input from Skyrim ON")
	end

	-- Q: Sandbox's spawn menu while held (works with or without a cursor up).
	local qDown, qUp = Edge(buttons, SKY_MENU)
	if qDown then Press("+menu", KEY_Q_) elseif qUp then Release("+menu") end

	SetCursor(vgui.CursorVisible())
	gmsky.SetClientInput(B.cursor)
	local aDown, aUp = Edge(buttons, SKY_ATTACK)
	local bDown, bUp = Edge(buttons, SKY_ATTACK2)
	if B.cursor then
		if dx ~= 0 or dy ~= 0 then
			local k = cursorSpeed:GetFloat()
			B.x = math.Clamp(math.floor(B.x + dx * k + 0.5), 0, ScrW() - 1)
			B.y = math.Clamp(math.floor(B.y + dy * k + 0.5), 0, ScrH() - 1)
			gui.InternalCursorMoved(B.x, B.y)
			B.stats.moves = B.stats.moves + 1
		end
		MenuMouse(MOUSE_LEFT_, aDown, aUp)
		MenuMouse(MOUSE_RIGHT_, bDown, bUp)
		if dw ~= 0 then gui.InternalMouseWheeled(dw) end
	else
		if aDown then Press("+attack", MOUSE_LEFT_) elseif aUp then Release("+attack") end
		if bDown then Press("+attack2", MOUSE_RIGHT_) elseif bUp then Release("+attack2") end
		-- The physics gun holding something keeps the wheel (the server gives it to the gun).
		local wep = LocalPlayer():GetActiveWeapon()
		local physgunHolding = IsValid(wep) and wep:GetClass() == "weapon_physgun" and bit.band(buttons, SKY_ATTACK) ~= 0
		if dw ~= 0 and not physgunHolding then
			-- Source's defaults: wheel up = invprev, wheel down = invnext, one bind per notch.
			for _ = 1, math.min(math.abs(dw), 10) do
				Press(dw > 0 and "invprev" or "invnext", dw > 0 and WHEEL_UP_ or WHEEL_DOWN_)
			end
		end
		local slot = bit.rshift(bit.band(buttons, SLOT_MASK), SLOT_SHIFT)
		local prevSlot = bit.rshift(bit.band(B.prev, SLOT_MASK), SLOT_SHIFT)
		if slot >= 1 and slot <= 6 and slot ~= prevSlot then Press("slot" .. slot, SlotKey(slot)) end
	end
	B.prev = buttons
end)

-- The cursor arrow, over everything else Garry's Mod draws (DrawOverlay comes after the HUD and
-- the menus, while the live overlay still writes alpha).
local ARROW_OUT = { { x = -1, y = -2 }, { x = 14, y = 13 }, { x = -1, y = 20 } }
local ARROW_IN = { { x = 0, y = 0 }, { x = 11, y = 12 }, { x = 0, y = 17 } }
local function Shifted(poly, x, y)
	local out = {}
	for i, p in ipairs(poly) do out[i] = { x = x + p.x, y = y + p.y } end
	return out
end
hook.Add("DrawOverlay", "gmsky_binds_cursor", function()
	if not B.cursor or not B.on then return end
	draw.NoTexture()
	surface.SetDrawColor(0, 0, 0, 255)
	surface.DrawPoly(Shifted(ARROW_OUT, B.x, B.y))
	surface.SetDrawColor(255, 255, 255, 255)
	surface.DrawPoly(Shifted(ARROW_IN, B.x, B.y))
end)

timer.Create("gmsky_binds_status", 1, 0, function()
	local s = B.stats
	if s.run + s.taken + s.clicks + s.moves > 0 then
		gmsky.Log(string.format("binds this second: %d run, %d taken by hooks; menu cursor: %d moves, %d clicks, at (%d, %d)",
			s.run, s.taken, s.moves, s.clicks, B.x or -1, B.y or -1))
	end
	B.stats = { run = 0, taken = 0, clicks = 0, moves = 0 }
end)
