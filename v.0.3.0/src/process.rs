//! Layer 4 (B1 + B2a) — per-process Sv39 address spaces, a fixed two-slot
//! process table, and a cooperative frame-swap context switch.
//!
//! The switch primitive (yield_to_next) is deliberately dumb and ordered:
//!   (a) advance the OUTGOING sepc past its ecall,
//!   (b) copy the live on-stack TrapFrame OUT into the outgoing slot,
//!   (c) pick the next Ready slot (round-robin),
//!   (d) switch satp to the incoming root,
//!   (e) copy the incoming slot's frame INTO the on-stack TrapFrame.
//! The trap vector's existing restore path then resumes the incoming process
//! with zero asm changes. Fresh and resumed processes flow through the same
//! path because a never-run process's slot frame IS its initial entry frame.
//!
//! ⚠️ static mut TABLE/CURRENT are safe ONLY because we're single-hart with
//! interrupts off (same justification as ipc::ENDPOINT). The moment the Layer 4
//! scheduler arms timer interrupts or SMP arrives, these need a lock.
//!
//! Distributed-native: `home_node` + `ProcessState::Migrating` are here from
//! day one so Layer 7 live-migration reshapes nothing.

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W, PTE_X, PTE_A, PTE_D};
use crate::memory::frame::alloc_frame;
use crate::memory::PAGE_SIZE;
use crate::trap::TrapFrame;
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const USER_CODE_VA:  usize = 0x0040_0000; // 4 MiB
pub const USER_STACK_VA: usize = 0x0080_0000; // 8 MiB

pub const MAX_PROCS: usize = 2;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready, Running, Blocked, Dead,
    Migrating, // distributed: reserved for Layer 7 live migration
}

#[allow(dead_code)]
pub struct Process {
    pub pid: usize,
    pub frame: TrapFrame, // saved register state (our real Layer 1 frame)
    pub root: usize,      // Sv39 root PA for satp
    pub state: ProcessState,
    pub home_node: u32,   // distributed: owning node (0 = this node)
}

static NEXT_PID: AtomicUsize = AtomicUsize::new(1);

// Single-hart + interrupts off => plain static mut is sound (see module doc).
// Accessed only via &raw mut to stay clear of the static_mut_refs footgun.
static mut TABLE: [Option<Process>; MAX_PROCS] = [None, None];
static mut CURRENT: usize = 0;

impl Process {
    /// Create a user process in its own address space from a position-
    /// independent code image `[code_src, code_src+code_len)` in kernel .text.
    pub unsafe fn new_user(code_src: usize, code_len: usize) -> Self {
        let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);

        // Fresh root with the kernel replicated in (so traps work under it).
        let root = paging::create_root();

        // Code frame: copy the image in, sync I-stream, map U+R+X (W^X: no write).
        let code_pa = alloc_frame().expect("process: OOM code frame");
        core::ptr::copy_nonoverlapping(code_src as *const u8, code_pa as *mut u8, code_len);
        asm!("fence.i", options(nostack)); // instructions written via data path
        paging::map_4k(root, USER_CODE_VA, code_pa, PTE_U | PTE_R | PTE_X | PTE_A);

        // Stack frame: zero, map U+R+W (no X).
        let stack_pa = alloc_frame().expect("process: OOM stack frame");
        core::ptr::write_bytes(stack_pa as *mut u8, 0, PAGE_SIZE);
        paging::map_4k(root, USER_STACK_VA, stack_pa, PTE_U | PTE_R | PTE_W | PTE_A | PTE_D);

        // Initial register state: entry PC + top-of-stack sp.
        let mut frame = TrapFrame::zero();
        frame.sepc = USER_CODE_VA;
        frame.regs[1] = USER_STACK_VA + PAGE_SIZE; // x2 / sp

        // ⚠️ CRITICAL: the trap vector's restore path does `csrw sstatus`
        // straight from frame.sstatus. A zeroed sstatus srets with UXL=0
        // (reserved encoding) - do not rely on WARL to save us. Initialise it
        // exactly like enter_user_mode: live sstatus, SPP=0 (return to U),
        // SPIE=0 (ints off after sret). Fresh-via-vector == fresh-via-
        // enter_user_mode, bit for bit.
        let mut sstatus: usize;
        asm!("csrr {s}, sstatus", s = out(reg) sstatus);
        sstatus &= !(1usize << 8); // SPP  = 0
        sstatus &= !(1usize << 5); // SPIE = 0
        frame.sstatus = sstatus;

        crate::kprintln!(
            "[L4] process pid={} created: root@{:#x} code_pa={:#x} stack_pa={:#x}",
            pid, root, code_pa, stack_pa
        );
        Process { pid, frame, root, state: ProcessState::Ready, home_node: 0 }
    }
}

/// Stage a process into the first free table slot. Returns its pid.
pub unsafe fn spawn(code_src: usize, code_len: usize) -> usize {
    let p = Process::new_user(code_src, code_len);
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

/// Enter the process in `slot` for the first time. Never returns (control
/// comes back only via a trap). Everything AFTER this first entry flows
/// through the vector's save/restore path via yield_to_next.
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

/// The B2a crux: cooperative switch, called from the SYS_YIELD arm with the
/// handler's &mut to the LIVE on-stack TrapFrame. See module doc for the
/// ordered (a)-(e) dance. Copy direction matters: out-then-in, never reversed.
pub fn yield_to_next(frame: &mut TrapFrame) {
    unsafe {
        let table = &mut *(&raw mut TABLE);
        let cur = *(&raw const CURRENT);

        // (a) advance the OUTGOING process past its ecall BEFORE saving, so it
        //     resumes at the instruction after `ecall` when next scheduled.
        //     (The dispatch arm early-returns, so the shared +4 never runs.)
        frame.sepc += 4;

        // (b) save the live frame OUT into the outgoing slot.
        let cur_pid = if let Some(p) = table[cur].as_mut() {
            p.frame = *frame;
            if p.state == ProcessState::Running { p.state = ProcessState::Ready; }
            p.pid
        } else { 0 };

        // (c) round-robin: first Ready slot after cur (wraps; cur itself is
        //     checked last, so a sole runnable process just resumes itself).
        let mut next = cur;
        for i in 1..=MAX_PROCS {
            let cand = (cur + i) % MAX_PROCS;
            if let Some(p) = table[cand].as_ref() {
                if p.state == ProcessState::Ready { next = cand; break; }
            }
        }

        *(&raw mut CURRENT) = next;
        let np = table[next].as_mut().expect("yield: no runnable process");
        np.state = ProcessState::Running;
        crate::kprintln!("[L4] yield: pid {} -> pid {}", cur_pid, np.pid);

        // (d) switch address spaces. Safe mid-handler: the trap stack and
        //     handler text are replicated (S-only) into every root, so the
        //     ground doesn't move under us when satp flips.
        paging::switch_to(np.root);

        // (e) restore the incoming frame INTO the live on-stack frame. The
        //     vector's normal restore path then srets as the incoming process,
        //     re-arming sscratch from its SPP=0 automatically.
        *frame = np.frame;
    }
}
