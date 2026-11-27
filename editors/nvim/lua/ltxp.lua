--- Neovim client for `ltxp`, the instant LaTeX preview language server.
--- Nothing starts on its own: `:LtxpPreview` starts ltxp for the current TeX buffer and opens
--- the preview; `:LtxpStop` stops it; `:LtxpLog` opens the latest LaTeX log; `:LtxpSync [on|off]`
--- toggles the preview following the cursor. Options for `setup`:
--- `{ cmd = { "/path/to/ltxp" }, cursor_delay_ms = 100 }`.
local M = { configured = false, opts = {}, sync = true }

local TEX_FT = { tex = true, plaintex = true, latex = true }

local function client_for(buf)
  return vim.lsp.get_clients({ bufnr = buf, name = "ltxp" })[1]
end

local function notify_cursor(buf)
  if not M.sync then return end
  local client = client_for(buf)
  if not client then return end
  client:notify("ltxp/cursor", vim.lsp.util.make_position_params(0, client.offset_encoding))
end

--- Asks the server for the viewer URL and opens it in the browser, reporting failures.
local function open_preview(client)
  client:request("workspace/executeCommand", { command = "ltxp.previewUrl", arguments = {} }, function(err, url)
    if err or type(url) ~= "string" then
      vim.notify("ltxp: no preview yet: " .. (err and err.message or "the file has no \\documentclass, and no main file was found"), vim.log.levels.ERROR)
      return
    end
    local _, open_err = vim.ui.open(url)
    if open_err then
      vim.notify(("ltxp: cannot open a browser (%s); open %s yourself"):format(open_err, url), vim.log.levels.WARN)
    else
      vim.notify("ltxp: preview at " .. url)
    end
  end)
end

local function track_cursor(buf)
  local timer = assert(vim.uv.new_timer())
  vim.api.nvim_create_autocmd({ "CursorMoved", "CursorMovedI" }, {
    group = vim.api.nvim_create_augroup("ltxp_cursor_" .. buf, {}),
    buffer = buf,
    callback = function()
      timer:stop()
      timer:start(M.opts.cursor_delay_ms or 100, 0, vim.schedule_wrap(function() notify_cursor(buf) end))
    end,
  })
end

--- Starts ltxp for `buf` (or reuses the running one) and opens the preview.
function M.preview(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  if not TEX_FT[vim.bo[buf].filetype] then
    vim.notify("ltxp: :LtxpPreview needs a TeX buffer", vim.log.levels.ERROR)
    return
  end
  local running = client_for(buf)
  if running then return open_preview(running) end
  local cmd = M.opts.cmd or { "ltxp" }
  if vim.fn.executable(cmd[1]) == 0 then
    vim.notify(("ltxp: %s not found; install ltxp or set cmd in setup()"):format(cmd[1]), vim.log.levels.ERROR)
    return
  end
  local root = vim.fs.root(buf, { ".git", ".latexmkrc" }) or vim.fs.dirname(vim.api.nvim_buf_get_name(buf))
  local id = vim.lsp.start({
    name = "ltxp",
    cmd = cmd,
    root_dir = root,
    init_options = { openBrowser = false },
    on_attach = function(client, b)
      track_cursor(b)
      if b == buf then open_preview(client) end
    end,
    on_exit = function(code)
      if code ~= 0 then
        vim.schedule(function() vim.notify(("ltxp exited with code %d; see :LspLog"):format(code), vim.log.levels.ERROR) end)
      end
    end,
  }, { bufnr = buf })
  if not id then return end
  -- The preview follows the whole project: attach other TeX buffers under the same root.
  local function attach(b)
    local name = vim.api.nvim_buf_get_name(b)
    if b ~= buf and TEX_FT[vim.bo[b].filetype] and vim.startswith(name, root) and vim.lsp.get_client_by_id(id) then
      vim.lsp.buf_attach_client(b, id)
    end
  end
  for _, b in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(b) then attach(b) end
  end
  vim.api.nvim_create_autocmd("FileType", {
    group = vim.api.nvim_create_augroup("ltxp_attach", {}),
    pattern = vim.tbl_keys(TEX_FT),
    callback = function(a) attach(a.buf) end,
  })
end

--- Opens the LaTeX log of the latest finished compile in a split.
function M.log()
  local client = client_for(vim.api.nvim_get_current_buf()) or vim.lsp.get_clients({ name = "ltxp" })[1]
  if not client then
    vim.notify("ltxp: not running; run :LtxpPreview first", vim.log.levels.ERROR)
    return
  end
  client:request("workspace/executeCommand", { command = "ltxp.logPath", arguments = {} }, function(err, path)
    if err or type(path) ~= "string" then
      vim.notify("ltxp: no log yet: no compile has finished", vim.log.levels.WARN)
      return
    end
    vim.cmd("split " .. vim.fn.fnameescape(path))
    vim.bo.readonly = true
    vim.cmd("normal! G")
  end)
end

--- `arg`: "on", "off", or "" to toggle whether the preview follows the cursor.
function M.set_sync(arg)
  if arg == "on" then M.sync = true elseif arg == "off" then M.sync = false else M.sync = not M.sync end
  vim.notify("ltxp: preview follows cursor: " .. (M.sync and "on" or "off"))
end

function M.stop()
  for _, c in ipairs(vim.lsp.get_clients({ name = "ltxp" })) do c:stop() end
  pcall(vim.api.nvim_del_augroup_by_name, "ltxp_attach")
end

function M.setup(opts)
  M.opts = opts or {}
  M.configured = true
  vim.api.nvim_create_user_command("LtxpPreview", function() M.preview() end, { desc = "Start ltxp and open the LaTeX preview" })
  vim.api.nvim_create_user_command("LtxpStop", M.stop, { desc = "Stop the ltxp LaTeX preview" })
  vim.api.nvim_create_user_command("LtxpLog", M.log, { desc = "Open the latest LaTeX log" })
  vim.api.nvim_create_user_command("LtxpSync", function(a) M.set_sync(a.args) end, {
    nargs = "?",
    complete = function() return { "on", "off" } end,
    desc = "Toggle the preview following the cursor",
  })
end

return M
