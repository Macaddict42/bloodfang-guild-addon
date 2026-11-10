-- Core: saved data, slash commands, the copy/paste window, linking and the export outbox.
local ADDON, ns = ...
ns.name = ADDON
ns.version = (C_AddOns and C_AddOns.GetAddOnMetadata or GetAddOnMetadata)(ADDON, "Version") or "0.0.0"
ns.prefix = ADDON:upper():sub(1, 16) -- addon message prefix (export-format.md, Addon messages)
local DBNAME = ADDON .. "DB"

function ns.print(msg) DEFAULT_CHAT_FRAME:AddMessage("|cff4fc3f7" .. ADDON .. "|r " .. msg) end

local function db() return _G[DBNAME] end
ns.db = db
function ns.settings() return db().settings end

-- ---------- character identity ----------
function ns.me()
  local name, realm = UnitFullName("player")
  return { guid = UnitGUID("player"), name = name, realm = (realm and realm ~= "") and realm or GetRealmName() }
end
function ns.secret() local c = db().chars[UnitGUID("player")] return c and c.secret end

-- Envelope per export-format.md. Returns the string, or nil when this character isn't linked.
function ns.export(ty, p)
  local secret = ns.secret()
  if not secret then
    ns.print("This character isn't linked yet. Get a code on the site's Link page, then type /bf link <code>.")
    return nil
  end
  local env = { id = ns.randomHex(16), t = GetServerTime(), addon = ns.version, char = ns.me(), p = p }
  -- Guild rank names (Guild.lua) ride on every other export, inside the signed envelope.
  if ty ~= "ranks" and ns.guildBlock then env.guild = ns.guildBlock() end
  return ns.encodeString(ty, 1, env, secret)
end

-- Outbox: the newest string per key (e.g. "gbank", "recipes:<guid>:Alchemy"). The uploader reads it from
-- SavedVariables after a /reload or logout; members without the uploader copy it with /bf export.
function ns.queue(key, ty, s)
  db().outbox[key] = { type = ty, s = s, t = GetServerTime() }
end

-- ---------- copy / paste window ----------
local box
local function window()
  if box then return box end
  local f = CreateFrame("Frame", ADDON .. "CopyFrame", UIParent, BackdropTemplateMixin and "BackdropTemplate" or nil)
  f:SetSize(520, 220) f:SetPoint("CENTER") f:SetFrameStrata("DIALOG") f:EnableMouse(true) f:SetMovable(true)
  f:RegisterForDrag("LeftButton") f:SetScript("OnDragStart", f.StartMoving) f:SetScript("OnDragStop", f.StopMovingOrSizing)
  if f.SetBackdrop then
    f:SetBackdrop({ bgFile = "Interface\\DialogFrame\\UI-DialogBox-Background", edgeFile = "Interface\\DialogFrame\\UI-DialogBox-Border", tile = true, tileSize = 32, edgeSize = 32, insets = { left = 8, right = 8, top = 8, bottom = 8 } })
  end
  f.title = f:CreateFontString(nil, "OVERLAY", "GameFontNormal") f.title:SetPoint("TOP", 0, -16)
  local sf = CreateFrame("ScrollFrame", nil, f, "UIPanelScrollFrameTemplate")
  sf:SetPoint("TOPLEFT", 20, -40) sf:SetPoint("BOTTOMRIGHT", -40, 50)
  local eb = CreateFrame("EditBox", nil, sf) eb:SetMultiLine(true) eb:SetFontObject(ChatFontNormal) eb:SetWidth(440) eb:SetAutoFocus(true)
  eb:SetScript("OnEscapePressed", function() f:Hide() end)
  sf:SetScrollChild(eb) f.edit = eb
  local ok = CreateFrame("Button", nil, f, "UIPanelButtonTemplate") ok:SetSize(100, 22) ok:SetPoint("BOTTOMRIGHT", -20, 16)
  f.ok = ok
  local close = CreateFrame("Button", nil, f, "UIPanelButtonTemplate") close:SetSize(100, 22) close:SetPoint("RIGHT", ok, "LEFT", -8, 0)
  close:SetText(CLOSE) close:SetScript("OnClick", function() f:Hide() end)
  tinsert(UISpecialFrames, f:GetName())
  box = f
  return f
end
function ns.showCopy(title, text)
  local f = window()
  f.title:SetText(title) f.edit:SetText(text) f.edit:HighlightText() f.edit:SetFocus()
  f.ok:SetText("Select all") f.ok:SetScript("OnClick", function() f.edit:HighlightText() f.edit:SetFocus() end)
  f:Show()
end
function ns.showPaste(title, onDone)
  local f = window()
  f.title:SetText(title) f.edit:SetText("") f.edit:SetFocus()
  f.ok:SetText(OKAY) f.ok:SetScript("OnClick", function() local t = f.edit:GetText() f:Hide() onDone(t) end)
  f:Show()
end

-- ---------- link and import (export-format.md) ----------
local function link(code)
  code = (code or ""):upper():gsub("%s", "")
  if not code:match("^[A-Z0-9]+$") or #code ~= 8 then ns.print("Usage: /bf link <8-character code from the site's Link page>") return end
  local _, classFile = UnitClass("player")
  local _, race = UnitRace("player")
  local env = { id = ns.randomHex(16), t = GetServerTime(), addon = ns.version, char = ns.me(), p = { code = code, class = classFile, race = race, level = UnitLevel("player") } }
  ns.showCopy("Copy this link string and paste it on the site's Link page", ns.encodeString("link", 1, env, nil))
end

-- Site -> addon imports. Owner decision, 2026-11-04: the Ed25519 signature is not checked yet (OPEN-QUESTIONS);
-- the import is only accepted for the character it names.
local function import(text)
  local ty, v, j = ns.decodeString(text)
  if not ty then ns.print("That isn't a site string. Copy the whole string again.") return end
  if v > 1 then ns.print("That string is newer than this addon. Update the addon.") return end
  if type(j.char) ~= "table" or j.char.guid ~= UnitGUID("player") then ns.print("That string is for a different character. Log in to it and import there.") return end
  if ty == "link-ok" then
    local s = type(j.p) == "table" and j.p.secret
    if type(s) ~= "string" or not s:match("^%x+$") or #s ~= 64 then ns.print("That link string is damaged.") return end
    db().chars[j.char.guid] = { secret = s:lower(), linkedAt = GetServerTime() }
    ns.print("Linked. This character can now send signed exports.")
  else
    ns.print("Imports of type " .. ty .. " aren't supported by this version yet.")
  end
end

-- ---------- slash commands ----------
ns.commands = {}
ns.ready = {}
ns.commands.link = link
ns.commands.import = function(arg) if arg ~= "" then import(arg) else ns.showPaste("Paste the string from the site", import) end end
ns.commands.export = function(arg)
  local list = {}
  -- `ranks` strings go up with the uploader only; the site has no paste box for them.
  for key, e in pairs(db().outbox) do if key ~= "ranks" and (arg == "" or key:find(arg, 1, true) == 1) then list[#list + 1] = e.s end end
  if #list == 0 then ns.print("Nothing to export yet. Open the guild bank or a profession window first.") return end
  ns.showCopy("Copy and paste on the site (Bank or Crafting page)", table.concat(list, "\n"))
end
ns.commands.help = function()
  ns.print("/bf link <code> · /bf import · /bf export [gbank|recipes] · /bf ranks · /bf location on|off · /bf where · /bf version")
end
SLASH_GUILDADDONBF1 = "/bf"
SlashCmdList.GUILDADDONBF = function(msg)
  local cmd, arg = (msg or ""):match("^%s*(%S*)%s*(.-)%s*$")
  local fn = ns.commands[(cmd or ""):lower()] or ns.commands.help
  fn(arg or "")
end

-- ---------- startup ----------
local ev = CreateFrame("Frame")
ns.events = {}
function ns.on(event, fn) ns.events[event] = ns.events[event] or {} table.insert(ns.events[event], fn) ev:RegisterEvent(event) end
ev:SetScript("OnEvent", function(_, event, ...) for _, fn in ipairs(ns.events[event]) do fn(...) end end)
ns.on("ADDON_LOADED", function(name)
  if name ~= ADDON then return end
  local d = _G[DBNAME] or {}
  d.chars, d.outbox, d.settings = d.chars or {}, d.outbox or {}, d.settings or {}
  if d.settings.location == nil then d.settings.location = true end -- on by default, opt-out (SPEC §18)
  _G[DBNAME] = d
  pcall(math.randomseed, GetServerTime() + math.floor((GetTime() % 1) * 1e6))
  for _, fn in ipairs(ns.ready or {}) do fn() end
end)
