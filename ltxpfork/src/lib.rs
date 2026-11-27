//! Lua 5.3 module for LuaTeX: fork-based snapshots of a running TeX process; see
//! `docs/engine.md`. The few Lua C API functions used are resolved against LuaTeX's own Lua
//! when the library is loaded.
use std::ffi::{CStr, CString, c_char, c_int};

#[allow(non_camel_case_types)]
type lua_State = std::ffi::c_void;
type LuaFn = unsafe extern "C" fn(*mut lua_State) -> c_int;

unsafe extern "C" {
    fn lua_pushinteger(l: *mut lua_State, n: i64);
    fn lua_pushstring(l: *mut lua_State, s: *const c_char) -> *const c_char;
    fn lua_pushnil(l: *mut lua_State);
    fn lua_createtable(l: *mut lua_State, narr: c_int, nrec: c_int);
    fn lua_pushcclosure(l: *mut lua_State, f: LuaFn, n: c_int);
    fn lua_setfield(l: *mut lua_State, idx: c_int, k: *const c_char);
    fn lua_toboolean(l: *mut lua_State, idx: c_int) -> c_int;
    fn luaL_checkinteger(l: *mut lua_State, arg: c_int) -> i64;
    fn luaL_checklstring(l: *mut lua_State, arg: c_int, len: *mut usize) -> *const c_char;
}

const MAX_FD: c_int = 256;

/// A writable output file open at the last recording fork.
struct Output {
    fd: c_int,
    /// base name; TeX writes its outputs into its working directory
    name: CString,
    /// bytes written before the fork
    len: libc::off_t,
    /// read-only handle opened before the fork, so the prefix stays readable even after the
    /// parent renames (`main.synctex(busy)`) or deletes the file
    read_fd: c_int,
}

static mut RECORDED: Vec<Output> = Vec::new();

unsafe fn push_str(l: *mut lua_State, s: &[u8]) {
    let c = CString::new(s).unwrap_or_default();
    unsafe { lua_pushstring(l, c.as_ptr()) };
}

unsafe fn arg_str<'a>(l: *mut lua_State, i: c_int) -> &'a CStr {
    unsafe { CStr::from_ptr(luaL_checklstring(l, i, std::ptr::null_mut())) }
}

#[cfg(target_os = "linux")]
fn fd_path(fd: c_int) -> Option<CString> {
    let p = std::fs::read_link(format!("/proc/self/fd/{fd}")).ok()?;
    CString::new(p.into_os_string().into_encoded_bytes()).ok()
}

#[cfg(target_os = "macos")]
fn fd_path(fd: c_int) -> Option<CString> {
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    if unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) } < 0 {
        return None;
    }
    Some(CStr::from_bytes_until_nul(&buf).ok()?.to_owned())
}

/// Writable regular-file fds with names, offsets and read handles.
unsafe fn record_outputs() -> Vec<Output> {
    let mut out = Vec::new();
    for fd in 3..MAX_FD {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } != 0 || st.st_mode & libc::S_IFMT != libc::S_IFREG {
            continue;
        }
        if unsafe { libc::fcntl(fd, libc::F_GETFL) } & libc::O_ACCMODE == libc::O_RDONLY {
            continue;
        }
        let Some(path) = fd_path(fd) else { continue };
        let bytes = path.as_bytes();
        let base = &bytes[bytes.iter().rposition(|&b| b == b'/').map_or(0, |i| i + 1)..];
        let Ok(name) = CString::new(base) else {
            continue;
        };
        let read_fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
        if read_fd < 0 {
            continue;
        }
        let len = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) };
        out.push(Output {
            fd,
            name,
            len,
            read_fd,
        });
    }
    out
}

/// Drops the record, closing its read handles.
unsafe fn clear_record() {
    for o in unsafe { std::mem::take(&mut *std::ptr::addr_of_mut!(RECORDED)) } {
        unsafe { libc::close(o.read_fd) };
    }
}

/// `fork(record)`: flushes stdio and forks. With `record`, the child gets a fresh record of the
/// writable outputs for a later `detach`; a clone of a parked snapshot passes false to keep
/// the snapshot's record. Returns the child pid in the parent, 0 in the child, nil on failure.
unsafe extern "C" fn l_fork(l: *mut lua_State) -> c_int {
    unsafe {
        libc::fflush(std::ptr::null_mut());
        let record = lua_toboolean(l, 1) != 0;
        if record {
            clear_record();
            RECORDED = record_outputs();
        }
        let pid = libc::fork();
        if pid != 0 && record {
            clear_record();
        }
        match pid {
            pid if pid < 0 => lua_pushnil(l),
            pid => lua_pushinteger(l, pid.into()),
        }
    }
    1
}

/// Copies the first `len` bytes of `src` to the start of `dst`.
unsafe fn copy_prefix(src: c_int, dst: c_int, len: libc::off_t) -> bool {
    let mut buf = vec![0u8; 1 << 20];
    let mut off: libc::off_t = 0;
    while off < len {
        let want = buf.len().min((len - off) as usize);
        let n = unsafe { libc::pread(src, buf.as_mut_ptr().cast(), want, off) };
        if n <= 0 || unsafe { libc::pwrite(dst, buf.as_ptr().cast(), n as usize, off) } != n {
            return false;
        }
        off += n as libc::off_t;
    }
    true
}

/// `detach(dir)`: makes `dir` the working directory, where TeX writes its outputs, and gives
/// every recorded output its own copy there holding the bytes written before the fork, so
/// processes never share output. Returns nil, or the path that failed.
unsafe extern "C" fn l_detach(l: *mut lua_State) -> c_int {
    unsafe {
        let dir = arg_str(l, 1);
        if libc::chdir(dir.as_ptr()) != 0 {
            push_str(l, dir.to_bytes());
            return 1;
        }
        for o in (*std::ptr::addr_of!(RECORDED)).iter() {
            let dst = libc::open(
                o.name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
                0o644,
            );
            if dst < 0 || !copy_prefix(o.read_fd, dst, o.len) {
                push_str(l, o.name.to_bytes());
                return 1;
            }
            libc::dup2(dst, o.fd);
            libc::close(dst);
            libc::lseek(o.fd, o.len, libc::SEEK_SET);
        }
        clear_record();
        lua_pushnil(l);
    }
    1
}

/// `readline(fd)`: blocks reading one line from `fd`.
unsafe extern "C" fn l_readline(l: *mut lua_State) -> c_int {
    unsafe {
        let fd = luaL_checkinteger(l, 1) as c_int;
        let mut buf = Vec::new();
        let mut c = 0u8;
        while libc::read(fd, (&mut c as *mut u8).cast(), 1) == 1 && c != b'\n' {
            buf.push(c);
        }
        push_str(l, &buf);
    }
    1
}

/// `mkfifo(path)`: creates a FIFO and opens it read-write so the open never blocks; returns fd.
unsafe extern "C" fn l_mkfifo(l: *mut lua_State) -> c_int {
    unsafe {
        let p = arg_str(l, 1);
        libc::mkfifo(p.as_ptr(), 0o600);
        lua_pushinteger(l, libc::open(p.as_ptr(), libc::O_RDWR).into());
    }
    1
}

/// `exit(code)`: exits without atexit handlers or stdio flushes, so a dropped snapshot never
/// touches shared output.
unsafe extern "C" fn l_exit(l: *mut lua_State) -> c_int {
    unsafe { libc::_exit(luaL_checkinteger(l, 1) as c_int) }
}

/// `reap()`: collects exited children without blocking.
unsafe extern "C" fn l_reap(_l: *mut lua_State) -> c_int {
    while unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } > 0 {}
    0
}

unsafe extern "C" fn l_getpid(l: *mut lua_State) -> c_int {
    unsafe { lua_pushinteger(l, libc::getpid().into()) };
    1
}

/// # Safety
/// Called by Lua's `package.loadlib` with a valid state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn luaopen_ltxpfork(l: *mut lua_State) -> c_int {
    let fns: [(&CStr, LuaFn); 7] = [
        (c"fork", l_fork),
        (c"detach", l_detach),
        (c"readline", l_readline),
        (c"mkfifo", l_mkfifo),
        (c"exit", l_exit),
        (c"reap", l_reap),
        (c"getpid", l_getpid),
    ];
    unsafe {
        lua_createtable(l, 0, fns.len() as c_int);
        for (name, f) in fns {
            lua_pushcclosure(l, f, 0);
            lua_setfield(l, -2, name.as_ptr());
        }
    }
    1
}
