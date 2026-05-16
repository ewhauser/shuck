local M = {}

function M.run(t)
  local bufnr, client_id = t.open_fixture("formatting/format_request_target.sh")
  local client = t.client(bufnr, client_id)

  local edits = t.client_request_sync(client, "textDocument/formatting", {
    textDocument = { uri = vim.uri_from_bufnr(bufnr) },
    options = {
      tabSize = 8,
      insertSpaces = true,
    },
  }, bufnr, 5000)

  assert(edits ~= nil, "expected formatting request to return an edit list")
  assert(#edits == 1, "expected formatting request to indent the command body")
  assert(vim.deep_equal(edits[1].range.start, { line = 2, character = 0 }))
  assert(vim.deep_equal(edits[1].range["end"], { line = 2, character = 0 }))
  assert(edits[1].newText == "\t", "expected formatting edit to insert one tab")

  t.apply_text_edits(bufnr, client, edits)
  t.wait_for_buffer_lines(bufnr, {
    "#!/bin/sh",
    "if true; then",
    "\techo ok",
    "fi",
  })
end

return M
