--- Neovim client for `ltxp`, the LaTeX live-preview language server.
--- `plugin/ltxp.lua` calls `setup` with defaults unless you call it first, e.g.
--- `require("ltxp").setup({ cmd = { "/path/to/ltxp" }, cursor_delay_ms = 100 })`.
local M = { configured = false }

local function notify_cursor(buf)
  local client = vim.lsp.get_clients({ bufnr = buf, name = "ltxp" })[1]
  if not client then return end
  local params = vim.lsp.util.make_position_params(0, client.offset_encoding)
  client:notify("ltxp/cursor", params)
end

function M.setup(opts)
  opts = opts or {}
  M.configured = true
  local group = vim.api.nvim_create_augroup("ltxp", {})
  vim.api.nvim_create_autocmd("FileType", {
    group = group,
    pattern = { "tex", "plaintex", "latex" },
    callback = function(a)
      vim.lsp.start({
        name = "ltxp",
        cmd = opts.cmd or { "ltxp" },
        root_dir = vim.fs.root(a.buf, { ".git", ".latexmkrc" }) or vim.fs.dirname(vim.api.nvim_buf_get_name(a.buf)),
      }, { bufnr = a.buf })
      local timer = assert(vim.uv.new_timer())
      vim.api.nvim_create_autocmd({ "CursorMoved", "CursorMovedI" }, {
        group = group,
        buffer = a.buf,
        callback = function()
          timer:stop()
          timer:start(opts.cursor_delay_ms or 100, 0, vim.schedule_wrap(function() notify_cursor(a.buf) end))
        end,
      })
    end,
  })
end

return M
