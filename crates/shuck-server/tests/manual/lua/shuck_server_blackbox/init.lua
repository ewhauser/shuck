local M = {}

function M.run()
  local support = require("shuck_server_blackbox.support")
  local case_name = support.getenv("SHUCK_LSP_CASE")
  local module_path = "shuck_server_blackbox." .. case_name:gsub("/", ".")
  local scenario = require(module_path)

  local ok, err = xpcall(function()
    scenario.run(support)
  end, debug.traceback)

  if ok then
    print("PASS " .. case_name)
    vim.cmd("qa!")
    return
  end

  vim.api.nvim_err_writeln(err)
  vim.cmd("cquit 1")
end

return M
