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
  assert(#edits == 0, "expected formatting request to reflect the current formatter no-op behavior")
  t.wait_for_buffer_lines(bufnr, {
    "#!/bin/sh",
    "if true; then",
    "echo ok",
    "fi",
  })
end

return M
