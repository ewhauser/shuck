local M = {}

function M.run(t)
  local bufnr = select(1, t.open_fixture("diagnostics/unused_assignment.sh"))

  t.wait_for_diagnostic_codes(bufnr, { "C001" })

  local diagnostics = vim.diagnostic.get(bufnr)
  assert(#diagnostics == 1, "expected one diagnostic after opening the fixture")
  assert(diagnostics[1].source == "shuck", "expected the diagnostic source to be shuck")

  t.set_buffer_contents(bufnr, {
    "#!/bin/sh",
    "foo=1",
    "printf '%s\\n' \"$foo\"",
  })

  t.wait_for_no_diagnostics(bufnr)
end

return M
