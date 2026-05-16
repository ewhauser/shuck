local M = {}

local function completion_items(result)
  assert(result ~= nil, "expected completion response")
  return result.items or result
end

local function labels_for(items)
  local labels = {}
  for _, item in ipairs(items) do
    labels[item.label] = item
  end
  return labels
end

local function assert_has(labels, label)
  assert(labels[label] ~= nil, "expected completion label " .. label)
end

local function assert_missing(labels, label)
  assert(labels[label] == nil, "did not expect completion label " .. label)
end

local function complete_at(t, bufnr, position)
  return completion_items(t.buffer_request_sync(bufnr, "textDocument/completion", {
    textDocument = t.document_identifier(bufnr),
    position = position,
  }, 5000))
end

function M.run(t)
  local bufnr, client_id = t.open_fixture("completion/semantic_completion.sh")
  local client = t.client(bufnr, client_id)

  assert(
    client.server_capabilities.completionProvider ~= nil,
    "expected completionProvider to be advertised"
  )
  assert(
    client.server_capabilities.completionProvider.resolveProvider == true,
    "expected completion resolve support"
  )

  local parameter_labels = labels_for(complete_at(t, bufnr, t.position_after_nth(bufnr, "$", 0)))
  assert_has(parameter_labels, "local_name")
  assert_has(parameter_labels, "top_level")
  assert_missing(parameter_labels, "hidden_name")
  assert_missing(parameter_labels, "build")

  local braced_labels = labels_for(complete_at(t, bufnr, t.position_after_nth(bufnr, "${", 0)))
  assert_has(braced_labels, "local_name")
  assert_has(braced_labels, "top_level")
  assert_missing(braced_labels, "hidden_name")

  local declaration_labels = labels_for(complete_at(t, bufnr, t.position_after_nth(bufnr, "local to", 0)))
  assert_has(declaration_labels, "top_level")
  assert_missing(declaration_labels, "hidden_name")
  assert_missing(declaration_labels, "build")

  local command_labels = labels_for(complete_at(t, bufnr, t.position_after_nth(bufnr, "b", 2)))
  assert_has(command_labels, "build")
  assert_has(command_labels, "break")
  assert_missing(command_labels, "top_level")
  assert_missing(command_labels, "local_name")

  local resolved_variable =
    t.client_request_sync(client, "completionItem/resolve", parameter_labels.top_level, bufnr, 5000)
  assert(
    resolved_variable.documentation.value:find("Variable defined at", 1, true),
    "expected variable completion documentation"
  )

  local resolved_builtin =
    t.client_request_sync(client, "completionItem/resolve", command_labels["break"], bufnr, 5000)
  assert(
    resolved_builtin.documentation.value:find("Shell builtin modeled by Shuck.", 1, true),
    "expected builtin completion documentation"
  )
end

return M
