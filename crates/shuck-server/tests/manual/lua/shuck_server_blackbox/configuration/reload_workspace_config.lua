local M = {}

function M.run(t)
  local bufnr, client_id = t.open_fixture("configuration/reload_lint_rules.sh")
  local client = t.client(bufnr, client_id)

  t.wait_for_pull_diagnostic_codes(bufnr, { "C001" })

  local config_path = t.fixture_path("shuck.toml")
  t.write_file(config_path, "[lint]\nselect = [\"C006\"]\n")
  t.notify(client, "workspace/didChangeWatchedFiles", {
    changes = {
      {
        uri = vim.uri_from_fname(config_path),
        type = 2,
      },
    },
  })

  t.wait_for_pull_diagnostic_codes(bufnr, { "C006" })
end

return M
