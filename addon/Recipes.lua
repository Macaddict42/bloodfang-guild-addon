-- Crafting recipe export (SPEC §12, contract/SPEC.md `recipes`): when a profession window opens, the
-- spell IDs this character knows for it. Enchanting uses the craft window in Classic clients.
local _, ns = ...
local KNOWN = { Alchemy = true, Blacksmithing = true, Cooking = true, Enchanting = true, Engineering = true, ["First Aid"] = true, Leatherworking = true, Tailoring = true }

-- Pure: links -> sorted unique spell IDs.
function ns.recipeIds(links)
  local seen, ids = {}, ns.array()
  for _, l in ipairs(links) do
    local id = l and tonumber(l:match("enchant:(%d+)") or l:match("spell:(%d+)"))
    if id and not seen[id] then seen[id] = true ids[#ids + 1] = id end
  end
  table.sort(ids)
  return ids
end

local function save(prof, skill, links)
  if not KNOWN[prof] then return end -- names are the English client's; other locales aren't supported yet
  local ids = ns.recipeIds(links)
  if #ids == 0 then return end
  local s = ns.export("recipes", { profession = prof, skill = skill or 0, recipes = ids })
  if s then
    ns.queue("recipes:" .. UnitGUID("player") .. ":" .. prof, "recipes", s)
    ns.print(("%s: %d recipes saved for the site."):format(prof, #ids))
  end
end

local function scanTrade()
  local prof, skill = GetTradeSkillLine()
  if not prof or prof == "UNKNOWN" then return end
  local links = {}
  for i = 1, GetNumTradeSkills() do
    local _, kind = GetTradeSkillInfo(i)
    if kind ~= "header" and kind ~= "subheader" then
      links[#links + 1] = (GetTradeSkillRecipeLink and GetTradeSkillRecipeLink(i)) or GetTradeSkillItemLink(i)
    end
  end
  save(prof, skill, links)
end
local function scanCraft()
  if not GetCraftDisplaySkillLine then return end
  local prof, skill = GetCraftDisplaySkillLine()
  if not prof then return end
  local links = {}
  for i = 1, GetNumCrafts() do
    local _, _, kind = GetCraftInfo(i)
    if kind ~= "header" then
      local l = GetCraftRecipeLink and GetCraftRecipeLink(i)
      if not l and GetCraftSpellID then local id = GetCraftSpellID(i) l = id and ("spell:" .. id) end
      links[#links + 1] = l or GetCraftItemLink(i)
    end
  end
  save(prof, skill, links)
end
-- The list fills in a moment after the window opens.
ns.on("TRADE_SKILL_SHOW", function() C_Timer.After(1, scanTrade) end)
pcall(ns.on, "CRAFT_SHOW", function() C_Timer.After(1, scanCraft) end)
