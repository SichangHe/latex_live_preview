--- Neovim client for `llp`, the LaTeX live-preview language server.
--- Usage: `require("llp").setup({ cmd = { "/path/to/latex_live_preview/server/main.js" } })`.
local M = {}

local function notify_cursor(buf)
  local client = vim.lsp.get_clients({ bufnr = buf, name = "llp" })[1]
  if not client then return end
  local params = vim.lsp.util.make_position_params(0, client.offset_encoding)
  client:notify("llp/cursor", params)
end

function M.setup(opts)
  opts = opts or {}
  local group = vim.api.nvim_create_augroup("llp", {})
  vim.api.nvim_create_autocmd("FileType", {
    group = group,
    pattern = { "tex", "plaintex", "latex" },
    callback = function(a)
      vim.lsp.start({
        name = "llp",
        cmd = opts.cmd or { "llp" },
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
