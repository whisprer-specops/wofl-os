//! Layer 4 — per-process Sv39 address spaces, two-slot process table,
//! cooperative switching, real exit + reclaim, and BLOCKING primitives.
//!
//! Three variations on one frame dance, differing ONLY in how the outgoing
//! process is treated:
//!   * yield_to_next:  sepc += 4, save frame, state = Ready   (resume AFTER ecall)
//!   * block_current:  NO +4,     save frame, state = Blocked (resume AT ecall:
//!                     the syscall RESTARTS on wake - recv retries and finds data)
//!   * exit_current:   no save at all, state = Dead, address space reclaimed
//! All three then: pick next Ready, switch satp, copy its frame into the live
//! on-stack TrapFrame, early-return from dispatch (shared +4 must never touch
//! the freshly restored incoming frame).
//!
//! ⚠️ static mut TABLE/CURRENT are safe ONLY because single-hart + interrupts
//! off (same justification as ipc::ENDPOINT). Lock them when preemption/SMP
//! arrive.
//!
//! Distributed-native: `home_node` + `ProcessState::Migrating` reserved for
//! Layer 7 live migration.

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W, PTE_X, PTE_A, PTE_D};
use crate::memory::frame::alloc_frame;
use crate::memory::PAGE_SIZE;
use crate::trap::TrapFrame;
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const USER_CODE_VA:  usize = 0x0040_0000; // 4 MiB
pub const USER_STACK_VA: usize = 0x0080_0000; // 8 MiB

/// A loadable userspace program: a flat blob (objcopy output) plus the page-
/// aligned section layout extracted from the linker script by build.sh. Every
/// program links at USER_CODE_VA; per-process roots keep same-VA regions in
/// DIFFERENT physical frames, so programs never collide despite shared VAs.
pub struct UserImage {
    pub blob: &'static [u8],
    pub text_va: usize,   pub text_len: usize,
    pub rodata_va: usize, pub rodata_len: usize,
    pub data_va: usize,   pub data_len: usize,
    pub bss_va: usize,    pub bss_len: usize,
    pub entry: usize,
}

// Compile-time contract: the loader computes blob offsets as (VA-USER_CODE_VA),
// so the linker's USER_BASE MUST equal USER_CODE_VA. Caught at BUILD, not boot.
const _: () = assert!(USER_CODE_VA == crate::user_layout::USER_BASE);

pub const MAX_PROCS: usize = 4;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready, Running, Blocked, Dead,
    Migrating, // distributed: reserved for Layer 7 live migration
}

#[allow(dead_code)]
pub struct Process {
    pub pid: usize,
    pub frame: TrapFrame, // saved register state
    pub root: usize,      // Sv39 root PA for satp
    pub state: ProcessState,
    pub home_node: u32,   // distributed: owning node (0 = this node)
    pub blocked_ep: usize,// which endpoint this process is Blocked waiting on
}

static NEXT_PID: AtomicUsize = AtomicUsize::new(1);

static mut TABLE: [Option<Process>; MAX_PROCS] = [None, None, None, None];
static mut CURRENT: usize = 0;

/// Total involuntary (timer) context switches - the preemption proof metric.
static TICK_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Map a file-backed region: copy blob bytes into fresh U-pages with `flags`.
/// blob offset = va - USER_CODE_VA (the validated flat-binary contract). `len`
/// is a page-multiple; len==0 maps nothing (empty section).
unsafe fn map_file_region(root: usize, img: &UserImage, va: usize, len: usize, flags: usize) {
    let mut off = 0;
    while off < len {
        let pa = alloc_frame().expect("process: OOM region frame");
        let file_off = (va - USER_CODE_VA) + off;
        core::ptr::copy_nonoverlapping(
            img.blob.as_ptr().add(file_off), pa as *mut u8, PAGE_SIZE);
        paging::map_4k(root, va + off, pa, PTE_U | flags | PTE_A);
        off += PAGE_SIZE;
    }
}

/// Map a zero-filled region (.bss): fresh zeroed U-pages, no blob source.
unsafe fn map_zero_region(root: usize, va: usize, len: usize, flags: usize) {
    let mut off = 0;
    while off < len {
        let pa = alloc_frame().expect("process: OOM bss frame");
        core::ptr::write_bytes(pa as *mut u8, 0, PAGE_SIZE);
        paging::map_4k(root, va + off, pa, PTE_U | flags | PTE_A);
        off += PAGE_SIZE;
    }
}

impl Process {
    /// Create a user process in its own address space from a position-
    /// independent code image `[code_src, code_src+code_len)` in kernel .text.
    pub unsafe fn new_user(img: &UserImage) -> Self {
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);

        let root = paging::create_root();

        // Per-section W^X mapping. .text R+X, .rodata R, .data/.bss R+W.
        map_file_region(root, img, img.text_va,   img.text_len,   PTE_R | PTE_X);
        map_file_region(root, img, img.rodata_va, img.rodata_len, PTE_R);
        map_file_region(root, img, img.data_va,   img.data_len,   PTE_R | PTE_W | PTE_D);
        map_zero_region(root, img.bss_va, img.bss_len,            PTE_R | PTE_W | PTE_D);

        // .text was written via the data path - sync the I-stream before it runs.
        asm!("fence.i", options(nostack));

        // User stack: one zeroed R+W page.
        let stack_pa = alloc_frame().expect("process: OOM stack frame");
        core::ptr::write_bytes(stack_pa as *mut u8, 0, PAGE_SIZE);
        paging::map_4k(root, USER_STACK_VA, stack_pa, PTE_U | PTE_R | PTE_W | PTE_A | PTE_D);

        let mut frame = TrapFrame::zero();
        frame.sepc = img.entry;
        frame.regs[1] = USER_STACK_VA + PAGE_SIZE; // x2 / sp

        // ⚠️ CRITICAL: the vector's restore does `csrw sstatus` straight from
        // frame.sstatus; a zeroed one srets with UXL=0 (reserved). Initialise
        // exactly like enter_user_mode: live sstatus, SPP=0, SPIE=0.
        let mut sstatus: usize;
        asm!("csrr {s}, sstatus", s = out(reg) sstatus);
        sstatus &= !(1usize << 8); // SPP  = 0
        sstatus &= !(1usize << 5); // SPIE = 0
        frame.sstatus = sstatus;

        crate::kprintln!(
            "[L4] process pid={} loaded: root@{:#x} entry={:#x} text={}B rodata={}B data={}B bss={}B",
            pid, root, img.entry, img.text_len, img.rodata_len, img.data_len, img.bss_len
        );
        Process { pid, frame, root, state: ProcessState::Ready, home_node: 0, blocked_ep: 0 }
    }
}

/// Stage a process into the first free table slot. Returns its pid.
pub unsafe fn spawn(img: &UserImage) -> usize {
    let p = Process::new_user(img);
    let pid = p.pid;
    let table = &mut *(&raw mut TABLE);
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(p);
            return pid;
        }
    }
    panic!("process table full");
}

/// Enter the process in `slot` for the first time. Never returns.
pub unsafe fn run_first(slot: usize) -> ! {
    let table = &mut *(&raw mut TABLE);
    *(&raw mut CURRENT) = slot;
    let p = table[slot].as_mut().expect("run_first: empty slot");
    p.state = ProcessState::Running;
    paging::switch_to(p.root);
    crate::kprintln!(
        "[L4] pid={} entering U-mode under its OWN root (satp switched) ...",
        p.pid
    );
    crate::trap::enter_user_mode(&p.frame)
}

/// Round-robin: first Ready slot after `cur` (wraps; `cur` checked last, so a
/// sole runnable process resumes itself on yield). Shared by yield/block/exit.
unsafe fn pick_next_ready(cur: usize) -> Option<usize> {
    let table = &*(&raw const TABLE);
    for i in 1..=MAX_PROCS {
        let cand = (cur + i) % MAX_PROCS;
        if let Some(p) = table[cand].as_ref() {
            if p.state == ProcessState::Ready { return Some(cand); }
        }
    }
    None
}

/// Switch INTO slot `next`: satp first, then copy its frame over the live
/// on-stack frame. The vector's restore path does the rest.
unsafe fn switch_into(frame: &mut TrapFrame, next: usize) {
    let table = &mut *(&raw mut TABLE);
    *(&raw mut CURRENT) = next;
    let np = table[next].as_mut().expect("switch_into: empty slot");
    np.state = ProcessState::Running;
    paging::switch_to(np.root);
    *frame = np.frame;
}

/// Cooperative yield: resume-point is AFTER the ecall (+4 before save).
pub fn yield_to_next(frame: &mut TrapFrame) {
    unsafe {
        let table = &mut *(&raw mut TABLE);
        let cur = *(&raw const CURRENT);

        frame.sepc += 4; // outgoing resumes past its ecall
        let cur_pid = if let Some(p) = table[cur].as_mut() {
            p.frame = *frame;
            if p.state == ProcessState::Running { p.state = ProcessState::Ready; }
            p.pid
        } else { 0 };

        let next = pick_next_ready(cur).unwrap_or(cur); // worst case: self
        crate::kprintln!("[L4] yield: pid {} -> pid {}",
            cur_pid, table[next].as_ref().map(|p| p.pid).unwrap_or(0));
        switch_into(frame, next);
    }
}

/// Timer preemption: the fourth frame dance. Distinguishing feature: the
/// outgoing frame is saved EXACTLY as trapped - no sepc adjustment, because an
/// interrupt's sepc already points AT the instruction to resume. (Contrast:
/// yield = +4 first; block = no +4 but restart-by-design; preempt = untouched.)
/// If nobody else is Ready the process just resumes itself (quantum renewed).
pub fn preempt(frame: &mut TrapFrame) {
    TICK_COUNT.fetch_add(1, Ordering::Relaxed);
    unsafe {
        let table = &mut *(&raw mut TABLE);
        let cur = *(&raw const CURRENT);

        let cur_pid = if let Some(p) = table[cur].as_mut() {
            p.frame = *frame;
            if p.state == ProcessState::Running { p.state = ProcessState::Ready; }
            p.pid
        } else { 0 };

        let next = pick_next_ready(cur).unwrap_or(cur);
        if next != cur {
            crate::kprintln!("[L4] tick: preempt pid {} -> pid {}",
                cur_pid, table[next].as_ref().map(|p| p.pid).unwrap_or(0));
        }
        switch_into(frame, next);
    }
}

/// Block the caller (e.g. recv on empty queue): NO +4 - the saved sepc points
/// AT the ecall, so on wake the syscall re-executes and retries. If nothing is
/// Ready, every live process is Blocked -> deadlock, diagnosed loudly.
///
/// ⚠️ CONTRACT: on return, `frame` holds the NEXT process. The caller must
/// touch NOTHING afterwards - no return value writes, no error encodes.
pub fn block_current(frame: &mut TrapFrame, ep: usize) {
    unsafe {
        let table = &mut *(&raw mut TABLE);
        let cur = *(&raw const CURRENT);

        // NO frame.sepc += 4 - restart semantics. Record WHICH endpoint we wait
        // on so wake_endpoint() only revives us when THAT endpoint gets a message.
        let cur_pid = if let Some(p) = table[cur].as_mut() {
            p.frame = *frame;
            p.state = ProcessState::Blocked;
            p.blocked_ep = ep;
            p.pid
        } else { 0 };

        match pick_next_ready(cur) {
            Some(next) => {
                crate::kprintln!("[L4] block: pid {} on ep {} -> pid {}",
                    cur_pid, ep, table[next].as_ref().map(|p| p.pid).unwrap_or(0));
                switch_into(frame, next);
            }
            None => {
                crate::kprintln!("");
                crate::kprintln!("[L4] DEADLOCK: all processes blocked, none runnable - halting");
                loop { asm!("wfi", options(nostack)); }
            }
        }
    }
}

/// Wake every Blocked process (single shared endpoint -> wake-all is correct:
/// woken receivers RETRY the syscall; one wins the message, others re-block).
pub fn tick_count() -> usize { TICK_COUNT.load(Ordering::Relaxed) }

pub fn wake_endpoint(ep: usize) {
    unsafe {
        let table = &mut *(&raw mut TABLE);
        for slot in table.iter_mut() {
            if let Some(p) = slot.as_mut() {
                if p.state == ProcessState::Blocked && p.blocked_ep == ep {
                    crate::kprintln!("[L4] wake: pid {} Ready (ep {} has a message)", p.pid, ep);
                    p.state = ProcessState::Ready;
                }
            }
        }
    }
}

/// Real process exit: mark Dead, reclaim the address space, schedule the next
/// Ready process - or diagnose the end-state: all Dead = clean idle, any
/// Blocked = deadlock (nobody left to ever send). No sepc adjustment anywhere.
pub fn exit_current(frame: &mut TrapFrame, code: usize) {
    unsafe {
        let table = &mut *(&raw mut TABLE);
        let cur = *(&raw const CURRENT);

        let (cur_pid, dead_root) = if let Some(p) = table[cur].as_mut() {
            p.state = ProcessState::Dead;
            (p.pid, p.root)
        } else { (0, 0) };
        crate::kprintln!("[L4] pid {} exited (code: {})", cur_pid, code);

        match pick_next_ready(cur) {
            Some(next) => {
                crate::kprintln!("[L4] exit: scheduling pid {}",
                    table[next].as_ref().map(|p| p.pid).unwrap_or(0));
                switch_into(frame, next);
                // satp now = next's root: safe to demolish the dead space.
                if dead_root != 0 {
                    paging::destroy_root(dead_root);
                    table[cur] = None;
                    let (used, total) = crate::memory::frame::get_stats();
                    crate::kprintln!("[L4] pid {} address space reclaimed ({}/{} frames in use)",
                        cur_pid, used, total);
                }
            }
            None => {
                // Leave the dying space FIRST - never free tables satp walks.
                paging::switch_to(paging::kernel_root());
                if dead_root != 0 {
                    paging::destroy_root(dead_root);
                    table[cur] = None;
                }
                // Distinguish the end-states.
                let any_blocked = table.iter().any(|s|
                    matches!(s.as_ref().map(|p| p.state), Some(ProcessState::Blocked)));
                if any_blocked {
                    crate::kprintln!("");
                    crate::kprintln!("[L4] DEADLOCK: last runnable process exited with receivers still blocked - halting");
                } else {
                    let (used, total) = crate::memory::frame::get_stats();
                    crate::kprintln!("[L4] all address spaces reclaimed ({}/{} frames in use)",
                        used, total);
                    crate::kprintln!("[L4] {} involuntary timer preemptions occurred",
                        tick_count());
                    crate::kprintln!("");
                    crate::kprintln!("*** woflOS: all processes exited - kernel outlived its children ***");
                    crate::kprintln!("Layer 4: exit, reclaim, blocking recv + wake. System idle.");
                }
                loop { asm!("wfi", options(nostack)); }
            }
        }
    }
}
