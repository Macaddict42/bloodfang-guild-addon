-- Guild rank names (site Batch 7). Battle.net only gives each member a rank index (0 = Guild Master), so the addon
-- reads the names in game and keeps them in SavedVariables with the time they were read. Every signed export
-- (gbank, recipes) carries them as `guild = { name, ranks, t }`, which the uploader already sends; a `ranks`
-- string is also kept in the outbox for an uploader that knows that type (the site accepts it). Every game call is guarded: an older or different client
-- without these functions simply never captures anything.
local _, ns = ...
local MAX_RANKS, MAX_LEN = 10, 40
local REQUEUE = 3600 -- seconds before an unchanged `ranks` string is re-signed

local function trim(s) return (s:gsub("^%s+", ""):gsub("%s+$", "")) end

-- Pure part, testable outside the game. GuildControlGetRankName is 1-based: 1 = Guild Master, which is
-- Battle.net index 0, so the array goes out in that order. Returns nil when anything is missing.
function ns.rankNames(numRanks, nameOf)
  local n = tonumber(numRanks)
  if not n or n < 1 then return nil end
  if n > MAX_RANKS then n = MAX_RANKS end
  local out = ns.array()
  for i = 1, n do
    local name = nameOf(i)
    if type(name) ~= "string" then return nil end
    name = trim(name)
    if name == "" or #name > MAX_LEN * 4 then return nil end
    out[#out + 1] = name
  end
  return out
end

local function same(a, b)
  if not a or not b or #a ~= #b then return false end
  for i = 1, #a do if a[i] ~= b[i] then return false end end
  return true
end

local function currentGuild()
  if not (IsInGuild and IsInGuild()) or not GetGuildInfo then return nil end
  local ok, name = pcall(GetGuildInfo, "player")
  if ok and type(name) == "string" and name ~= "" then return name end
end

local function readRanks()
  if not (GuildControlGetNumRanks and GuildControlGetRankName) then return nil end
  local ok, n = pcall(GuildControlGetNumRanks)
  if not ok then return nil end
  return ns.rankNames(n, function(i)
    local ok2, v = pcall(GuildControlGetRankName, i)
    return ok2 and v or nil
  end)
end

-- The block that rides on exports: only for the guild this character is in now.
function ns.guildBlock()
  local g = ns.db().guild
  local cur = currentGuild()
  if not g or not cur or g.name ~= cur or type(g.ranks) ~= "table" or #g.ranks == 0 then return nil end
  return { name = g.name, ranks = ns.array({ unpack(g.ranks) }), t = g.t }
end

function ns.captureRanks()
  local guild = currentGuild()
  if not guild then return end
  local names = readRanks()
  if not names then return end
  local d = ns.db()
  local prev = d.guild
  local changed = not (prev and prev.name == guild and same(prev.ranks, names))
  -- Always refresh the time: the site keeps the newest capture, so a stale copy never overwrites a rename.
  d.guild = { name = guild, ranks = names, t = GetServerTime() }
  local q = d.outbox.ranks
  if ns.secret() and (changed or not q or GetServerTime() - (q.t or 0) >= REQUEUE) then
    local b = ns.guildBlock()
    local s = b and ns.export("ranks", b)
    if s then ns.queue("ranks", "ranks", s) end
  end
end

-- Ask the server for the roster once guild data can load; GUILD_ROSTER_UPDATE then carries the rank list.
local function request()
  if not (IsInGuild and IsInGuild()) then return end
  local fn = (C_GuildInfo and C_GuildInfo.GuildRoster) or GuildRoster
  if fn then pcall(fn) end
end
-- A few seconds after login or a /reload, ask for the roster (and read the ranks once more a little later), so the
-- names are captured without anyone opening the guild window. The server throttles roster requests (about one per
-- 10 s), so the two events share one request.
local lastAsk = -1e9
local function soon()
  local now = GetTime and GetTime() or 0
  if now - lastAsk < 10 then return end
  lastAsk = now
  if not C_Timer then request() ns.captureRanks() return end
  C_Timer.After(4, request)
  C_Timer.After(9, ns.captureRanks)
end
ns.soonRanks = soon
ns.on("PLAYER_LOGIN", soon)
ns.on("PLAYER_ENTERING_WORLD", function(initial, reload) if initial or reload or initial == nil then soon() end end)
ns.on("PLAYER_GUILD_UPDATE", function() request() ns.captureRanks() end)
ns.on("GUILD_ROSTER_UPDATE", function() ns.captureRanks() end)
ns.commands.ranks = function()
  local g = ns.db().guild
  if not g then ns.print("No guild rank names read yet. They are read a few seconds after login; open the guild window or /reload while in a guild.") return end
  ns.print(("%s: %s"):format(g.name, table.concat(g.ranks, ", ")))
end
