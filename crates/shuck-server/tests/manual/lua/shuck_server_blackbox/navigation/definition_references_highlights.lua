local M = {}

local READ = 2
local WRITE = 3

local function sorted_start_lines(locations)
  local lines = {}
  for _, location in ipairs(locations or {}) do
    table.insert(lines, location.range.start.line)
  end
  table.sort(lines)
  return lines
end

local function highlight_kinds(highlights)
  local kinds = {}
  for _, highlight in ipairs(highlights or {}) do
    table.insert(kinds, highlight.kind)
  end
  table.sort(kinds)
  return kinds
end

function M.run(t)
  local bufnr, client_id = t.open_fixture("navigation/definition_references_highlights.sh")
  local client = t.client(bufnr, client_id)

  assert(client.server_capabilities.definitionProvider == true, "expected definitionProvider")
  assert(client.server_capabilities.referencesProvider == true, "expected referencesProvider")
  assert(
    client.server_capabilities.documentHighlightProvider == true,
    "expected documentHighlightProvider"
  )

  local function_call = t.position_for_nth(bufnr, "greet", 1)
  local definition = t.buffer_request_sync(bufnr, "textDocument/definition", {
    textDocument = t.document_identifier(bufnr),
    position = function_call,
  }, 5000)

  assert(definition ~= nil, "expected function definition location")
  assert(definition.range.start.line == 3, "expected function definition line: " .. vim.inspect(definition))
  assert(definition.range.start.character == 0, "expected function definition column")

  local variable_reference = t.position_for_nth(bufnr, "name", 1)
  local references_without_declaration = t.buffer_request_sync(bufnr, "textDocument/references", {
    textDocument = t.document_identifier(bufnr),
    position = variable_reference,
    context = { includeDeclaration = false },
  }, 5000)
  assert(
    vim.deep_equal(sorted_start_lines(references_without_declaration), { 4 }),
    "expected only read reference: " .. vim.inspect(references_without_declaration)
  )

  local references_with_declaration = t.buffer_request_sync(bufnr, "textDocument/references", {
    textDocument = t.document_identifier(bufnr),
    position = variable_reference,
    context = { includeDeclaration = true },
  }, 5000)
  assert(
    vim.deep_equal(sorted_start_lines(references_with_declaration), { 1, 4 }),
    "expected declaration plus read reference: " .. vim.inspect(references_with_declaration)
  )

  local highlights = t.buffer_request_sync(bufnr, "textDocument/documentHighlight", {
    textDocument = t.document_identifier(bufnr),
    position = variable_reference,
  }, 5000)
  assert(
    vim.deep_equal(highlight_kinds(highlights), { READ, WRITE }),
    "expected read and write highlights: " .. vim.inspect(highlights)
  )
end

return M
