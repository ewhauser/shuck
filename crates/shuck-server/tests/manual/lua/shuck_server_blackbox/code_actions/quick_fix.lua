local M = {}

function M.run(t)
  local bufnr, client_id = t.open_fixture("code_actions/quick_fix_unused_assignment.sh", {
    init_options = {
      shuck = {
        unsafeFixes = true,
      },
    },
  })
  local client = t.client(bufnr, client_id)

  t.wait_for_diagnostic_codes(bufnr, { "C001" })

  local diagnostics = t.pull_diagnostics(bufnr)
  assert(#diagnostics == 1, "expected one pull diagnostic for the quick-fix fixture")

  local actions = t.code_actions(bufnr, diagnostics, {
    only = { "quickfix" },
  })
  assert(#actions >= 2, "expected quick-fix and disable actions")

  local quick_fix = t.find_action(actions, function(action)
    return action.kind == "quickfix" and action.isPreferred == true and action.edit ~= nil
  end, "preferred quick-fix action")
  t.find_action(actions, function(action)
    return action.kind == "quickfix"
      and action.title ~= nil
      and string.find(action.title, "Disable for this line", 1, true) ~= nil
  end, "disable-for-this-line action")

  t.apply_workspace_edit(client, quick_fix.edit)
  t.wait_for_no_diagnostics(bufnr)
  t.wait_for_buffer_lines(bufnr, { "_=1" })
end

return M
