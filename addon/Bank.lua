-- Guild bank capture (SPEC §13, export-format.md `gbank`): every viewable tab's item IDs and counts, each
-- item's icon as the game reports it, and guild bank gold. Runs when the guild bank window opens.
local _, ns = ...
local MAX_TABS = 8
local SLOTS = 98
local pending, scanned, timer

-- Pure part, testable outside the game: raw slots -> payload.
-- tabs = { { tab = n, name = "...", slots = { { link = "|Hitem:123:...|h", count = 5, texture = 1234 }, ... } } }
function ns.bankPayload(tabs, copper, iconFor)
  local out = ns.array()
  for _, t in ipairs(tabs) do
    local items, idx = ns.array(), {}
    for _, s in ipairs(t.slots) do
      local id = s.link and tonumber(s.link:match("item:(%d+)"))
      if id and s.count and s.count > 0 then
        local e = idx[id]
        if e then e.count = e.count + s.count
        else
          e = { itemId = id, count = s.count, icon = (iconFor and iconFor(id)) or s.texture }
          idx[id] = e
          items[#items + 1] = e
        end
      end
    end
    out[#out + 1] = { tab = t.tab, name = t.name, items = items }
  end
  local p = { tabs = out }
  if copper then p.copper = copper end
  return p
end

local function iconFor(id)
  local icon = GetItemInfoInstant and select(5, GetItemInfoInstant(id))
  return icon
end

local function finish()
  local tabs = {}
  for tab = 1, GetNumGuildBankTabs() do
    local name, _, viewable = GetGuildBankTabInfo(tab)
    if viewable then
      local slots = {}
      for slot = 1, SLOTS do
        local texture, count = GetGuildBankItemInfo(tab, slot)
        local link = GetGuildBankItemLink(tab, slot)
        if link then slots[#slots + 1] = { link = link, count = count, texture = texture } end
      end
      tabs[#tabs + 1] = { tab = tab, name = name, slots = slots }
    end
  end
  if #tabs == 0 then return end
  local money = GetGuildBankMoney and GetGuildBankMoney()
  local s = ns.export("gbank", ns.bankPayload(tabs, money, iconFor))
  if s then
    ns.queue("gbank", "gbank", s)
    local n = 0
    for _, t in ipairs(tabs) do n = n + #t.slots end
    ns.print(("Guild bank captured (%d tabs, %d slots). It uploads with the uploader after /reload or logout, or copy it with /bf export gbank."):format(#tabs, n))
  end
end

-- Query each viewable tab once; the server answers with GUILDBANKBAGSLOTS_CHANGED. Capture after a quiet second.
local function settle()
  if timer then timer:Cancel() end
  timer = C_Timer.NewTimer(1.5, function() timer = nil if pending then pending = false finish() end end)
end
ns.on("GUILDBANKFRAME_OPENED", function()
  pending, scanned = true, {}
  for tab = 1, math.min(GetNumGuildBankTabs(), MAX_TABS) do
    local _, _, viewable = GetGuildBankTabInfo(tab)
    if viewable then
      C_Timer.After(0.3 * tab, function() if pending then QueryGuildBankTab(tab) end end)
    end
  end
  settle()
end)
ns.on("GUILDBANKBAGSLOTS_CHANGED", function() if pending then settle() end end)
ns.on("GUILDBANKFRAME_CLOSED", function() if pending then pending = false if timer then timer:Cancel() timer = nil end finish() end end)
