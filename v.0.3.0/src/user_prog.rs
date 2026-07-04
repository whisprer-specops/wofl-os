//! Layer 3 self-IPC proof: user writes a magic payload, SYS_SENDs it to a
//! LOCAL endpoint (node_id 0), SYS_RECVs it back into a different buffer, and
//! compares. If the bytes survived the kernel round-trip (copy_from_user ->
//! endpoint queue -> copy_to_user, all with SUM off and capability checks),
//! it exits 7 — a sentinel distinct from Layer 1/2's 3, so we KNOW IPC ran.

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W, PTE_X, PTE_A, PTE_D};
use crate::memory::frame::alloc_frame;
use crate::memory::PAGE_SIZE;
use core::arch::asm;

const USER_CODE_VA:  usize = 0x0040_0000;
const USER_STACK_VA: usize = 0x0080_0000;

// Position-independent (immediates + ecall + PC-relative branches => zero
// relocations). SYS_SEND=10, SYS_RECV=11, SYS_EXIT=1.
//   send buf @ sp-256, recv buf @ sp-512 (both inside the mapped stack page).
//   a2=0 selects LOCAL routing. Flip to `li a2, 1` to fire the send_remote stub.
core::arch::global_asm!(r#"
.section .text
.align 2
.global user_prog_start
.global user_prog_end
user_prog_start:
    li   t0, 0xC0FFEE          # magic payload
    addi a3, sp, -256          # send buffer VA
    sd   t0, 0(a3)             # write magic into send buffer
    # SYS_SEND(a0=buf, a1=len, a2=node_id)
    mv   a0, a3
    li   a1, 8
    li   a2, 0                 # node_id 0 = LOCAL
    li   a7, 10
    ecall                      # a0 = 0 on success
    # SYS_RECV(a0=dstbuf, a1=maxlen)
    addi a4, sp, -512          # recv buffer VA (different from send)
    mv   a0, a4
    li   a1, 8
    li   a7, 11
    ecall                      # a0 = bytes received
    # compare round-tripped magic
    ld   t1, 0(a4)
    li   t2, 0xC0FFEE
    li   a0, 0                 # default: failure code 0
    bne  t1, t2, 1f
    li   a0, 7                 # SUCCESS: payload survived kernel round-trip
1:
    li   a7, 1                 # SYS_EXIT
    ecall
2:  j 2b
user_prog_end:
"#);

extern "C" {
    static user_prog_start: u8;
    static user_prog_end: u8;
}

pub fn launch() -> ! {
    let root = paging::kernel_root();

    let src   = &raw const user_prog_start as *const u8 as usize;
    let src_e = &raw const user_prog_end   as *const u8 as usize;
    let len   = src_e - src;

    let code_pa = alloc_frame().expect("user: OOM allocating code frame");
    unsafe {
        core::ptr::copy_nonoverlapping(src as *const u8, code_pa as *mut u8, len);
        asm!("fence.i", options(nostack));
    }

    let stack_pa = alloc_frame().expect("user: OOM allocating stack frame");
    unsafe { core::ptr::write_bytes(stack_pa as *mut u8, 0, PAGE_SIZE); }

    unsafe {
        paging::map_4k(root, USER_CODE_VA,  code_pa,  PTE_U | PTE_R | PTE_X | PTE_A);
        paging::map_4k(root, USER_STACK_VA, stack_pa, PTE_U | PTE_R | PTE_W | PTE_A | PTE_D);
        asm!("sfence.vma", options(nostack));
        // SUM stays OFF: the kernel reaches user bytes via translate()+identity
        // map after validating the U bit, never by ambient S->U access.
    }

    crate::kprintln!(
        "[L3] user mapped: code@{:#x} stack@{:#x} ({} bytes) — self-IPC test",
        USER_CODE_VA, USER_STACK_VA, len
    );
    crate::kprintln!("[L3] dropping to U-mode: will SYS_SEND a magic word, SYS_RECV it back...");

    let frame = crate::trap::create_test_user_context(USER_CODE_VA, USER_STACK_VA + PAGE_SIZE);
    crate::trap::enter_user_mode(&frame)
}
