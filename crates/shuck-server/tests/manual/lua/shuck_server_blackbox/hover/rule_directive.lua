local M = {}

function M.run(t)
  local bufnr = select(1, t.open_fixture("hover/shellcheck_disable_sc2154.sh"))

  local line = vim.api.nvim_buf_get_lines(bufnr, 1, 2, false)[1]
  assert(line, "expected the hover fixture to have a second line")

  local code_index = assert(string.find(line, "SC2154", 1, true), "failed to find SC2154 in the fixture")
  local hover = t.hover_at(bufnr, 1, code_index - 1)
  local markdown = t.hover_markdown(hover)

  assert(string.find(markdown, "SC2154", 1, true), "expected hover markdown to mention SC2154")
  assert(string.find(markdown, "C006", 1, true), "expected hover markdown to mention C006")
  assert(string.find(markdown, "Undefined Variable", 1, true), "expected hover markdown to mention the rule name")
  assert(string.find(markdown, "No auto-fix", 1, true), "expected hover markdown to mention fix availability")
end

return M
