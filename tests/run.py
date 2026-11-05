# Loads the addon under Lua 5.1 and LuaJIT with game API stubs, produces export strings, writes them for verify.mjs.
import sys, json, lupa.lua51 as L51, lupa.luajit21 as LJ
root = __import__("os").path.dirname(__file__) + "/../addon/"
import os
SECRET = os.environ.get("E2E_SECRET", "11" * 32)
GUID = os.environ.get("E2E_GUID", "Player-4395-0ABCDEF1")
STUBS = r'''
local t0 = E2E_T0
GetServerTime = function() return t0 end
GetTime = function() return 12.5 end
time = GetServerTime
UnitGUID = function() return E2E_GUID end
UnitFullName = function() return "Testchar", "Testrealm" end
GetRealmName = function() return "Testrealm" end
UnitClass = function() return "Mage", "MAGE" end
UnitRace = function() return "Gnome", "Gnome" end
UnitLevel = function() return 60 end
GetAddOnMetadata = function() return "0.1.0" end
printed = {}
DEFAULT_CHAT_FRAME = { AddMessage = function(_, m) printed[#printed+1] = m end }
local frame = {} frame.__index = function() return function() end end
CreateFrame = function() return setmetatable({ RegisterEvent = function() end, SetScript = function(self, k, f) self[k] = f end }, frame) end
C_Timer = { After = function(_, f) end, NewTimer = function() return { Cancel = function() end } end, NewTicker = function() end }
SlashCmdList = {}
UISpecialFrames = {}
tinsert = table.insert
GetItemInfoInstant = function(id) return id, nil, nil, nil, 100000 + id end
'''
def load(lua, shim):
    g = lua.globals()
    ns = lua.table()
    lua.globals().E2E_GUID = GUID
    lua.globals().E2E_T0 = int(os.environ.get("E2E_T0", "1793836800"))
    lua.execute(STUBS)
    if shim: lua.execute(open(__import__("os").path.dirname(__file__) + "/bitshim.lua").read())
    for f in ["Libs/LibDeflate.lua", "Codec.lua", "Core.lua", "Bank.lua", "Recipes.lua"]:
        lua.eval("function(src, name, ns) local f = assert(loadstring(src, name)) return f('Guildaddon', ns) end")(open(root + f).read(), f, ns)
    lua.execute("GuildaddonDB = { chars = { [E2E_GUID] = { secret = '%s' } }, outbox = {}, settings = {} }" % SECRET)
    return ns
out = {}
for label, mk, shim in [("luajit", LJ.LuaRuntime, False), ("lua51", L51.LuaRuntime, True)]:
    lua = mk()
    ns = load(lua, shim)
    h = lua.eval("function(ns) return ns.toHex(ns.sha256('abc')), ns.toHex(ns.hmac(string.rep(string.char(0x0b),20), 'Hi There')) end")(ns)
    tabs = lua.eval('''function(ns) return { { tab = 1, name = "Mats: \\"ore\\"", slots = {
      { link = "|cffffffff|Hitem:2771::::::::60:::::|h[Tin Ore]|h|r", count = 20, texture = 135 },
      { link = "|Hitem:2771::|h", count = 5 }, { link = "|Hitem:13444::|h", count = 1 } } },
      { tab = 2, name = "Épées", slots = {} } } end''')(ns)
    p = lua.eval("function(ns, tabs) return ns.bankPayload(tabs, 1234567, function(id) return 100000 + id end) end")(ns, tabs)
    gb = ns.export("gbank", p)
    rc = ns.export("recipes", lua.eval("function(ns) return { profession = 'Alchemy', skill = 300, recipes = ns.recipeIds({ '|Henchant:17187|h', 'spell:3170', '|Henchant:17187|h' }) } end")(ns))
    rt = lua.eval("function(ns, s) local ty, v, j = ns.decodeString(s) return ty, v, j.p.tabs[1].items[1].count end")(ns, gb)
    out[label] = {"sha": h[0], "hmac": h[1], "gbank": gb, "recipes": rc, "roundtrip": list(rt)}
    print(label, h[0][:16], h[1][:16], len(gb), list(rt))
json.dump({"secret": SECRET, "runs": out}, open("/tmp/addon-out.json", "w"))
for k, r in out.items():
    assert r["sha"] == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", k
    assert r["hmac"] == "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7", k
    assert r["roundtrip"] == ["gbank", 1, 25], k
    assert r["gbank"].startswith("!BF:gbank:v1:") and r["recipes"].startswith("!BF:recipes:v1:"), k
print("all addon tests passed")
