-- Export string codec (contract/SPEC.md v1): canonical JSON, SHA-256, HMAC-SHA256, LibDeflate print encoding.
-- Plain Lua 5.1 with the game's `bit` library. No game API calls here, so it runs in tests outside the game.
local _, ns = ...
local bit = ns.bit or bit
local band, bor, bxor, bnot, rshift, lshift = bit.band, bit.bor, bit.bxor, bit.bnot, bit.rshift, bit.lshift
local floor, char, byte, format, concat = math.floor, string.char, string.byte, string.format, table.concat
local M32 = 4294967296
local function u(x) return x % M32 end -- the game returns unsigned, LuaJIT signed: normalize both

-- ---------- JSON (sorted keys, integers only; matches the site's canonical()) ----------
local ARRAY = {}
function ns.array(t) return setmetatable(t or {}, ARRAY) end
local esc = { ['"'] = '\\"', ['\\'] = '\\\\', ['\b'] = '\\b', ['\f'] = '\\f', ['\n'] = '\\n', ['\r'] = '\\r', ['\t'] = '\\t' }
local function str(s) return '"' .. s:gsub('[%c"\\]', function(c) return esc[c] or format("\\u%04x", byte(c)) end) .. '"' end
local function isArray(t)
  if getmetatable(t) == ARRAY then return true end
  local n = 0
  for _ in pairs(t) do n = n + 1 end
  return n > 0 and n == #t
end
local function enc(v, out)
  local ty = type(v)
  if ty == "string" then out[#out + 1] = str(v)
  elseif ty == "number" then
    assert(v == floor(v) and v > -2^53 and v < 2^53, "integers only")
    out[#out + 1] = format("%.0f", v)
  elseif ty == "boolean" then out[#out + 1] = tostring(v)
  elseif ty == "table" then
    if isArray(v) then
      out[#out + 1] = "["
      for i = 1, #v do if i > 1 then out[#out + 1] = "," end enc(v[i], out) end
      out[#out + 1] = "]"
    else
      local keys = {}
      for k in pairs(v) do keys[#keys + 1] = tostring(k) end
      table.sort(keys) -- byte order, same as JS sort() for ASCII keys
      out[#out + 1] = "{"
      for i, k in ipairs(keys) do
        if i > 1 then out[#out + 1] = "," end
        out[#out + 1] = str(k) .. ":"
        enc(v[k], out)
      end
      out[#out + 1] = "}"
    end
  else error("cannot encode " .. ty) end
end
function ns.jsonEncode(v) local out = {} enc(v, out) return concat(out) end

-- Small JSON decoder for site -> addon imports.
function ns.jsonDecode(s)
  local i = 1
  local function ws() i = s:find("[^ \t\r\n]", i) or #s + 1 end
  local val
  local function strv()
    local out, j = {}, i + 1
    while true do
      local c = s:sub(j, j)
      if c == "" then error("eof") end
      if c == '"' then i = j + 1 return concat(out) end
      if c == "\\" then
        local e = s:sub(j + 1, j + 1)
        local map = { b = "\b", f = "\f", n = "\n", r = "\r", t = "\t", ['"'] = '"', ["\\"] = "\\", ["/"] = "/" }
        if e == "u" then
          local cp = tonumber(s:sub(j + 2, j + 5), 16)
          if cp < 128 then out[#out + 1] = char(cp)
          elseif cp < 2048 then out[#out + 1] = char(192 + floor(cp / 64), 128 + cp % 64)
          else out[#out + 1] = char(224 + floor(cp / 4096), 128 + floor(cp / 64) % 64, 128 + cp % 64) end
          j = j + 6
        else out[#out + 1] = map[e] or error("bad escape") j = j + 2 end
      else out[#out + 1] = c j = j + 1 end
    end
  end
  function val()
    ws()
    local c = s:sub(i, i)
    if c == "{" then
      local t = {} i = i + 1 ws()
      if s:sub(i, i) == "}" then i = i + 1 return t end
      while true do
        ws() local k = strv() ws()
        if s:sub(i, i) ~= ":" then error("colon") end
        i = i + 1 t[k] = val() ws()
        local d = s:sub(i, i) i = i + 1
        if d == "}" then return t elseif d ~= "," then error("comma") end
      end
    elseif c == "[" then
      local t = ns.array() i = i + 1 ws()
      if s:sub(i, i) == "]" then i = i + 1 return t end
      while true do
        t[#t + 1] = val() ws()
        local d = s:sub(i, i) i = i + 1
        if d == "]" then return t elseif d ~= "," then error("comma") end
      end
    elseif c == '"' then return strv()
    elseif s:sub(i, i + 3) == "true" then i = i + 4 return true
    elseif s:sub(i, i + 4) == "false" then i = i + 5 return false
    elseif s:sub(i, i + 3) == "null" then i = i + 4 return nil
    else
      local n = s:match("^-?%d+%.?%d*[eE]?[-+]?%d*", i)
      if not n or n == "" then error("bad value") end
      i = i + #n return tonumber(n)
    end
  end
  local ok, r = pcall(val)
  if ok then return r end
end

-- ---------- SHA-256 / HMAC ----------
local K = {
  0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,
  0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
  0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,
  0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
  0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,
  0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2 }
local function x3(a, b, c) return bxor(bxor(a, b), c) end -- two-argument calls only, to be safe in every client
local function rotr(x, n) return u(bor(rshift(x, n), lshift(x, 32 - n))) end
function ns.sha256(msg)
  local len = #msg
  msg = msg .. "\128" .. string.rep("\0", (55 - len) % 64)
  local bits = len * 8
  local tail = {}
  for i = 7, 0, -1 do tail[#tail + 1] = char(floor(bits / 2 ^ (8 * i)) % 256) end
  msg = msg .. concat(tail)
  local h = { 0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19 }
  local w = {}
  for blk = 1, #msg, 64 do
    for i = 0, 15 do
      local a, b, c, d = byte(msg, blk + 4 * i, blk + 4 * i + 3)
      w[i] = ((a * 256 + b) * 256 + c) * 256 + d
    end
    for i = 16, 63 do
      local x, y = w[i - 15], w[i - 2]
      local s0 = u(x3(rotr(x, 7), rotr(x, 18), rshift(x, 3)))
      local s1 = u(x3(rotr(y, 17), rotr(y, 19), rshift(y, 10)))
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) % M32
    end
    local a, b, c, d, e, f, g, hh = h[1], h[2], h[3], h[4], h[5], h[6], h[7], h[8]
    for i = 0, 63 do
      local S1 = u(x3(rotr(e, 6), rotr(e, 11), rotr(e, 25)))
      local ch = u(bxor(band(e, f), band(bnot(e), g)))
      local t1 = (hh + S1 + ch + K[i + 1] + w[i]) % M32
      local S0 = u(x3(rotr(a, 2), rotr(a, 13), rotr(a, 22)))
      local maj = u(x3(band(a, b), band(a, c), band(b, c)))
      local t2 = (S0 + maj) % M32
      hh, g, f, e, d, c, b, a = g, f, e, (d + t1) % M32, c, b, a, (t1 + t2) % M32
    end
    h[1], h[2], h[3], h[4] = (h[1] + a) % M32, (h[2] + b) % M32, (h[3] + c) % M32, (h[4] + d) % M32
    h[5], h[6], h[7], h[8] = (h[5] + e) % M32, (h[6] + f) % M32, (h[7] + g) % M32, (h[8] + hh) % M32
  end
  local out = {}
  for i = 1, 8 do local x = h[i] out[i] = char(floor(x / 16777216) % 256, floor(x / 65536) % 256, floor(x / 256) % 256, x % 256) end
  return concat(out)
end
function ns.toHex(s) return (s:gsub(".", function(c) return format("%02x", byte(c)) end)) end
function ns.fromHex(h) return (h:gsub("..", function(x) return char(tonumber(x, 16)) end)) end
function ns.hmac(key, msg)
  if #key > 64 then key = ns.sha256(key) end
  key = key .. string.rep("\0", 64 - #key)
  local ip, op = {}, {}
  for i = 1, 64 do local k = byte(key, i) ip[i] = char(u(bxor(k, 0x36))) op[i] = char(u(bxor(k, 0x5c))) end
  return ns.sha256(concat(op) .. ns.sha256(concat(ip) .. msg))
end

-- ---------- Envelope ----------
local function deflate() return ns.LibDeflate or (LibStub and LibStub("LibDeflate", true)) end
function ns.randomHex(n)
  local t = {}
  for i = 1, n do t[i] = format("%02x", math.random(0, 255)) end
  return concat(t)
end
-- Build a signed `!BF:<type>:v<v>:` string. `secretHex` nil only for `link`.
function ns.encodeString(ty, v, env, secretHex)
  if secretHex then env.sig = ns.toHex(ns.hmac(ns.fromHex(secretHex), ns.jsonEncode(env))) end
  local L = deflate()
  return "!BF:" .. ty .. ":v" .. v .. ":" .. L:EncodeForPrint(L:CompressDeflate(ns.jsonEncode(env), { level = 9 }))
end
function ns.decodeString(s)
  local ty, v, body = (s or ""):gsub("^%s+", ""):gsub("%s+$", ""):match("^!BF:([a-z%-]+):v(%d+):(.+)$")
  if not ty then return nil, "bad_prefix" end
  local L = deflate()
  local bin = L:DecodeForPrint(body)
  local txt = bin and L:DecompressDeflate(bin)
  local j = txt and ns.jsonDecode(txt)
  if type(j) ~= "table" then return nil, "decode_failed" end
  return ty, tonumber(v), j
end
