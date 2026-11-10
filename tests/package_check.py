"""Checks a built addon zip: TOC icon line and file, generated Config.lua, and that loading the packaged Lua registers
exactly the configured slash commands (short + addon name) and never /bf."""
import io, re, sys, zipfile
import lupa.lua51 as L51
from PIL import Image

zpath, name, short, long_ = sys.argv[1:5]
z = zipfile.ZipFile(zpath)
names = z.namelist()
toc = z.read(f"{name}/{name}.toc").decode()

icon = [l for l in toc.splitlines() if l.startswith("## IconTexture:")]
assert icon == [f"## IconTexture: Interface\\AddOns\\{name}\\Icon"], icon
assert f"{name}/Icon.tga" in names, names
im = Image.open(io.BytesIO(z.read(f"{name}/Icon.tga")))
assert im.format == "TGA" and im.size == (64, 64) and im.mode == "RGBA", (im.format, im.size, im.mode)

files = [l.strip().replace("\\", "/") for l in toc.splitlines() if l.strip() and not l.startswith("##")]
assert files[0] == "Config.lua", files
for f in files: assert f"{name}/{f}" in names, f

lua_src = {f: z.read(f"{name}/{f}").decode("utf-8") for f in files}
for f, src in lua_src.items():
    if f.startswith("Libs/"): continue
    assert not re.search(r"/bf\b", src), f"/bf left in {f}"

lua = L51.LuaRuntime()
lua.execute(open(__file__.rsplit("/", 1)[0] + "/bitshim.lua").read())
lua.execute(r'''
SlashCmdList = {} printed = {}
DEFAULT_CHAT_FRAME = { AddMessage = function(_, m) printed[#printed+1] = m end }
local frame = {} frame.__index = function() return function() end end
CreateFrame = function() return setmetatable({ RegisterEvent = function() end, SetScript = function() end }, frame) end
GetAddOnMetadata = function() return "9.9.9" end
GetServerTime = function() return 0 end GetTime = function() return 0 end time = GetServerTime
C_Timer = { After = function() end, NewTicker = function() end, NewTimer = function() return {} end }
UISpecialFrames = {} tinsert = table.insert
''')
ns = lua.table()
run = lua.eval("function(src, f, name, ns) return assert(loadstring(src, f))(name, ns) end")
for f in files: run(lua_src[f], f, name, ns)
g = lua.globals()
key = re.sub(r"[^A-Z0-9]", "", name.upper())
slashes = {k: g[k] for k in lua.eval("function() local t = {} for k in pairs(_G) do if type(k) == 'string' and k:match('^SLASH_') then t[#t+1] = k end end return t end")().values()}
assert slashes == {f"SLASH_{key}1": short, f"SLASH_{key}2": long_}, slashes
assert lua.eval("function(k) return type(SlashCmdList[k]) == 'function' end")(key)
lua.eval("function(k) SlashCmdList[k]('help') end")(key)
helptext = lua.eval("function() return printed[#printed] end")()
assert f"{short} link <code>" in helptext and long_ in helptext and "/bf " not in helptext, helptext
print(f"package check: {zpath} registers {short} and {long_} (not /bf), Icon.tga 64x64 RGBA, TOC icon line OK")
