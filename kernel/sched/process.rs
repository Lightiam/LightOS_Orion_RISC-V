//! Process Control Blocks, the process table, and lifecycle operations
//! (spawn/fork/exec/exit/wait).
//!
//! Concurrency model: single scheduling hart, kernel non-preemptible
//! (supervisor interrupts stay masked inside the kernel). Blocking
//! syscalls suspend by parking the process and jumping to the
//! scheduler; whoever unblocks them writes the syscall result directly
//! into the sleeper's trap frame.
#![allow(unsafe_code)] // trap-frame array access and page-table plumbing

use crate::elf;
use crate::fs;
use crate::lock::SpinLock;
use crate::mem::layout::PAGE_SIZE;
use crate::mem::mmu::{self, PageTable, PTE_R, PTE_U, PTE_W};
use crate::mem::{page, uaccess};
use crate::sched::{self, CURRENT};
use crate::trap::context::TrapFrame;
use crate::uart_println;
use core::cell::UnsafeCell;
use core::sync::atomic::Ordering;

pub const MAX_PROCS: usize = 64;
/// Slot index meaning "no process".
pub const NO_PROC: usize = usize::MAX;

pub const USER_STACK_TOP: usize = 0x7fff_f000;
const USER_STACK_PAGES: usize = 16;
const MMAP_BASE: usize = 0x5000_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcState {
    Ready,
    Running,
    /// Blocked in wait4 until a child exits.
    SleepChild,
    /// Blocked in read(0) until console input arrives.
    SleepConsole,
    /// Exited; PCB retained until the parent reaps it.
    Zombie,
}

impl ProcState {
    /// Short label for `ps`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcState::Ready => "ready",
            ProcState::Running => "run",
            ProcState::SleepChild => "wait",
            ProcState::SleepConsole => "sleep",
            ProcState::Zombie => "zombie",
        }
    }
}

/// What an fd >= 3 refers to.
#[derive(Clone, Copy)]
pub enum FdKind {
    /// Regular file or directory on the root filesystem.
    File { ino: u32 },
    /// NCE character device (/dev/nceN).
    Nce { slot: usize },
}

/// An open descriptor (fds 0-2 are the console and have no entry
/// here; fds from 3 index this table).
#[derive(Clone, Copy)]
pub struct OpenFile {
    pub kind: FdKind,
    /// Byte offset for files, entry index for directories.
    pub pos: usize,
}

pub struct Process {
    pub pid: usize,
    pub state: ProcState,
    /// Slot index of the parent (NO_PROC for init).
    pub parent: usize,
    /// Physical address of this process's root page table.
    pub root_pa: usize,
    pub exit_code: i32,
    /// wait4: user address to store the wstatus at on wake-up.
    pub wait_status_uva: usize,
    /// read(0): destination buffer while blocked on console input.
    pub read_buf_uva: usize,
    pub read_len: usize,
    pub next_mmap: usize,
    pub brk: usize,
    pub name: [u8; 16],
    /// Open files, indexed by fd - 3.
    pub files: alloc::vec::Vec<Option<OpenFile>>,
    /// NCE affinity hint mask (bit n = wants NCE slot n proximity).
    pub nce_affinity: usize,
}

pub struct ProcTable {
    pub slots: [Option<Process>; MAX_PROCS],
    next_pid: usize,
    pub last_run: usize,
}

/// The process table. Lock invariant: guards all PCB state; never held
/// across a return to user mode or an idle wait.
pub static PROCS: SpinLock<ProcTable> = SpinLock::new(ProcTable {
    slots: [const { None }; MAX_PROCS],
    next_pid: 1,
    last_run: 0,
});

/// Trap frames live outside the lock so the trap trampoline can write
/// them via sscratch without taking it. Exclusive access is guaranteed
/// by the single-hart, kernel-non-preemptible design: a frame is only
/// touched by its own trap path or by a syscall completing on behalf
/// of a *sleeping* process.
struct TrapFrames(UnsafeCell<[TrapFrame; MAX_PROCS]>);
unsafe impl Sync for TrapFrames {}
static TRAPFRAMES: TrapFrames = TrapFrames(UnsafeCell::new([TrapFrame::zeroed(); MAX_PROCS]));

#[allow(clippy::mut_from_ref)]
pub fn tf_mut(slot: usize) -> &'static mut TrapFrame {
    unsafe { &mut (*TRAPFRAMES.0.get())[slot] }
}

fn kernel_stack_top() -> usize {
    // Hart 0's boot stack; every trap re-enters at the top since the
    // kernel never sleeps mid-stack.
    crate::mem::layout::stack_start() + 64 * 1024
}

fn set_name(dst: &mut [u8; 16], name: &str) {
    let n = name.len().min(15);
    dst[..n].copy_from_slice(&name.as_bytes()[..n]);
    dst[n] = 0;
}

/// Build a fresh user address space for `image` (+ user stack) and
/// return (root_pa, entry, brk).
fn build_address_space(image: &[u8]) -> Result<(usize, usize, usize), &'static str> {
    let root = mmu::new_user_root().ok_or("proc: out of memory for root table")?;
    let loaded = elf::load(image, root)?;
    let stack_base = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
    for i in 0..USER_STACK_PAGES {
        let frame = page::page_alloc(1).ok_or("proc: out of memory for stack")? as usize;
        root.map(stack_base + i * PAGE_SIZE, frame, PTE_U | PTE_R | PTE_W)?;
    }
    Ok((root as *mut PageTable as usize, loaded.entry, loaded.brk))
}

/// Create a new process from a program image (used for init).
pub fn spawn(name: &str) -> Result<usize, &'static str> {
    let image = fs::load_program(name).ok_or("proc: no such program")?;
    let (root_pa, entry, brk) = build_address_space(&image)?;

    let mut procs = PROCS.lock();
    let slot = procs
        .slots
        .iter()
        .position(|s| s.is_none())
        .ok_or("proc: process table full")?;
    let pid = procs.next_pid;
    procs.next_pid += 1;

    let tf = tf_mut(slot);
    *tf = TrapFrame::zeroed();
    tf.sepc = entry;
    tf.regs[2] = USER_STACK_TOP; // sp
    tf.kernel_sp = kernel_stack_top();

    let mut name_buf = [0u8; 16];
    set_name(&mut name_buf, name);
    procs.slots[slot] = Some(Process {
        pid,
        state: ProcState::Ready,
        parent: NO_PROC,
        root_pa,
        exit_code: 0,
        wait_status_uva: 0,
        read_buf_uva: 0,
        read_len: 0,
        next_mmap: MMAP_BASE,
        brk,
        name: name_buf,
        files: alloc::vec::Vec::new(),
        nce_affinity: 0,
    });
    Ok(pid)
}

/// fork(): duplicate the current process. Returns the child pid to the
/// parent; the child resumes with a0 = 0.
pub fn fork_current(tf: &TrapFrame) -> isize {
    let cur = CURRENT.load(Ordering::Relaxed);
    let mut procs = PROCS.lock();

    let Some(slot) = procs.slots.iter().position(|s| s.is_none()) else {
        return -11; // EAGAIN
    };
    let (parent_root, next_mmap, brk, name, files) = {
        let p = procs.slots[cur].as_ref().expect("fork: no current process");
        (p.root_pa, p.next_mmap, p.brk, p.name, p.files.clone())
    };

    let Some(child_root) = mmu::new_user_root() else {
        return -12; // ENOMEM
    };
    if let Err(e) = mmu::clone_user_space(mmu::table_at(parent_root), child_root) {
        mmu::free_user_space(child_root);
        page::page_free(child_root as *mut PageTable as *mut u8, 1);
        uart_println!("fork: {}", e);
        return -12;
    }

    let pid = procs.next_pid;
    procs.next_pid += 1;

    let child_tf = tf_mut(slot);
    *child_tf = *tf;
    child_tf.set_a0(0); // child's fork() return value
    child_tf.kernel_sp = kernel_stack_top();

    procs.slots[slot] = Some(Process {
        pid,
        state: ProcState::Ready,
        parent: cur,
        root_pa: child_root as *mut PageTable as usize,
        exit_code: 0,
        wait_status_uva: 0,
        read_buf_uva: 0,
        read_len: 0,
        next_mmap,
        brk,
        name,
        files,
        nce_affinity: 0,
    });
    pid as isize
}

/// execve(): replace the current image. Builds the new address space
/// first so a failed exec leaves the caller intact.
pub fn exec_current(tf: &mut TrapFrame, name: &str) -> isize {
    let Some(image) = fs::load_program(name) else {
        return -2; // ENOENT
    };
    let (new_root, entry, brk) = match build_address_space(&image) {
        Ok(v) => v,
        Err(e) => {
            uart_println!("exec {:?}: {}", name, e);
            return -12; // ENOMEM
        }
    };

    let cur = CURRENT.load(Ordering::Relaxed);
    let mut procs = PROCS.lock();
    let p = procs.slots[cur].as_mut().expect("exec: no current process");

    let old_root = mmu::table_at(p.root_pa);
    mmu::free_user_space(old_root);
    page::page_free(old_root as *mut PageTable as *mut u8, 1);

    p.root_pa = new_root;
    p.next_mmap = MMAP_BASE;
    p.brk = brk;
    set_name(&mut p.name, name);

    *tf = TrapFrame {
        kernel_sp: kernel_stack_top(),
        ..TrapFrame::zeroed()
    };
    tf.sepc = entry;
    tf.regs[2] = USER_STACK_TOP;
    0
}

/// exit(): free the address space, go zombie, hand children to init,
/// complete the parent's pending wait4 if there is one. Never returns.
pub fn exit_current(code: i32) -> ! {
    let cur = CURRENT.load(Ordering::Relaxed);
    {
        let mut procs = PROCS.lock();

        let (pid, root_pa, parent) = {
            let p = procs.slots[cur].as_ref().expect("exit: no current process");
            (p.pid, p.root_pa, p.parent)
        };
        if pid == 1 {
            panic!("init (PID 1) exited with code {}", code);
        }

        mmu::free_user_space(mmu::table_at(root_pa));

        // Orphans are re-parented to init (slot of pid 1).
        let init_slot = procs
            .slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|p| p.pid == 1));
        for p in procs.slots.iter_mut().flatten() {
            if p.parent == cur {
                p.parent = init_slot.unwrap_or(NO_PROC);
            }
        }

        {
            let p = procs.slots[cur].as_mut().expect("exit: no current process");
            p.state = ProcState::Zombie;
            p.exit_code = code;
        }

        // If the parent is already blocked in wait4, complete it now.
        if parent != NO_PROC {
            if let Some(parent_proc) = procs.slots[parent].as_ref() {
                if parent_proc.state == ProcState::SleepChild {
                    complete_wait(&mut procs, parent, cur);
                }
            }
        }
    }
    CURRENT.store(NO_PROC, Ordering::Relaxed);
    sched::schedule()
}

/// Reap `child_slot` (a Zombie) on behalf of the sleeping/waiting
/// `parent_slot`: deliver pid + wstatus and free the PCB.
fn complete_wait(procs: &mut ProcTable, parent_slot: usize, child_slot: usize) {
    let (child_pid, code, child_root) = {
        let c = procs.slots[child_slot].as_ref().expect("reap: no child");
        (c.pid, c.exit_code, c.root_pa)
    };
    let status_uva = {
        let p = procs.slots[parent_slot].as_mut().expect("reap: no parent");
        p.state = ProcState::Ready;
        p.wait_status_uva
    };

    if status_uva != 0 {
        let parent_root = procs.slots[parent_slot].as_ref().expect("reap").root_pa;
        let wstatus = (code & 0xff) << 8;
        let _ = uaccess::copy_out(
            mmu::table_at(parent_root),
            status_uva,
            &wstatus.to_le_bytes(),
        );
    }

    tf_mut(parent_slot).set_a0(child_pid);
    page::page_free(child_root as *mut u8, 1); // root table page
    procs.slots[child_slot] = None;
}

/// wait4(): reap a zombie child now, or block until one exits.
/// Returns to user only via the scheduler. Never returns.
pub fn wait_current(status_uva: usize) -> ! {
    let cur = CURRENT.load(Ordering::Relaxed);
    {
        let mut procs = PROCS.lock();

        let mut have_child = false;
        let mut zombie = None;
        for (i, slot) in procs.slots.iter().enumerate() {
            if let Some(p) = slot {
                if p.parent == cur {
                    have_child = true;
                    if p.state == ProcState::Zombie {
                        zombie = Some(i);
                        break;
                    }
                }
            }
        }

        let p = procs.slots[cur].as_mut().expect("wait: no current process");
        p.wait_status_uva = status_uva;

        if let Some(child_slot) = zombie {
            // complete_wait expects the parent in SleepChild.
            p.state = ProcState::SleepChild;
            complete_wait(&mut procs, cur, child_slot);
        } else if have_child {
            p.state = ProcState::SleepChild;
            // Sleep; a child's exit_current completes the wait.
        } else {
            p.state = ProcState::Ready;
            tf_mut(cur).set_a0(-10_isize as usize); // ECHILD
        }
    }
    CURRENT.store(NO_PROC, Ordering::Relaxed);
    sched::schedule()
}

/// Console input arrived: if a process is blocked in read(0), complete
/// its read with whatever bytes are buffered. Called from the UART IRQ
/// path.
pub fn wake_console_reader() {
    let mut procs = PROCS.lock();
    let Some(slot) = procs.slots.iter().position(|s| {
        s.as_ref()
            .is_some_and(|p| p.state == ProcState::SleepConsole)
    }) else {
        return;
    };

    let (root_pa, buf_uva, len) = {
        let p = procs.slots[slot].as_ref().expect("wake: no sleeper");
        (p.root_pa, p.read_buf_uva, p.read_len)
    };

    let mut tmp = [0u8; 128];
    let mut n = 0;
    while n < len.min(tmp.len()) {
        match crate::uart::read_byte() {
            Some(b) => {
                tmp[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    if n == 0 {
        return;
    }

    if uaccess::copy_out(mmu::table_at(root_pa), buf_uva, &tmp[..n]).is_ok() {
        tf_mut(slot).set_a0(n);
    } else {
        tf_mut(slot).set_a0(-14_isize as usize); // EFAULT
    }
    let p = procs.slots[slot].as_mut().expect("wake: no sleeper");
    p.state = ProcState::Ready;
}

/// Block the current process on console input. Never returns.
pub fn block_on_console(buf_uva: usize, len: usize) -> ! {
    let cur = CURRENT.load(Ordering::Relaxed);
    {
        let mut procs = PROCS.lock();
        let p = procs.slots[cur].as_mut().expect("read: no current process");
        p.state = ProcState::SleepConsole;
        p.read_buf_uva = buf_uva;
        p.read_len = len;
    }
    CURRENT.store(NO_PROC, Ordering::Relaxed);
    sched::schedule()
}

/// Number of live processes (any non-empty slot).
pub fn count_active() -> usize {
    PROCS.lock().slots.iter().filter(|s| s.is_some()).count()
}

/// Format a `ps`-style listing of all processes into a heap string.
pub fn listing() -> alloc::string::String {
    use core::fmt::Write;
    let procs = PROCS.lock();
    let mut out = alloc::string::String::new();
    let _ = writeln!(out, "  PID  PPID  STATE   NAME");
    for p in procs.slots.iter().flatten() {
        let name_end = p.name.iter().position(|&b| b == 0).unwrap_or(p.name.len());
        let name = core::str::from_utf8(&p.name[..name_end]).unwrap_or("?");
        let ppid = if p.parent == NO_PROC {
            0
        } else {
            procs.slots[p.parent].as_ref().map_or(0, |pp| pp.pid)
        };
        let _ = writeln!(
            out,
            "{:>5} {:>5}  {:<7} {}",
            p.pid,
            ppid,
            p.state.as_str(),
            name
        );
    }
    out
}

/// (root_pa, pid) of the current process.
pub fn current_info() -> (usize, usize) {
    let cur = CURRENT.load(Ordering::Relaxed);
    let procs = PROCS.lock();
    let p = procs.slots[cur].as_ref().expect("no current process");
    (p.root_pa, p.pid)
}
