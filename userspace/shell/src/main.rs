//! LightOS shell: line editor over read(0), builtins (cd, pwd, ls,
//! cat, echo, exit), external commands via fork + exec + wait.
//!
//! The kernel has no per-process cwd in v1, so the shell tracks it and
//! resolves relative paths itself before making syscalls.
#![no_std]
#![no_main]

use libc_shim::{
    close, exec, exit, fork, getdents64, getpid, open, open_flags, print, println, read, wait,
    write, O_DIRECTORY,
};

const MAX_LINE: usize = 256;
const MAX_PATH: usize = 120;

struct Cwd {
    buf: [u8; MAX_PATH],
    len: usize,
}

impl Cwd {
    fn new() -> Self {
        let mut buf = [0u8; MAX_PATH];
        buf[0] = b'/';
        Cwd { buf, len: 1 }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("/")
    }

    /// Resolve `path` against the cwd into `out`; handles "..", ".".
    fn resolve<'a>(&self, path: &str, out: &'a mut [u8; MAX_PATH]) -> &'a str {
        let mut len = 0usize;
        let absolute = path.starts_with('/');
        if !absolute {
            len = self.len;
            out[..len].copy_from_slice(&self.buf[..self.len]);
        } else {
            out[0] = b'/';
            len = 1;
        }
        for comp in path.split('/').filter(|c| !c.is_empty()) {
            match comp {
                "." => {}
                ".." => {
                    while len > 1 && out[len - 1] != b'/' {
                        len -= 1;
                    }
                    if len > 1 {
                        len -= 1; // drop the trailing '/'
                    }
                }
                _ => {
                    if len > 1 || out[0] != b'/' || len == 0 {
                        if len + 1 < MAX_PATH {
                            out[len] = b'/';
                            len += 1;
                        }
                    } else if len == 1 && out[0] == b'/' {
                        // root: components append directly after '/'
                    }
                    let n = comp.len().min(MAX_PATH - len - 1);
                    out[len..len + n].copy_from_slice(&comp.as_bytes()[..n]);
                    len += n;
                }
            }
        }
        if len == 0 {
            out[0] = b'/';
            len = 1;
        }
        core::str::from_utf8(&out[..len]).unwrap_or("/")
    }
}

fn read_line(buf: &mut [u8; MAX_LINE]) -> usize {
    let mut len = 0;
    loop {
        let mut byte = [0u8; 1];
        let n = read(0, &mut byte);
        if n <= 0 {
            return len;
        }
        match byte[0] {
            b'\n' => return len,
            0x7f | 0x08 => {
                // backspace: the kernel already echoed it; best-effort
                if len > 0 {
                    len -= 1;
                }
            }
            b => {
                if len < MAX_LINE - 1 {
                    buf[len] = b;
                    len += 1;
                }
            }
        }
    }
}

fn cmd_ls(path: &str) {
    let fd = open_flags(path, O_DIRECTORY);
    if fd < 0 {
        println!("ls: cannot open {}: error {}", path, fd);
        return;
    }
    let mut buf = [0u8; 512];
    loop {
        let n = getdents64(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let mut off = 0usize;
        while off + 19 < n as usize {
            let reclen =
                u16::from_le_bytes([buf[off + 16], buf[off + 17]]) as usize;
            let dtype = buf[off + 18];
            let name_end = (off + 19..off + reclen)
                .find(|&i| buf[i] == 0)
                .unwrap_or(off + reclen);
            if let Ok(name) = core::str::from_utf8(&buf[off + 19..name_end]) {
                if dtype == 4 {
                    print!("{}/  ", name);
                } else {
                    print!("{}  ", name);
                }
            }
            off += reclen;
        }
    }
    println!();
    close(fd);
}

fn cmd_cat(path: &str) {
    let fd = open(path);
    if fd < 0 {
        println!("cat: cannot open {}: error {}", path, fd);
        return;
    }
    let mut buf = [0u8; 256];
    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        write(1, &buf[..n as usize]);
    }
    close(fd);
}

fn run_external(cwd: &Cwd, cmd: &str) {
    let mut path_buf = [0u8; MAX_PATH];
    // Bare names are looked up in /bin.
    let path: &str = if cmd.contains('/') {
        cwd.resolve(cmd, &mut path_buf)
    } else {
        let mut tmp = [0u8; MAX_PATH];
        tmp[..5].copy_from_slice(b"/bin/");
        let n = cmd.len().min(MAX_PATH - 6);
        tmp[5..5 + n].copy_from_slice(&cmd.as_bytes()[..n]);
        path_buf[..5 + n].copy_from_slice(&tmp[..5 + n]);
        core::str::from_utf8(&path_buf[..5 + n]).unwrap_or(cmd)
    };

    let pid = fork();
    if pid == 0 {
        exec(path);
        println!("sh: {}: command not found", cmd);
        exit(127);
    } else if pid > 0 {
        let mut status = 0;
        wait(&mut status);
        let code = status >> 8;
        if code != 0 {
            println!("sh: {} exited with code {}", cmd, code);
        }
    } else {
        println!("sh: fork failed");
    }
}

#[no_mangle]
extern "C" fn main() -> i32 {
    println!("LightOS sh v0.1 (pid {}) — type 'help'", getpid());
    let mut cwd = Cwd::new();
    let mut line = [0u8; MAX_LINE];

    loop {
        print!("lightos:{}$ ", cwd.as_str());
        let len = read_line(&mut line);
        let Ok(text) = core::str::from_utf8(&line[..len]) else {
            continue;
        };
        let mut parts = text.trim().splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();

        match cmd {
            "" => {}
            "help" => {
                println!("builtins: cd pwd ls cat echo help exit; anything else runs /bin/<cmd>")
            }
            "exit" => exit(0),
            "pwd" => println!("{}", cwd.as_str()),
            "echo" => println!("{}", arg),
            "cd" => {
                let target = if arg.is_empty() { "/" } else { arg };
                let mut resolved = [0u8; MAX_PATH];
                let path = cwd.resolve(target, &mut resolved);
                let fd = open_flags(path, O_DIRECTORY);
                if fd < 0 {
                    println!("cd: {}: not a directory", path);
                } else {
                    close(fd);
                    let bytes = path.as_bytes();
                    cwd.buf[..bytes.len()].copy_from_slice(bytes);
                    cwd.len = bytes.len();
                }
            }
            "ls" => {
                let target = if arg.is_empty() { cwd.as_str() } else { arg };
                let mut resolved = [0u8; MAX_PATH];
                // Manual borrow dance: resolve needs cwd by ref.
                let path = if target.starts_with('/') && arg.is_empty() {
                    target
                } else {
                    cwd.resolve(target, &mut resolved)
                };
                cmd_ls(path);
            }
            "cat" => {
                if arg.is_empty() {
                    println!("cat: missing operand");
                } else {
                    let mut resolved = [0u8; MAX_PATH];
                    let path = cwd.resolve(arg, &mut resolved);
                    cmd_cat(path);
                }
            }
            _ => run_external(&cwd, text.trim()),
        }
    }
}
