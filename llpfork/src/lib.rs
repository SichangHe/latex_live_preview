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
/// TeX runs with `-output-directory=/proc/self/fd/200`, so each process's output directory is
/// whatever its own fd 200 refers to.
const OUT_DIR_FD: c_int = 200;

/// Writable regular files open at the last recording fork: (fd, name, write offset).
static mut RECORDED: Vec<(c_int, CString, libc::off_t)> = Vec::new();

unsafe fn push_str(l: *mut lua_State, s: &[u8]) {
    let c = CString::new(s).unwrap_or_default();
    unsafe { lua_pushstring(l, c.as_ptr()) };
}

unsafe fn arg_str<'a>(l: *mut lua_State, i: c_int) -> &'a CStr {
    unsafe { CStr::from_ptr(luaL_checklstring(l, i, std::ptr::null_mut())) }
}

fn fd_link(fd: c_int) -> CString {
    CString::new(format!("/proc/self/fd/{fd}")).expect("no NUL in fd path")
}

/// Writable regular-file fds with their base names and offsets.
unsafe fn record_outputs() -> Vec<(c_int, CString, libc::off_t)> {
    let mut out = Vec::new();
    for fd in 3..MAX_FD {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut st) } != 0 || st.st_mode & libc::S_IFMT != libc::S_IFREG {
            continue;
        }
        if unsafe { libc::fcntl(fd, libc::F_GETFL) } & libc::O_ACCMODE == libc::O_RDONLY {
            continue;
        }
        let Ok(path) = std::fs::read_link(format!("/proc/self/fd/{fd}")) else {
            continue;
        };
        let Some(name) = path.file_name() else {
            continue;
        };
        let Ok(name) = CString::new(name.as_encoded_bytes()) else {
            continue;
        };
        out.push((fd, name, unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) }));
    }
    out
}

/// `fork(record)`: flushes stdio and forks. With `record`, first records writable outputs for a
/// later `detach`; a clone of a parked snapshot passes false to keep the snapshot's record.
/// Returns the child pid in the parent, 0 in the child, nil on failure.
unsafe extern "C" fn l_fork(l: *mut lua_State) -> c_int {
    unsafe {
        libc::fflush(std::ptr::null_mut());
        if lua_toboolean(l, 1) != 0 {
            RECORDED = record_outputs();
        }
        match libc::fork() {
            pid if pid < 0 => lua_pushnil(l),
            pid => lua_pushinteger(l, pid.into()),
        }
    }
    1
}

/// `detach(dir)`: points fd `OUT_DIR_FD` at `dir` and gives every recorded output its own copy
/// there holding the bytes written before the fork, so processes never share output. Names
/// and offsets come from fork time, and the prefix is read through /proc, because the parent
/// may have renamed or deleted the file since. Returns nil, or the path that failed.
unsafe extern "C" fn l_detach(l: *mut lua_State) -> c_int {
    unsafe {
        let dir = arg_str(l, 1);
        let dfd = libc::open(dir.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY);
        if dfd < 0 || libc::dup2(dfd, OUT_DIR_FD) < 0 {
            push_str(l, dir.to_bytes());
            return 1;
        }
        libc::close(dfd);
        for (fd, name, len) in (*std::ptr::addr_of!(RECORDED)).iter() {
            let src = libc::open(fd_link(*fd).as_ptr(), libc::O_RDONLY);
            let dst = libc::openat(
                OUT_DIR_FD,
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_TRUNC,
                0o644,
            );
            if src < 0 || dst < 0 {
                push_str(l, name.to_bytes());
                return 1;
            }
            let mut off: libc::off_t = 0;
            while off < *len && libc::sendfile(dst, src, &mut off, (*len - off) as usize) > 0 {}
            libc::close(src);
            libc::dup2(dst, *fd);
            libc::close(dst);
            libc::lseek(*fd, *len, libc::SEEK_SET);
        }
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
pub unsafe extern "C" fn luaopen_llpfork(l: *mut lua_State) -> c_int {
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
