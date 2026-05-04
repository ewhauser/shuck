local M = {}

function M.run(t)
  local bufnr, client_id = t.open_fixture("code_actions/fix_all_unused_assignments.sh", {
    init_options = {
      shuck = {
        unsafeFixes = true,
      },
    },
  })
  local client = t.client(bufnr, client_id)

  t.wait_for_diagnostic_codes(bufnr, { "C001", "C001" })

  local diagnostics = t.pull_diagnostics(bufnr)
  local actions = t.code_actions(bufnr, diagnostics, {
    only = { "source.fixAll" },
    range = t.document_range(bufnr),
  })

  local fix_all = t.find_action(actions, function(action)
    return action.kind == "source.fixAll.shuck"
  end, "source.fixAll.shuck action")
  local resolved = t.resolve_code_action(bufnr, client, fix_all)

  assert(resolved.edit ~= nil, "expected fix-all resolution to produce an edit")

  t.apply_workspace_edit(client, resolved.edit)
  t.wait_for_no_diagnostics(bufnr)
  t.wait_for_buffer_lines(bufnr, {
    "_=1",
    "_=2",
  })
end

return M
