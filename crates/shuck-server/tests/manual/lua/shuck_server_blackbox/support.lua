local M = {}

local CLIENT_NAME = "shuck-server-blackbox"

function M.getenv(name)
  local value = os.getenv(name)
  assert(value and value ~= "", "missing environment variable: " .. name)
  return value
end

local function unwrap_nil(value)
  if value == vim.NIL then
    return nil
  end
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

function M.wait_for(label, predicate, timeout_ms)
  local ok = vim.wait(timeout_ms or 5000, predicate, 50)
  assert(ok, "timed out waiting for " .. label)
end

function M.workspace_root()
  return M.getenv("SHUCK_LSP_WORKSPACE_ROOT")
end

function M.fixture_path(relative_path)
  return M.workspace_root() .. "/" .. relative_path
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

local function start_client(bufnr, target_path, init_options)
  local project_root = find_project_root(target_path)
  local server_binary = M.getenv("SHUCK_LSP_SERVER_BINARY")
  local log_file = project_root .. "/shuck-server.log"
  local base_init_options = {
    shuck = {
      tracing = {
        logLevel = "debug",
        logFile = log_file,
      },
    },
  }
  local resolved_init_options = init_options
      and vim.tbl_deep_extend("force", base_init_options, init_options)
    or base_init_options

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
    init_options = resolved_init_options,
  })

  assert(client_id, "failed to start the Neovim LSP client")
  local attached = vim.lsp.buf_attach_client(bufnr, client_id)
  assert(attached, "failed to attach the Neovim LSP client to the buffer")

  M.wait_for("LSP client attach", function()
    return #active_clients(bufnr) > 0
  end, 5000)

  return client_id
end

function M.client(bufnr, client_id)
  if client_id ~= nil then
    local client = vim.lsp.get_client_by_id(client_id)
    assert(client, "failed to resolve LSP client " .. tostring(client_id))
    return client
  end

  local clients = active_clients(bufnr)
  assert(#clients == 1, "expected exactly one active shuck client")
  return clients[1]
end

function M.open_fixture(relative_path, opts)
  local target_path = M.fixture_path(relative_path)
  vim.cmd.edit(vim.fn.fnameescape(target_path))
  local bufnr = vim.api.nvim_get_current_buf()
  local client_id = start_client(bufnr, target_path, opts and opts.init_options or nil)
  return bufnr, client_id, target_path
end

function M.buffer_lines(bufnr)
  return vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
end

function M.buffer_text(bufnr)
  return table.concat(M.buffer_lines(bufnr), "\n")
end

function M.set_buffer_contents(bufnr, lines)
  vim.api.nvim_buf_set_lines(bufnr, 0, -1, false, lines)
end

function M.wait_for_buffer_lines(bufnr, expected_lines)
  M.wait_for("buffer contents", function()
    return vim.deep_equal(M.buffer_lines(bufnr), expected_lines)
  end, 5000)
end

local function sorted_codes(diagnostics)
  local codes = {}
  for _, diagnostic in ipairs(diagnostics) do
    local code = unwrap_nil(diagnostic.code)
    if type(code) == "string" then
      table.insert(codes, code)
    elseif type(code) == "number" then
      table.insert(codes, tostring(code))
    end
  end
  table.sort(codes)
  return codes
end

function M.wait_for_diagnostic_codes(bufnr, expected_codes)
  local expected = table.concat(expected_codes, ",")
  M.wait_for("diagnostics " .. expected, function()
    local diagnostics = vim.diagnostic.get(bufnr)
    if #diagnostics ~= #expected_codes then
      return false
    end
    return table.concat(sorted_codes(diagnostics), ",") == expected
  end, 5000)
end

function M.wait_for_no_diagnostics(bufnr)
  M.wait_for("diagnostic clear", function()
    return #vim.diagnostic.get(bufnr) == 0
  end, 5000)
end

local function extract_sync_result(method, responses)
  assert(responses ~= nil, method .. " request timed out")

  for _, response in pairs(responses) do
    local error_value = unwrap_nil(response.err or response.error)
    assert(error_value == nil, method .. " request failed: " .. vim.inspect(error_value))
    return unwrap_nil(response.result)
  end

  error("no response received for " .. method)
end

function M.buffer_request_sync(bufnr, method, params, timeout_ms)
  return extract_sync_result(method, vim.lsp.buf_request_sync(bufnr, method, params, timeout_ms or 5000))
end

function M.client_request_sync(client, method, params, bufnr, timeout_ms)
  local done = false
  local err_value
  local result
  local request_id = client.request(method, params, function(err, response_result)
    err_value = unwrap_nil(err)
    result = unwrap_nil(response_result)
    done = true
  end, bufnr)
  assert(request_id, "failed to send " .. method .. " request")

  M.wait_for(method .. " response", function()
    return done
  end, timeout_ms or 5000)

  assert(err_value == nil, method .. " request failed: " .. vim.inspect(err_value))
  return result
end

local function document_identifier(bufnr)
  return { uri = vim.uri_from_bufnr(bufnr) }
end

function M.document_range(bufnr)
  local line_count = vim.api.nvim_buf_line_count(bufnr)
  local last_line = vim.api.nvim_buf_get_lines(bufnr, line_count - 1, line_count, false)[1] or ""
  return {
    start = { line = 0, character = 0 },
    ["end"] = {
      line = math.max(line_count - 1, 0),
      character = #last_line,
    },
  }
end

function M.pull_diagnostics(bufnr)
  local report = M.buffer_request_sync(bufnr, "textDocument/diagnostic", {
    textDocument = document_identifier(bufnr),
  }, 5000)

  if report == nil then
    return {}
  end
  if report.items then
    return report.items
  end
  if report.fullDocumentDiagnosticReport and report.fullDocumentDiagnosticReport.items then
    return report.fullDocumentDiagnosticReport.items
  end

  error("unrecognized diagnostic report: " .. vim.inspect(report))
end

function M.wait_for_pull_diagnostic_codes(bufnr, expected_codes)
  local expected = table.concat(expected_codes, ",")
  M.wait_for("pull diagnostics " .. expected, function()
    local diagnostics = M.pull_diagnostics(bufnr)
    if #diagnostics ~= #expected_codes then
      return false
    end
    return table.concat(sorted_codes(diagnostics), ",") == expected
  end, 5000)
end

function M.code_actions(bufnr, diagnostics, opts)
  opts = opts or {}
  return M.buffer_request_sync(bufnr, "textDocument/codeAction", {
    textDocument = document_identifier(bufnr),
    range = opts.range or diagnostics[1].range or M.document_range(bufnr),
    context = {
      diagnostics = diagnostics,
      only = opts.only,
      triggerKind = opts.trigger_kind,
    },
  }, 5000) or {}
end

function M.find_action(actions, predicate, description)
  for _, action in ipairs(actions) do
    if predicate(action) then
      return action
    end
  end

  error("failed to find " .. description .. " in " .. vim.inspect(actions))
end

function M.resolve_code_action(bufnr, client, action)
  if action.edit ~= nil then
    return action
  end

  return M.client_request_sync(client, "codeAction/resolve", action, bufnr, 5000)
end

function M.apply_workspace_edit(client, edit)
  vim.lsp.util.apply_workspace_edit(edit, client.offset_encoding or "utf-16")
end

function M.apply_text_edits(bufnr, client, edits)
  vim.lsp.util.apply_text_edits(edits, bufnr, client.offset_encoding or "utf-16")
end

function M.hover_at(bufnr, line, character)
  local result = M.buffer_request_sync(bufnr, "textDocument/hover", {
    textDocument = document_identifier(bufnr),
    position = { line = line, character = character },
  }, 5000)
  assert(result ~= nil, "hover response was empty")
  return result
end

function M.hover_markdown(result)
  local contents = unwrap_nil(result.contents)
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

function M.write_file(path, contents)
  local handle = assert(io.open(path, "w"))
  handle:write(contents)
  handle:close()
end

function M.notify(client, method, params)
  local ok = client.notify(method, params)
  assert(ok ~= false, "failed to send " .. method .. " notification")
end

return M
