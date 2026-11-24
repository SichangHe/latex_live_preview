/* Lua 5.3 C module for LuaTeX: fork-based snapshots of a running TeX process.
 * Lua headers are absent on the host; the few API prototypes are declared here and
 * resolved against LuaTeX's own Lua at load time. */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sendfile.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

typedef struct lua_State lua_State;
typedef long long lua_Integer;
typedef int (*lua_CFunction)(lua_State *);
extern void lua_pushinteger(lua_State *, lua_Integer);
extern void lua_pushstring(lua_State *, const char *);
extern void lua_pushnil(lua_State *);
extern void lua_createtable(lua_State *, int, int);
extern void lua_pushcclosure(lua_State *, lua_CFunction, int);
extern void lua_setfield(lua_State *, int, const char *);
extern int lua_toboolean(lua_State *, int);
extern lua_Integer luaL_checkinteger(lua_State *, int);
extern const char *luaL_checklstring(lua_State *, int, size_t *);

#define MAX_FD 256
static off_t fork_offs[MAX_FD];
static char fork_names[MAX_FD][4096];

/** Fork, flushing stdio first. With `record` true, first record names and write offsets of
 * all writable regular-file fds for a later `detach`; a clone of a parked snapshot passes
 * false to keep the snapshot's own record. Returns child pid in parent, 0 in child. */
static int l_fork(lua_State *L) {
  fflush(NULL);
  int record = lua_toboolean(L, 1);
  for (int fd = 3; fd < MAX_FD; fd++) {
    struct stat st;
    if (!record) break;
    fork_offs[fd] = -1;
    if (fstat(fd, &st) == 0 && S_ISREG(st.st_mode)) {
      int fl = fcntl(fd, F_GETFL);
      if ((fl & O_ACCMODE) != O_RDONLY) {
        char link[64], path[4096];
        snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
        ssize_t n = readlink(link, path, sizeof path - 1);
        if (n < 0) continue;
        path[n] = 0;
        const char *base = strrchr(path, '/');
        snprintf(fork_names[fd], sizeof fork_names[fd], "%s", base ? base + 1 : path);
        fork_offs[fd] = lseek(fd, 0, SEEK_CUR);
      }
    }
  }
  pid_t pid = fork();
  if (pid < 0) { lua_pushnil(L); return 1; }
  lua_pushinteger(L, pid);
  return 1;
}

/** Child after resume: chdir to `dir` and give every inherited writable file its own copy
 * there holding the bytes written before the fork, so generations never share output.
 * Names are taken at fork time and the prefix is read through /proc, because the parent may
 * have renamed or deleted the file since. */
static int l_detach(lua_State *L) {
  const char *dir = luaL_checklstring(L, 1, NULL);
  char link[64];
  if (chdir(dir) != 0) { lua_pushstring(L, "chdir failed"); return 1; }
  for (int fd = 3; fd < MAX_FD; fd++) {
    if (fork_offs[fd] < 0) continue;
    snprintf(link, sizeof link, "/proc/self/fd/%d", fd);
    int in = open(link, O_RDONLY), out = open(fork_names[fd], O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (in < 0 || out < 0) { lua_pushstring(L, fork_names[fd]); return 1; }
    off_t o = 0;
    while (o < fork_offs[fd] && sendfile(out, in, &o, fork_offs[fd] - o) > 0) {}
    close(in);
    dup2(out, fd);
    close(out);
    lseek(fd, fork_offs[fd], SEEK_SET);
  }
  lua_pushnil(L);
  return 1;
}

/** Block reading one line from fd (the snapshot's control pipe). */
static int l_readline(lua_State *L) {
  int fd = (int)luaL_checkinteger(L, 1);
  char buf[4096];
  size_t n = 0;
  char c;
  while (n < sizeof buf - 1 && read(fd, &c, 1) == 1 && c != '\n') buf[n++] = c;
  buf[n] = 0;
  lua_pushstring(L, buf);
  return 1;
}

static int l_reap(lua_State *L) {
  (void)L;
  while (waitpid(-1, NULL, WNOHANG) > 0) {}
  return 0;
}

/** Create a FIFO and open it read-write so open() never blocks; returns fd. */
static int l_mkfifo(lua_State *L) {
  const char *p = luaL_checklstring(L, 1, NULL);
  mkfifo(p, 0600);
  lua_pushinteger(L, open(p, O_RDWR));
  return 1;
}

/** Exit without atexit handlers or stdio flushes, so a dropped snapshot never touches shared output. */
static int l_exit(lua_State *L) {
  _exit((int)luaL_checkinteger(L, 1));
  return 0;
}

static int l_getpid(lua_State *L) {
  lua_pushinteger(L, getpid());
  return 1;
}

int luaopen_llpfork(lua_State *L) {
  lua_createtable(L, 0, 7);
  lua_pushcclosure(L, l_fork, 0); lua_setfield(L, -2, "fork");
  lua_pushcclosure(L, l_detach, 0); lua_setfield(L, -2, "detach");
  lua_pushcclosure(L, l_readline, 0); lua_setfield(L, -2, "readline");
  lua_pushcclosure(L, l_mkfifo, 0); lua_setfield(L, -2, "mkfifo");
  lua_pushcclosure(L, l_exit, 0); lua_setfield(L, -2, "exit");
  lua_pushcclosure(L, l_reap, 0); lua_setfield(L, -2, "reap");
  lua_pushcclosure(L, l_getpid, 0); lua_setfield(L, -2, "getpid");
  return 1;
}
