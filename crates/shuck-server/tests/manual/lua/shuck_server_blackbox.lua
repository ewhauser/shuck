local M = {}

local CLIENT_NAME = "shuck-server-blackbox"

local function getenv(name)
  local value = os.getenv(name)
  assert(value and value ~= "", "missing environment variable: " .. name)
  return value
end

local function active_clients(bufnr)
  if vim.lsp.get_clients then
    return vim.lsp.get_clients({ bufnr = bufnr, name = CLIENT_NAME })
  end

  local matches = {}
  for _, client in ipairs(vim.lsp.get_active_clients()) do
    if client.name == CLIENT_NAME then
      table.insert(matches, client)
    end
  end
  return matches
end

local function wait_for(label, predicate, timeout_ms)
  local ok = vim.wait(timeout_ms or 5000, predicate, 50)
  assert(ok, "timed out waiting for " .. label)
end

local function find_project_root(path)
  local markers = { ".git", ".shuck.toml", "shuck.toml" }
  local current = vim.fs.dirname(path)
  while current and current ~= "" do
    for _, marker in ipairs(markers) do
      local candidate = current .. "/" .. marker
      if vim.loop.fs_stat(candidate) then
        return current
      end
    end

    local parent = vim.fs.dirname(current)
    if not parent or parent == current then
      break
    end
    current = parent
  end

  error("failed to find a project root for " .. path)
end

local function start_client(bufnr, target_path)
  local project_root = find_project_root(target_path)
  local server_binary = getenv("SHUCK_LSP_SERVER_BINARY")
  local log_file = project_root .. "/shuck-server.log"
  local client_id = vim.lsp.start_client({
    name = CLIENT_NAME,
    cmd = { server_binary, "server" },
    root_dir = project_root,
    workspace_folders = {
      {
        uri = vim.uri_from_fname(project_root),
        name = project_root,
      },
    },
    init_options = {
      shuck = {
        tracing = {
          logLevel = "debug",
          logFile = log_file,
        },
      },
    },
  })

  assert(client_id, "failed to start the Neovim LSP client")
  local attached = vim.lsp.buf_attach_client(bufnr, client_id)
  assert(attached, "failed to attach the Neovim LSP client to the buffer")

  wait_for("LSP client attach", function()
    return #active_clients(bufnr) > 0
  end, 5000)

  return client_id
end

local function diagnostic_codes(bufnr)
  local diagnostics = vim.diagnostic.get(bufnr)
  local codes = {}
  for _, diagnostic in ipairs(diagnostics) do
    if type(diagnostic.code) == "string" then
      table.insert(codes, diagnostic.code)
    elseif type(diagnostic.code) == "number" then
      table.insert(codes, tostring(diagnostic.code))
    end
  end
  table.sort(codes)
  return diagnostics, codes
end

local function wait_for_diagnostic_codes(bufnr, expected_codes)
  local expected = table.concat(expected_codes, ",")
  wait_for("diagnostics " .. expected, function()
    local diagnostics, codes = diagnostic_codes(bufnr)
    if #diagnostics ~= #expected_codes then
      return false
    end
    return table.concat(codes, ",") == expected
  end, 5000)
end

local function wait_for_no_diagnostics(bufnr)
  wait_for("diagnostic clear", function()
    return #vim.diagnostic.get(bufnr) == 0
  end, 5000)
end

local function set_buffer_contents(bufnr, lines)
  vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, lines)
end

local function hover_at(bufnr, line, character)
  local result = vim.lsp.buf_request_sync(bufnr, "textDocument/hover", {
    textDocument = { uri = vim.uri_from_bufnr(bufnr) },
    position = { line = line, character = character },
  }, 5000)
  assert(result ~= nil, "hover request timed out")

  for _, response in pairs(result) do
    if response.result then
      return response.result
    end
  end

  error("hover response was empty")
end

local function hover_markdown(result)
  local contents = result.contents
  if type(contents) == "string" then
    return contents
  end
  if contents.kind and contents.value then
    return contents.value
  end
  if vim.islist(contents) then
    local chunks = {}
    for _, item in ipairs(contents) do
      if type(item) == "string" then
        table.insert(chunks, item)
      elseif item.value then
        table.insert(chunks, item.value)
      end
    end
    return table.concat(chunks, "\n")
  end
  error("unrecognized hover payload")
end

local function open_fixture()
  local target_path = getenv("SHUCK_LSP_TARGET_FILE")
  vim.cmd.edit(vim.fn.fnameescape(target_path))
  local bufnr = vim.api.nvim_get_current_buf()
  local client_id = start_client(bufnr, target_path)
  return bufnr, client_id, target_path
end

local scenarios = {}

function scenarios.diagnostics()
  local bufnr = select(1, open_fixture())

  wait_for_diagnostic_codes(bufnr, { "C001" })

  local diagnostics = vim.diagnostic.get(bufnr)
  assert(#diagnostics == 1, "expected one diagnostic after opening the bad file")
  assert(diagnostics[1].source == "shuck", "expected the diagnostic source to be shuck")

  set_buffer_contents(bufnr, {
    "#!/bin/sh",
    "foo=1",
    "printf '%s\\n' \"$foo\"",
  })

  wait_for_no_diagnostics(bufnr)
end

function scenarios.hover()
  local bufnr = select(1, open_fixture())

  local line = vim.api.nvim_buf_get_lines(bufnr, 1, 2, false)[1]
  assert(line, "expected the hover fixture to have a second line")
  local code_index = assert(string.find(line, "SC2154", 1, true), "failed to find SC2154 in the fixture")
  local hover = hover_at(bufnr, 1, code_index - 1)
  local markdown = hover_markdown(hover)

  assert(string.find(markdown, "SC2154", 1, true), "expected hover markdown to mention SC2154")
  assert(string.find(markdown, "C006", 1, true), "expected hover markdown to mention C006")
  assert(
    string.find(markdown, "Undefined Variable", 1, true),
    "expected hover markdown to mention the rule name"
  )
end

function M.run()
  local case_name = getenv("SHUCK_LSP_CASE")
  local scenario = assert(scenarios[case_name], "unknown scenario: " .. case_name)

  local ok, err = xpcall(scenario, debug.traceback)
  if ok then
    print("PASS " .. case_name)
    vim.cmd("qa!")
    return
  end

  vim.api.nvim_err_writeln(err)
  vim.cmd("cquit 1")
end

return M
