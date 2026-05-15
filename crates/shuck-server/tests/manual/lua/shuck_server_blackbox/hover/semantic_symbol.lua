local M = {}

function M.run(t)
  local bufnr = select(1, t.open_fixture("hover/semantic_symbol.sh"))

  local line = vim.api.nvim_buf_get_lines(bufnr, 4, 5, false)[1]
  assert(line, "expected the hover fixture to have a variable reference")

  local name_index = assert(string.find(line, "name", 1, true), "failed to find name reference")
  local hover = t.hover_at(bufnr, 4, name_index - 1)
  local markdown = t.hover_markdown(hover)

  assert(string.find(markdown, "`name`", 1, true), "expected hover markdown to name the symbol")
  assert(string.find(markdown, "Variable", 1, true), "expected variable hover")
  assert(string.find(markdown, "Defined at line 3", 1, true), "expected definition location")
end

return M
