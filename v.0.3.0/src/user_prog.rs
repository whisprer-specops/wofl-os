//! Layer 2 — the first *properly mapped* user program.
//!
//! Runs in its own U-bit pages under Sv39. The ecall round-trip traps through
//! the hardened vector, which switches to a trusted kernel trap stack via
//! sscratch — so the handler never runs on, or touches, user memory. SUM stays
//! off: S-mode has no ambient reach into U pages (zero-trust, no ambient authority).

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W, PTE_X, PTE_A, PTE_D};
use crate::memory::frame::alloc_frame;
use crate::memory::PAGE_SIZE;
use core::arch::asm;

const USER_CODE_VA:  usize = 0x0040_0000; // 4 MiB — user text
const USER_STACK_VA: usize = 0x0080_0000; // 8 MiB — user stack base (one page)

// Position-independent user stub (immediates + ecall + PC-relative branch =>
// zero relocations), so running the copy at USER_CODE_VA is provably correct.
core::arch::global_asm!(r#"
.section .text
.align 2
.global user_prog_start
.global user_prog_end
user_prog_start:
    li   a0, 0
    addi a0, a0, 1        # op 1 -> 1
    addi a0, a0, 1        # op 2 -> 2
    addi a0, a0, 1        # op 3 -> 3   (counter proves real U-mode execution)
    mv   s0, a0           # stash counter across the test syscall
    li   a7, 0            # SYS_TEST
    ecall                 # a0 := 42 on return (ignored)
    mv   a0, s0           # exit code = counter = 3
    li   a7, 1            # SYS_EXIT
    ecall
1:  j 1b
user_prog_end:
"#);

extern "C" {
    static user_prog_start: u8;
    static user_prog_end: u8;
}

pub fn launch() -> ! {
    let root = paging::kernel_root();

    let src   = unsafe { &raw const user_prog_start as *const u8 as usize };
    let src_e = unsafe { &raw const user_prog_end   as *const u8 as usize };
    let len   = src_e - src;

    let code_pa = alloc_frame().expect("user: OOM allocating code frame");
    unsafe {
        // src (kernel .text) and code_pa (fresh DRAM) are both in the identity
        // map, so this S-mode copy needs no user access.
        core::ptr::copy_nonoverlapping(src as *const u8, code_pa as *mut u8, len);
        asm!("fence.i", options(nostack)); // sync I-stream to the data writes
    }

    let stack_pa = alloc_frame().expect("user: OOM allocating stack frame");
    unsafe { core::ptr::write_bytes(stack_pa as *mut u8, 0, PAGE_SIZE); }

    unsafe {
        // Code:  U+R+X (W^X: user can't rewrite its code). Stack: U+R+W (no X).
        paging::map_4k(root, USER_CODE_VA,  code_pa,  PTE_U | PTE_R | PTE_X | PTE_A);
        paging::map_4k(root, USER_STACK_VA, stack_pa, PTE_U | PTE_R | PTE_W | PTE_A | PTE_D);
        asm!("sfence.vma", options(nostack));
        // NOTE: no `csrs sstatus, SUM` anymore — the trap-stack switch removed
        // the only reason S-mode ever needed to write a U page. Hole closed.
    }

    crate::kprintln!(
        "[L2] user mapped: code@{:#x}(pa {:#x}) stack@{:#x}(pa {:#x}) {} bytes",
        USER_CODE_VA, code_pa, USER_STACK_VA, stack_pa, len
    );
    crate::kprintln!("[L2] dropping to U-mode (trap stack trusted, SUM off)...");

    let frame = crate::trap::create_test_user_context(USER_CODE_VA, USER_STACK_VA + PAGE_SIZE);
    crate::trap::enter_user_mode(&frame)
}
