-- In-game addon messages (export-format.md, Addon messages): VER/VREQ version checks and LOC location sharing.
local _, ns = ...
local CT = C_ChatInfo
local lastVer, vreqSent, warned, replyPending = 0, false, false, false
local higher = {} -- version -> { [sender] = true }
ns.roster = {}    -- sender -> { zone, sub, x, y, t } for /bf where

local function send(msg) if IsInGuild() then CT.SendAddonMessage(ns.prefix, msg, "GUILD") end end
local function parse(v) local a, b, c = (v or ""):match("^(%d+)%.(%d+)%.(%d+)$") if a then return tonumber(a), tonumber(b), tonumber(c) end end

-- Pure: is `theirs` a plausible version newer than `ours`?
function ns.plausiblyNewer(ours, theirs)
  local A, B, C = parse(ours) local a, b, c = parse(theirs)
  if not a or not A then return false end
  if a > A + 1 then return false end
  if a == A and b > B + 20 then return false end
  if a ~= A then return a > A end
  if b ~= B then return b > B end
  return c > C
end

local function sendVer()
  if GetTime() - lastVer < 60 then return end
  lastVer = GetTime() send("VER:" .. ns.version)
end

local function onVer(sender, v)
  if not ns.plausiblyNewer(ns.version, v) then return end
  higher[v] = higher[v] or {}
  higher[v][sender] = true
  local n = 0 for _ in pairs(higher[v]) do n = n + 1 end
  if n >= 2 and not warned then
    warned = true
    ns.print(("A newer version (%s) is out. The uploader can update it, or download it from the site's Addons page."):format(v))
  end
end

-- Location: zone and rounded map position, every 60 s, outside instances only. In game only; nothing is sent to the site.
local function locMsg()
  if not ns.settings().location or IsInInstance() then return nil end
  local zone, sub = GetRealZoneText() or "", GetSubZoneText() or ""
  local x, y = 0, 0
  local map = C_Map and C_Map.GetBestMapForUnit("player")
  local pos = map and C_Map.GetPlayerMapPosition(map, "player")
  if pos then x, y = math.floor(pos.x * 1000 + 0.5), math.floor(pos.y * 1000 + 0.5) end
  return ("LOC:%s:%s:%d:%d"):format(zone:gsub(":", ""), sub:gsub(":", ""), x, y)
end

ns.on("CHAT_MSG_ADDON", function(prefix, msg, channel, sender)
  if prefix ~= ns.prefix or channel ~= "GUILD" then return end
  local me = ns.me() if sender == me.name .. "-" .. me.realm or sender == me.name then return end
  local cmd, rest = msg:match("^(%u+):?(.*)$")
  if cmd == "VER" then onVer(sender, rest)
  elseif cmd == "VREQ" and not replyPending then
    replyPending = true
    C_Timer.After(2 + math.random() * 8, function() replyPending = false sendVer() end)
  elseif cmd == "LOC" then
    local zone, sub, x, y = rest:match("^([^:]*):([^:]*):(%d+):(%d+)$")
    if zone then ns.roster[sender] = { zone = zone, sub = sub, x = tonumber(x), y = tonumber(y), t = time() } end
  end
end)


ns.on("PLAYER_LOGIN", function()
  CT.RegisterAddonMessagePrefix(ns.prefix)
  C_Timer.After(5 + math.random() * 10, function()
    sendVer()
    if not vreqSent then vreqSent = true send("VREQ") end
  end)
  C_Timer.NewTicker(60, function() local m = locMsg() if m then send(m) end end)
end)

ns.commands.location = function(arg)
  arg = arg:lower()
  if arg == "on" or arg == "off" then ns.settings().location = (arg == "on") end
  ns.print("Location sharing with guildmates is " .. (ns.settings().location and "on" or "off") .. ". It stays in game. /bf location on|off")
end
ns.commands.where = function()
  local n = 0
  for who, l in pairs(ns.roster) do
    if time() - l.t < 180 then
      n = n + 1
      ns.print(("%s: %s%s (%.1f, %.1f)"):format(who, l.zone, l.sub ~= "" and (" - " .. l.sub) or "", l.x / 10, l.y / 10))
    end
  end
  if n == 0 then ns.print("No guildmates have shared a location in the last 3 minutes.") end
end
ns.commands.version = function() ns.print("Version " .. ns.version) end
