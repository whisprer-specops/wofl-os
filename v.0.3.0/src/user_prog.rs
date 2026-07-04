//! B2a user program images: two position-independent stubs (immediates +
//! ecall only => zero relocations), staged into SEPARATE address spaces.
//!
//! A: SYS_YIELD immediately, then SYS_EXIT(0) if ever resumed.
//! B: SYS_EXIT(5) straight away.
//!
//! Sentinel 5 is deliberate: not 3 (L1/L2 counter), not 7 (IPC round-trip).
//! Seeing exit code 5 proves ONE thing unambiguously - the save-frame /
//! switch-satp / restore-frame path carried execution from A's address space
//! into B's. B2b layers IPC on top and brings 7 back.

core::arch::global_asm!(r#"
.section .text
.align 2

.global user_prog_a_start
.global user_prog_a_end
user_prog_a_start:
    li   a7, 2             # SYS_YIELD - hand the CPU to B
    ecall
    li   a0, 0             # if A is ever resumed, exit 0 (also a useful signal)
    li   a7, 1             # SYS_EXIT
    ecall
1:  j 1b
user_prog_a_end:

.align 2
.global user_prog_b_start
.global user_prog_b_end
user_prog_b_start:
    li   a0, 5             # B2a sentinel: the switch itself worked
    li   a7, 1             # SYS_EXIT
    ecall
1:  j 1b
user_prog_b_end:
"#);

extern "C" {
    static user_prog_a_start: u8;
    static user_prog_a_end: u8;
    static user_prog_b_start: u8;
    static user_prog_b_end: u8;
}

/// (source_address, length) of program A's image in kernel .text.
pub fn image_a() -> (usize, usize) {
    let s = &raw const user_prog_a_start as *const u8 as usize;
    let e = &raw const user_prog_a_end   as *const u8 as usize;
    (s, e - s)
}

/// (source_address, length) of program B's image in kernel .text.
pub fn image_b() -> (usize, usize) {
    let s = &raw const user_prog_b_start as *const u8 as usize;
    let e = &raw const user_prog_b_end   as *const u8 as usize;
    (s, e - s)
}
