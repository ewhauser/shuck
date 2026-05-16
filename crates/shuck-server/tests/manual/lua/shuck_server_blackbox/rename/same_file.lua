local M = {}

local function workspace_edit_count(edit)
  if edit.documentChanges ~= nil then
    local count = 0
    for _, change in ipairs(edit.documentChanges) do
      count = count + #(change.edits or {})
    end
    return count
  end

  local count = 0
  for _, edits in pairs(edit.changes or {}) do
    count = count + #edits
  end
  return count
end

local function prepare_rename(t, bufnr, position)
  return t.buffer_request_sync(bufnr, "textDocument/prepareRename", {
    textDocument = t.document_identifier(bufnr),
    position = position,
  }, 5000)
end

function M.run(t)
  local bufnr, client_id = t.open_fixture("rename/same_file.sh")
  local client = t.client(bufnr, client_id)

  assert(client.server_capabilities.renameProvider ~= nil, "expected renameProvider")
  assert(
    client.server_capabilities.renameProvider.prepareProvider == true,
    "expected prepareRename support"
  )

  local top_level_reference = t.position_for_nth(bufnr, "name", 1)
  local prepared = prepare_rename(t, bufnr, top_level_reference)
  assert(prepared ~= nil, "expected top-level variable to be renameable")
  assert(prepared.placeholder == "name", "expected original placeholder")

  local edit = t.buffer_request_sync(bufnr, "textDocument/rename", {
    textDocument = t.document_identifier(bufnr),
    position = top_level_reference,
    newName = "target_name",
  }, 5000)
  assert(edit ~= nil, "expected rename edit")
  assert(workspace_edit_count(edit) == 2, "expected two edits: " .. vim.inspect(edit))

  t.apply_workspace_edit(client, edit)
  t.wait_for_buffer_lines(bufnr, {
    "#!/usr/bin/env bash",
    "target_name=world",
    "echo \"$target_name\"",
    "",
    "shadowed() {",
    "  local name=shadow",
    "  echo \"$name\"",
    "}",
    "",
    "echo \"$1\"",
    "echo \"$?\"",
    "declare -n ref=name",
    "echo \"$ref\"",
  })

  assert(
    prepare_rename(t, bufnr, t.position_after_nth(bufnr, "$1", 0)) == nil,
    "expected positional parameter rename to be rejected"
  )
  assert(
    prepare_rename(t, bufnr, t.position_after_nth(bufnr, "$?", 0)) == nil,
    "expected special parameter rename to be rejected"
  )
  assert(
    prepare_rename(t, bufnr, t.position_for_nth(bufnr, "ref", 1)) == nil,
    "expected nameref rename to be rejected"
  )
end

return M
