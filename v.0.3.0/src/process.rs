//! Layer 4 (step B1) — a Process with its OWN Sv39 address space.
//!
//! Each process gets a fresh root (kernel replicated in, S-only) plus its own
//! U-mapped code/stack. Same user VA in different processes backs DIFFERENT
//! physical frames -> real isolation. A context switch = save regs, switch_to
//! the next root (satp + sfence), restore regs, sret. B1 proves the per-process
//! root + satp switch with ONE process; B2 adds a second and switches between.
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

        crate::kprintln!(
            "[L4] process pid={} created: root@{:#x} code_pa={:#x} stack_pa={:#x}",
            pid, root, code_pa, stack_pa
        );
        Process { pid, frame, root, state: ProcessState::Ready, home_node: 0 }
    }
}

/// Switch satp to `p`'s address space and enter it in U-mode. Never returns
/// (control comes back only via a trap). B1's single-process entry point.
pub fn run_first(p: &Process) -> ! {
    unsafe { paging::switch_to(p.root); }
    crate::kprintln!(
        "[L4] pid={} entering U-mode under its OWN root (satp switched) ...",
        p.pid
    );
    crate::trap::enter_user_mode(&p.frame)
}
