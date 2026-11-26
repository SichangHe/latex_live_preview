-- Starts llp for TeX files. Set `vim.g.llp_disable = true` to opt out, or
-- `vim.g.llp_opts` to pass options to `require("llp").setup`.
if vim.g.llp_disable or require("llp").configured then return end
require("llp").setup(vim.g.llp_opts)
