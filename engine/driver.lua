--- Snapshotting LuaLaTeX driver; see docs/engine.md.
--- Env: `LLP_NATIVE` llpfork.so, `LLP_EVENTS` event FIFO, `LLP_RUN` run dir,
--- `LLP_SHADOW` dir mirroring unsaved editor buffers by absolute path, `LLP_GEN` generation.
local F = package.loadlib(os.getenv("LLP_NATIVE"), "luaopen_llpfork")()
local run_dir, shadow = os.getenv("LLP_RUN"), os.getenv("LLP_SHADOW")
local FINISH_GAP_S = 0.15
local ev = io.open(os.getenv("LLP_EVENTS"), "w")
ev:setvbuf("line")
local gen, page, start_page, last_finish_s = tonumber(os.getenv("LLP_GEN")), 0, 0, 0
local open_recs = {}
local max_read = {}

local function read_lines(path)
  local f = io.open(shadow .. path, "rb") or io.open(path, "rb")
  if not f then return nil end
  local s = f:read("a")
  f:close()
  local lines = {}
  for l in (s .. "\n"):gmatch("([^\n]*)\n") do lines[#lines + 1] = l:gsub("\r$", "") end
  if s:sub(-1) == "\n" then lines[#lines] = nil end
  return lines
end

local function emit(...)
  ev:write(table.concat({ gen, ... }, " "), "\n")
end

--- Serves every input file from memory so a resumed snapshot can swap in new content
--- while keeping its line position.
luatexbase.add_to_callback("open_read_file", function(name)
  local abs = name:sub(1, 1) == "/" and name or lfs.currentdir() .. "/" .. name
  local lines = read_lines(abs)
  if not lines then return nil end
  local rec = { name = abs, lines = lines, pos = 0 }
  open_recs[rec] = true
  return {
    reader = function()
      rec.pos = rec.pos + 1
      if rec.pos > (max_read[abs] or 0) then max_read[abs] = rec.pos end
      return rec.lines[rec.pos]
    end,
    close = function() open_recs[rec] = nil end,
  }
end, "llp.open_read_file")

local function encode_max_read()
  local t = {}
  for name, n in pairs(max_read) do
    if not name:find("/texmf", 1, true) then t[#t + 1] = name .. ":" .. n end
  end
  return table.concat(t, "\t")
end

--- What this process reports once its PDF, SyncTeX and log are closed.
local exit_msg = "done " .. lfs.currentdir()

--- Moves this process's outputs to `dir` and makes it report `msg` when they are closed.
local function become(dir, msg)
  lfs.mkdir(dir)
  local err = F.detach(dir)
  if err then emit("error", "detach", err) end
  exit_msg = msg
end

--- Snapshot: park on a FIFO; each `go GEN` forks a clone that resumes as generation GEN
--- with fresh source content; `quit` ends the snapshot. The clone stops Lua GC because a
--- collection touches the whole inherited heap, making the kernel copy it page by page.
local function park()
  local fd = F.mkfifo(run_dir .. "/snap-" .. F.getpid())
  emit("snap", page, F.getpid(), encode_max_read())
  while true do
    local cmd = F.readline(fd)
    local new_gen = cmd:match("^go (%d+)$")
    if not new_gen then F.exit(0) end
    F.reap()
    if F.fork(false) == 0 then
      collectgarbage("stop")
      gen, start_page, last_finish_s = tonumber(new_gen), page, 0
      local dir = run_dir .. "/g" .. gen
      become(dir, "done " .. dir)
      for rec in pairs(open_recs) do rec.lines = read_lines(rec.name) or rec.lines end
      emit("run", page, F.getpid())
      return
    end
  end
end

--- Finisher: closes a copy of the PDF right away so the viewer gets pages before this
--- generation reaches the end. It ends at the next `buildpage_filter`, once the page just
--- shipped is in the page tree; `tex.finish` inside `finish_pdfpage` would drop that page.
local finishing = false
local function finish_early()
  local dir = run_dir .. "/g" .. gen .. "p" .. page
  collectgarbage("stop")
  become(dir, "partial " .. page .. " " .. dir)
  finishing = true
end

luatexbase.add_to_callback("buildpage_filter", function()
  if finishing then tex.finish() end
end, "llp.buildpage_filter")

luatexbase.add_to_callback("finish_pdfpage", function(shipping)
  page = page + 1
  F.reap()
  if F.fork(true) == 0 then
    park()
    return shipping
  end
  local now = os.gettimeofday()
  if page > start_page and now - last_finish_s >= FINISH_GAP_S then
    last_finish_s = now
    if F.fork(true) == 0 then
      finish_early()
      return shipping
    end
  end
  emit("page", page)
  return shipping
end, "llp.finish_pdfpage")

--- Outputs are closed here; skipping LuaTeX's teardown saves ~30 ms per run.
luatexbase.add_to_callback("wrapup_run", function()
  emit(exit_msg)
  F.exit(0)
end, "llp.wrapup_run")
