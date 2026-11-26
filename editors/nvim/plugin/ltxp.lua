-- Starts ltxp for TeX files. Set `vim.g.ltxp_disable = true` to opt out, or
-- `vim.g.ltxp_opts` to pass options to `require("ltxp").setup`.
if vim.g.ltxp_disable or require("ltxp").configured then return end
require("ltxp").setup(vim.g.ltxp_opts)
