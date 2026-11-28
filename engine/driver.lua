--- Snapshotting LuaLaTeX driver; see docs/engine.md.
--- Run as `\directlua{dofile(os.getenv("LTXP_DRIVER"))}`; inputs `LTXP_MAIN` when loaded.
--- Env: `LTXP_NATIVE` libltxpfork.so, `LTXP_EVENTS` event FIFO, `LTXP_RUN` run dir,
--- `LTXP_SHADOW` dir mirroring unsaved editor buffers by absolute path, `LTXP_GEN` generation,
--- `LTXP_PROJECT` the main file's dir. TeX starts in its output dir and writes outputs there.
local F = package.loadlib(os.getenv("LTXP_NATIVE"), "luaopen_ltxpfork")()
local run_dir, shadow = os.getenv("LTXP_RUN"), os.getenv("LTXP_SHADOW")
local FINISH_GAP_S = 0.15
local ev = io.open(os.getenv("LTXP_EVENTS"), "w")
ev:setvbuf("line")
local gen, page, start_page, last_finish_s = tonumber(os.getenv("LTXP_GEN")), 0, 0, 0
--- Paragraph snapshots since the last page end, and when the last snapshot was taken.
local sub, last_snap_s = 0, 0
--- Least time between paragraph snapshots, so a full compile forks only a few per page.
local PAR_SNAP_GAP_S = 0.05
local open_recs = {}

--- Shell commands (`\write18`, `\ShellEscape`, ...) run in the project dir, as in a normal
--- build, rather than in this process's output dir.
local project = os.getenv("LTXP_PROJECT")
local cd_project = "cd '" .. project:gsub("'", "'\\''") .. "' && "
for _, f in ipairs({ "execute", "spawn", "exec" }) do
  local orig = os[f]
  if orig then
    os[f] = function(cmd, ...)
      if type(cmd) == "string" then cmd = cd_project .. cmd end
      return orig(cmd, ...)
    end
  end
end
local orig_popen = io.popen
io.popen = function(cmd, ...)
  return orig_popen(type(cmd) == "string" and cd_project .. cmd or cmd, ...)
end
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

local function is_file(path)
  return lfs.attributes(path, "mode") == "file"
end

--- Where a file TeX asks for really is. Like a normal build in the project dir: this run's
--- own outputs (`main.aux`, ...) in the cwd first, then paths relative to the project dir
--- (so `\input{../sections/x}` works), then the usual kpathsea search.
local function resolve(name, kpse_type)
  for _, n in ipairs({ name, name .. ".tex" }) do
    if n:sub(1, 1) == "/" then
      if is_file(n) then return n end
    else
      if is_file(n) then return n end
      if is_file(project .. "/" .. n) then return project .. "/" .. n end
    end
  end
  if name:sub(1, 1) == "/" then return nil end
  return kpse.find_file(name, kpse_type)
end

luatexbase.add_to_callback("find_read_file", function(_, name)
  return resolve(name, "tex")
end, "ltxp.find_read_file")

luatexbase.add_to_callback("find_image_file", function(name)
  return resolve(name, "graphic/figure") or name
end, "ltxp.find_image_file")

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
end, "ltxp.open_read_file")

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
--- with fresh source content; `quit` ends the snapshot. The clone pauses Lua GC until its
--- first new page is out: a collection touches the whole inherited heap, making the kernel
--- copy it page by page.
local function park()
  local fd = F.mkfifo(run_dir .. "/snap-" .. F.getpid())
  emit("snap", page, sub, F.getpid(), encode_max_read())
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

--- Takes a snapshot if the last one is at least `PAR_SNAP_GAP_S` old. Called between
--- paragraphs and at page ends, where TeX can resume with the rest of the source changed.
local function maybe_snapshot(at_page_end)
  local now = os.gettimeofday()
  if not at_page_end and now - last_snap_s < PAR_SNAP_GAP_S then return end
  last_snap_s = now
  sub = at_page_end and 0 or sub + 1
  F.reap()
  if F.fork(true) == 0 then park() end
end

luatexbase.add_to_callback("buildpage_filter", function(info)
  if finishing then tex.finish() end
  -- A paragraph just went onto the page being built.
  if info == "hmode_par" and not finishing then maybe_snapshot(false) end
end, "ltxp.buildpage_filter")

--- Snapshot "page 0", taken once the preamble is done (LaTeX's `begindocument/end` hook), so
--- edits on page 1 skip the preamble, which is most of the work for short documents.
function ltxp_snapshot_begin_document()
  if page == 0 and start_page == 0 then maybe_snapshot(true) end
end

luatexbase.add_to_callback("finish_pdfpage", function(shipping)
  -- Also called for form XObjects (embedded PDF figures, saved boxes); only pages count.
  if not shipping then return shipping end
  page = page + 1
  if page == start_page + 2 then collectgarbage("restart") end
  maybe_snapshot(true)
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
end, "ltxp.finish_pdfpage")

--- Outputs are closed here; skipping LuaTeX's teardown saves ~30 ms per run.
luatexbase.add_to_callback("wrapup_run", function()
  emit(exit_msg)
  F.exit(0)
end, "ltxp.wrapup_run")

-- Newer LaTeX checks that a file exists with `kpse.find_file`, which resolves `../x` against
-- this process's output dir. Searching the project dir too (`\input@path`, also used by
-- graphicx) makes `\input{../sections/x}` and `\includegraphics{../figs/x}` work.
tex.sprint("\\expandafter\\def\\csname input@path\\endcsname{{")
tex.sprint(-2, project .. "/")
tex.sprint("}}")
tex.sprint("\\AddToHook{begindocument/end}{\\directlua{ltxp_snapshot_begin_document()}}")
tex.sprint("\\input{")
tex.sprint(-2, os.getenv("LTXP_MAIN"))
tex.sprint("}")
