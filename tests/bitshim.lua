-- Plain Lua 5.1 has no bit library; this pure-Lua one returns unsigned results, as Codec normalises anyway.
local function op(a, b, f)
  a, b = a % 2^32, b % 2^32
  local r, m = 0, 1
  for _ = 1, 32 do
    local x, y = a % 2, b % 2
    if f(x, y) then r = r + m end
    a, b, m = (a - x) / 2, (b - y) / 2, m * 2
  end
  return r
end
bit = {
  band = function(a, b) return op(a, b, function(x, y) return x == 1 and y == 1 end) end,
  bor = function(a, b) return op(a, b, function(x, y) return x == 1 or y == 1 end) end,
  bxor = function(a, b) return op(a, b, function(x, y) return x ~= y end) end,
  bnot = function(a) return (2^32 - 1) - a % 2^32 end,
  lshift = function(a, n) return (a * 2^n) % 2^32 end,
  rshift = function(a, n) return math.floor((a % 2^32) / 2^n) end,
}
