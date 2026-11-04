-- The language server of Isocron in Neovim (D269), run by run.sh in a copy
-- of tools/vscode/test/integration/workspace: the configuration
-- tools/neovim/isocron.lua, then the hover, the definition and the errors
-- of the text as typed (not saved). Each step prints "ok   ..." or
-- "FAIL ..."; a step that fails ends Neovim with the code 1.
vim.g.isocron_lsp = os.getenv('ISC_LSP')
local here = vim.fs.dirname(debug.getinfo(1, 'S').source:sub(2))
dofile(here .. '/../isocron.lua')

local failed = 0
local function step(name, f)
  local ok, detail = pcall(f)
  print((ok and 'ok   ' or 'FAIL ') .. name .. ': ' .. tostring(detail))
  if not ok then
    failed = failed + 1
  end
end

local function wait(what, f)
  if not vim.wait(10000, f, 50) then
    error('timed out waiting for ' .. what)
  end
end

-- the position (from 0) of the first `word` of the line that holds `text`
local function position(text, word)
  for l, line in ipairs(vim.api.nvim_buf_get_lines(0, 0, -1, false)) do
    local at = line:find(text, 1, true)
    if at then
      return { line = l - 1, character = line:find(word, at, true) - 1 }
    end
  end
  error('no line with ' .. text)
end

local function request(method, text, word)
  local params = { textDocument = { uri = vim.uri_from_bufnr(0) }, position = position(text, word) }
  for _, r in pairs(vim.lsp.buf_request_sync(0, method, params, 5000) or {}) do
    if r.result then
      return r.result
    end
  end
  return nil
end

local function hover(text, word)
  local r = request('textDocument/hover', text, word)
  return r and r.contents.value or ''
end

step('a .isc file, the server started for it', function()
  vim.cmd.edit('main.isc')
  if vim.bo.filetype ~= 'isocron' then
    error(vim.bo.filetype)
  end
  wait('the server', function()
    local c = vim.lsp.get_clients({ bufnr = 0, name = 'isc-lsp' })[1]
    return c and c.server_capabilities ~= nil
  end)
  return vim.bo.filetype .. ', isc-lsp attached'
end)

step('no error in a correct file', function()
  vim.wait(1000)
  local n = #vim.diagnostic.get(0)
  if n ~= 0 then
    error(n .. ' errors')
  end
  return 'none'
end)

step('the hover of a function of an imported module: its signature, its comment', function()
  local t = hover('print(area(', 'area')
  if not (t:find('function area(r: f64): f64', 1, true) and t:find('what it exports is imported', 1, true)) then
    error(t)
  end
  return 'function area(r: f64): f64 + its comment'
end)

step('the hover of a parameter: its type', function()
  local t = hover('return 2.0 * x', 'x')
  if not (t:find('x: f64', 1, true) and t:find('parameter', 1, true)) then
    error(t)
  end
  return 'x: f64'
end)

step('the hover of a function of the standard library: its documentation', function()
  local t = hover('print(area(', 'print')
  if not (t:find('`print(x)`', 1, true) and t:find('docs/language.md#', 1, true)) then
    error(t)
  end
  return vim.split(t, '\n')[1]
end)

step('the definition of a name of an imported module', function()
  local r = request('textDocument/definition', 'print(area(', 'area')
  local where = r and (vim.fs.basename(vim.uri_to_fname(r.uri)) .. ':' .. (r.range.start.line + 1)) or 'none'
  if where ~= 'geom.isc:2' then
    error(where)
  end
  return where
end)

step('an error typed, not saved: at its place', function()
  local p = position('return 2.0 * x', 'x')
  vim.api.nvim_buf_set_lines(0, p.line, p.line + 1, false, { '    return 2.0 * y;' })
  wait('the error', function()
    return #vim.diagnostic.get(0) > 0
  end)
  local d = vim.diagnostic.get(0)[1]
  local at = (d.lnum + 1) .. ':' .. (d.col + 1)
  if at ~= '4:18' or not d.message:find('unknown variable `y`', 1, true) then
    error(at .. ' ' .. d.message)
  end
  return at .. ' ' .. d.message
end)

step('the error fixed: gone', function()
  local p = position('return 2.0 * y', 'y')
  vim.api.nvim_buf_set_lines(0, p.line, p.line + 1, false, { '    return 2.0 * x;' })
  wait('no error', function()
    return #vim.diagnostic.get(0) == 0
  end)
  return 'cleared'
end)

for _, c in ipairs(vim.lsp.get_clients()) do
  c.stop()
end
os.exit(failed == 0 and 0 or 1)
