local init_path = debug.getinfo(1, "S").source:sub(2)
local manual_root = vim.fn.fnamemodify(init_path, ":p:h")

if vim.loader and vim.loader.enable then
  vim.loader.enable()
end

package.path = table.concat({
  manual_root .. "/lua/?.lua",
  manual_root .. "/lua/?/init.lua",
  package.path,
}, ";")

vim.cmd("filetype on")
vim.opt.swapfile = false
vim.opt.shadafile = "NONE"
vim.opt.shortmess:append("I")
