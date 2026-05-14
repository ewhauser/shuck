local M = {}

local function symbol_names(symbols)
  local names = {}
  for _, symbol in ipairs(symbols or {}) do
    table.insert(names, symbol.name)
  end
  table.sort(names)
  return names
end

local function contains(values, needle)
  for _, value in ipairs(values) do
    if value == needle then
      return true
    end
  end
  return false
end

function M.run(t)
  local bufnr, client_id = t.open_fixture("symbols/workspace_symbols_open.sh")
  local client = t.client(bufnr, client_id)

  assert(
    client.server_capabilities.workspaceSymbolProvider ~= nil,
    "expected workspaceSymbolProvider to be advertised"
  )

  t.set_buffer_contents(bufnr, {
    "buffer_workspace_symbol() {",
    "  :",
    "}",
  })

  local symbols = t.client_request_sync(client, "workspace/symbol", {
    query = "workspace_symbol",
  }, bufnr, 5000)
  local names = symbol_names(symbols)

  assert(
    contains(names, "buffer_workspace_symbol"),
    "expected unsaved open-buffer symbol: " .. vim.inspect(names)
  )
  assert(
    contains(names, "closed_workspace_symbol"),
    "expected closed-file workspace symbol: " .. vim.inspect(names)
  )
  assert(
    not contains(names, "disk_workspace_symbol"),
    "expected open buffer to override disk summary: " .. vim.inspect(names)
  )

  for _, symbol in ipairs(symbols) do
    if symbol.name == "closed_workspace_symbol" then
      assert(symbol.location ~= nil, "expected workspace symbol location")
      assert(symbol.location.uri ~= nil, "expected concrete location URI")
      assert(symbol.location.range ~= nil, "expected concrete location range")
    end
  end
end

return M
