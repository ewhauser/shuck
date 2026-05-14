local M = {}

local function child_names(symbol)
  local names = {}
  for _, child in ipairs(symbol.children or {}) do
    table.insert(names, child.name)
  end
  return names
end

function M.run(t)
  local bufnr, client_id = t.open_fixture("symbols/document_symbols.sh")
  local client = t.client(bufnr, client_id)

  assert(
    client.server_capabilities.documentSymbolProvider ~= nil,
    "expected documentSymbolProvider to be advertised"
  )

  local symbols = t.client_request_sync(client, "textDocument/documentSymbol", {
    textDocument = { uri = vim.uri_from_bufnr(bufnr) },
  }, bufnr, 5000)

  assert(symbols ~= nil, "expected document symbols response")
  assert(#symbols == 3, "expected three top-level symbols: " .. vim.inspect(symbols))
  assert(symbols[1].name == "VERSION", "expected VERSION as first symbol")
  assert(symbols[2].name == "LOOKUP", "expected LOOKUP as second symbol")
  assert(symbols[3].name == "build", "expected build as third symbol")

  local build = symbols[3]
  local build_children = child_names(build)
  assert(vim.deep_equal(build_children, { "artifact", "item", "helper" }), vim.inspect(build))

  local helper = build.children[3]
  assert(
    vim.deep_equal(child_names(helper), { "nested" }),
    "expected helper to contain nested local: " .. vim.inspect(helper)
  )
end

return M
