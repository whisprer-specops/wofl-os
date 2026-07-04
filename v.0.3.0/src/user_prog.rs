//! Layer 2 — the first *properly mapped* user program.
//!
//! Layer 1 cheated: it ran `user_main` (a kernel .text function) in U-mode,
//! only because OpenSBI's PMP left every page U-accessible. Now that Sv39 is on
//! with no U bit on kernel pages, that trick faults. So we do it right:
//!
//!   1. The user program is a self-contained, position-independent asm stub
//!      (only immediates + ecall + a PC-relative branch -> zero relocations),
//!      so copying it to an arbitrary physical frame and running it at an
//!      arbitrary user VA is guaranteed correct.
//!   2. We allocate physical frames for its code and stack, copy the stub in,
//!      and map them at user VAs with the U bit set (code R+X, stack R+W).
//!   3. We enter U-mode at the code VA with sp = top of the mapped stack.
//!
//! The ecall round-trip then runs through genuine page tables: same syscall
//! path as Layer 1, but now under enforced memory protection.

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W, PTE_X, PTE_A, PTE_D};
use crate::memory::frame::alloc_frame;
use crate::memory::PAGE_SIZE;
use core::arch::asm;

// User virtual addresses. Chosen in the low VA space, well clear of the
// kernel's identity-mapped DRAM (0x8000_0000+) and the UART page
// (0x1000_0000). Each is its own 4 KiB page.
const USER_CODE_VA:  usize = 0x0040_0000; //  4 MiB — user text
const USER_STACK_VA: usize = 0x0080_0000; //  8 MiB — user stack base (one page)

// The position-independent user program. Lives in kernel .text so the kernel
// can READ it to copy it out; it is NEVER executed in place — only the copy at
// USER_CODE_VA runs. Labels bracket it so we can compute its exact length.
core::arch::global_asm!(r#"
.section .text
.align 2
.global user_prog_start
.global user_prog_end
user_prog_start:
    li   a0, 0
    addi a0, a0, 1        # op 1  -> 1
    addi a0, a0, 1        # op 2  -> 2
    addi a0, a0, 1        # op 3  -> 3   (counter: proves real U-mode execution)
    mv   s0, a0           # stash counter across the test syscall
    li   a7, 0            # SYS_TEST
    ecall                 # a0 := 42 on return (ignored)
    mv   a0, s0           # exit code = counter = 3
    li   a7, 1            # SYS_EXIT
    ecall
1:  j 1b                  # never reached; spin if the kernel ever returns
user_prog_end:
"#);

extern "C" {
    static user_prog_start: u8;
    static user_prog_end: u8;
}

/// Stage and launch the first mapped user process. Never returns — it drops to
/// U-mode; the kernel's next action is via a trap.
pub fn launch() -> ! {
    let root = paging::kernel_root();

    // ---- 1. Copy the user program into a fresh physical frame ----
    let src   = unsafe { &raw const user_prog_start as *const u8 as usize };
    let src_e = unsafe { &raw const user_prog_end   as *const u8 as usize };
    let len   = src_e - src;

    let code_pa = alloc_frame().expect("user: OOM allocating code frame");
    unsafe {
        // Both src (kernel .text) and code_pa (fresh DRAM frame) are covered by
        // the identity map, so plain copies work in S-mode.
        core::ptr::copy_nonoverlapping(src as *const u8, code_pa as *mut u8, len);
        // fence.i: we wrote instructions via the DATA path; synchronize the
        // instruction stream so the fetch of the copy sees them. Single-hart, so
        // a local fence.i suffices (SMP would need a remote SBI fence). Classic
        // loaded-code gotcha — omit it and you can fetch stale prefetch on real
        // silicon (QEMU is lenient, but we write it correct).
        asm!("fence.i", options(nostack));
    }

    // ---- 2. Allocate + zero a user stack frame ----
    let stack_pa = alloc_frame().expect("user: OOM allocating stack frame");
    unsafe { core::ptr::write_bytes(stack_pa as *mut u8, 0, PAGE_SIZE); }

    // ---- 3. Map both into the address space with the U bit ----
    // Code:  U + R + X (+A). No W  -> user cannot modify its own code (W^X).
    // Stack: U + R + W (+A +D). No X -> stack is not executable (W^X).
    unsafe {
        paging::map_4k(root, USER_CODE_VA,  code_pa,  PTE_U | PTE_R | PTE_X | PTE_A);
        paging::map_4k(root, USER_STACK_VA, stack_pa, PTE_U | PTE_R | PTE_W | PTE_A | PTE_D);
        asm!("sfence.vma", options(nostack)); // new mappings -> flush TLB

        // SUM = 1 (sstatus bit 18): permit S-mode to access U-marked pages. On
        // an ecall from U-mode the trap handler is still on the USER stack
        // (RISC-V doesn't auto-switch sp), so it must be allowed to push the
        // trap frame onto that U page. NOTE: this makes the handler TRUST the
        // user sp — a hole the next step closes with an sscratch kernel stack.
        asm!("csrs sstatus, {b}", b = in(reg) 1usize << 18, options(nostack));
    }

    crate::kprintln!(
        "[L2] user mapped: code@{:#x}(pa {:#x}) stack@{:#x}(pa {:#x}) {} bytes",
        USER_CODE_VA, code_pa, USER_STACK_VA, stack_pa, len
    );
    crate::kprintln!("[L2] dropping to U-mode through real page tables...");

    // ---- 4. Enter U-mode at the mapped VAs ----
    let frame = crate::trap::create_test_user_context(USER_CODE_VA, USER_STACK_VA + PAGE_SIZE);
    crate::trap::enter_user_mode(&frame)
}
