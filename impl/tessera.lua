-- tessera.lua - Tessera v1 for Lua 5.2 (also runs on 5.3/5.4).
--
-- Storage layout: each Config is a plain table `cfg` where
--   cfg[key] = { i=number, f=number, s=string, b=string, l=boolean,
--                inners=Config }
-- The i/f/s/b/l/inners slots correspond to the type tables from the spec,
-- so a key can hold values of several types at once.
--
-- Access after parse: root.version.i, root.server.inners.port.i, ...
--
-- Lua 5.2 has no integer type; tonumber returns double, so i values
-- beyond 2^53 lose precision. The code also runs on 5.3+ unchanged.

local Tessera = {}

Tessera.E = {
  MissingName  = "MissingName",
  CantPopStack = "CantPopStack",
  MissingParts = "MissingParts",
  InvalidInt   = "InvalidInt",
  InvalidFloat = "InvalidFloat",
  InvalidBool  = "InvalidBool",
  BadEscape    = "BadEscape",
  BadBase64    = "BadBase64",
  UnknownType  = "UnknownType",
}

local function new_config() return {} end

-- Same set as Rust str::trim_ascii(): [ \t\n\r\f]
local function trim_ascii(s)
  return (s:gsub("^[ \t\r\n\f]+", ""):gsub("[ \t\r\n\f]+$", ""))
end

local function utf8_encode(cp)
  if cp < 0x80 then return string.char(cp) end
  if cp < 0x800 then
    return string.char(0xC0 + math.floor(cp / 0x40), 0x80 + cp % 0x40)
  end
  if cp < 0x10000 then
    return string.char(0xE0 + math.floor(cp / 0x1000),
                       0x80 + math.floor(cp / 0x40) % 0x40,
                       0x80 + cp % 0x40)
  end
  return string.char(0xF0 + math.floor(cp / 0x40000),
                     0x80 + math.floor(cp / 0x1000) % 0x40,
                     0x80 + math.floor(cp / 0x40) % 0x40,
                     0x80 + cp % 0x40)
end

local function unescape(s)
  local out, i, n = {}, 1, #s
  while i <= n do
    local c = s:sub(i, i)
    if c ~= "\\" then
      out[#out+1] = c
      i = i + 1
    else
      if i == n then return nil, Tessera.E.BadEscape end
      local e = s:sub(i + 1, i + 1)
      if     e == "\\" then out[#out+1] = "\\"; i = i + 2
      elseif e == "\"" then out[#out+1] = "\""; i = i + 2
      elseif e == "n"  then out[#out+1] = "\n"; i = i + 2
      elseif e == "r"  then out[#out+1] = "\r"; i = i + 2
      elseif e == "t"  then out[#out+1] = "\t"; i = i + 2
      elseif e == "u"  then
        local hex = s:sub(i + 2, i + 5)
        if #hex < 4 or not hex:match("^%x%x%x%x$") then
          return nil, Tessera.E.BadEscape
        end
        local cp = tonumber(hex, 16)
        if cp >= 0xD800 and cp <= 0xDFFF then
          return nil, Tessera.E.BadEscape
        end
        out[#out+1] = utf8_encode(cp)
        i = i + 6
      else
        return nil, Tessera.E.BadEscape
      end
    end
  end
  return table.concat(out)
end

local B64_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
local B64_LOOKUP = {}
for i = 1, 64 do B64_LOOKUP[B64_CHARS:sub(i, i)] = i - 1 end

local function b64decode(s)
  if #s % 4 ~= 0 then return nil end
  if #s == 0 then return "" end
  local out, i = {}, 1
  while i <= #s do
    local c1, c2 = s:sub(i, i), s:sub(i+1, i+1)
    local c3, c4 = s:sub(i+2, i+2), s:sub(i+3, i+3)
    local v1, v2 = B64_LOOKUP[c1], B64_LOOKUP[c2]
    if not v1 or not v2 then return nil end
    local b1 = v1 * 4 + math.floor(v2 / 16)
    if c3 == "=" then
      if c4 ~= "=" then return nil end
      out[#out+1] = string.char(b1)
    elseif c4 == "=" then
      local v3 = B64_LOOKUP[c3]
      if not v3 then return nil end
      out[#out+1] = string.char(b1)
      out[#out+1] = string.char((v2 % 16) * 16 + math.floor(v3 / 4))
    else
      local v3, v4 = B64_LOOKUP[c3], B64_LOOKUP[c4]
      if not v3 or not v4 then return nil end
      out[#out+1] = string.char(b1)
      out[#out+1] = string.char((v2 % 16) * 16 + math.floor(v3 / 4))
      out[#out+1] = string.char((v3 % 4) * 64 + v4)
    end
    i = i + 4
  end
  return table.concat(out)
end

-- Convenience accessors. Optional; the table is directly reachable.
function Tessera.get(cfg, vtype, key)
  local slot = cfg[key]
  return slot and slot[vtype]
end

function Tessera.inner(cfg, key)
  local slot = cfg[key]
  return slot and slot.inners
end

function Tessera.parse(src)
  local errors = {}
  local root   = new_config()
  -- stack[i] = { cfg = <config table> }
  local stack  = { { cfg = root } }
  local lineno = 0

  local function add_error(msg)
    errors[#errors+1] = { line = lineno, error = msg }
  end

  local function parse_field(key, vtype, value)
    local cfg = stack[#stack].cfg
    local n

    if vtype == "i" then
      if value:find(" ", 1, true) or not value:match("^[+-]?%d+$") then
        return add_error(Tessera.E.InvalidInt)
      end
      n = tonumber(value)
      if not n or n ~= n or n == math.huge or n == -math.huge then
        return add_error(Tessera.E.InvalidInt)
      end
      local slot = cfg[key]
      if not slot then slot = {}; cfg[key] = slot end
      slot.i = n
    elseif vtype == "f" then
      -- Rust f64::from_str rejects hex; tonumber("0x10") accepts it.
      if value:find(" ", 1, true) or value:find("[xX]") then
        return add_error(Tessera.E.InvalidFloat)
      end
      n = tonumber(value)
      if not n then return add_error(Tessera.E.InvalidFloat) end
      local slot = cfg[key]
      if not slot then slot = {}; cfg[key] = slot end
      slot.f = n
    elseif vtype == "s" then
      local v = value
      if #v >= 2 and v:sub(1,1) == '"' and v:sub(-1) == '"' then
        v = v:sub(2, -2)
      end
      local str, err = unescape(v)
      if not str then return add_error(err) end
      local slot = cfg[key]
      if not slot then slot = {}; cfg[key] = slot end
      slot.s = str
    elseif vtype == "b" then
      if value:find(" ", 1, true) then
        return add_error(Tessera.E.BadBase64)
      end
      local bytes = b64decode(value)
      if not bytes then return add_error(Tessera.E.BadBase64) end
      local slot = cfg[key]
      if not slot then slot = {}; cfg[key] = slot end
      slot.b = bytes
    elseif vtype == "l" then
      local v = trim_ascii(value)
      local b
      if v == "true" or v == "1" then b = true
      elseif v == "false" or v == "0" then b = false
      else return add_error(Tessera.E.InvalidBool) end
      local slot = cfg[key]
      if not slot then slot = {}; cfg[key] = slot end
      slot.l = b
    else
      add_error(Tessera.E.UnknownType)
    end
  end

  local function process_line(raw)
    local line = trim_ascii(raw)
    if line == "" then return end
    local first = line:sub(1, 1)

    if first == "<" then
      local name = trim_ascii(line:sub(2))
      if name == "" then
        add_error(Tessera.E.MissingName)
      else
        local parent = stack[#stack].cfg
        local slot = parent[name]
        if not slot then slot = {}; parent[name] = slot end
        local child = new_config()
        slot.inners = child
        stack[#stack+1] = { cfg = child }
      end
      return
    end

    if line == ">" then
      if #stack <= 1 then add_error(Tessera.E.CantPopStack)
      else stack[#stack] = nil end
      return
    end

    if first == "#" then return end

    local p1 = line:find(" ", 1, true)
    if not p1 then return add_error(Tessera.E.MissingParts) end
    local p2 = line:find(" ", p1 + 1, true)
    if not p2 then return add_error(Tessera.E.MissingParts) end

    parse_field(
      line:sub(1, p1 - 1),
      line:sub(p1 + 1, p2 - 1),
      line:sub(p2 + 1))
  end

  -- Matches str::lines(): split on \n; \r\n handled by trim_ascii.
  local pos, n = 1, #src
  while pos <= n do
    local nl = src:find("\n", pos, true)
    local line
    if nl then
      line = src:sub(pos, nl - 1)
      pos  = nl + 1
    else
      line = src:sub(pos)
      pos  = n + 1
    end
    lineno = lineno + 1
    process_line(line)
  end

  return root, errors
end

return Tessera
