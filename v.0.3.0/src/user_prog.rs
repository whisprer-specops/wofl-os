//! User program image: a position-independent self-IPC stub (immediates + ecall
//! + PC-relative branches => zero relocations), so running the copy at any VA is
//! provably correct. Staged into a process's own address space by Process::new_user.
//!
//! Logic: write magic 0xC0FFEE into a send buffer, SYS_SEND it LOCAL (node 0),
//! SYS_RECV it into a different buffer, compare. Survived the kernel round-trip
//! => exit 7 (sentinel distinct from L1/L2's 3, so we KNOW IPC ran).

core::arch::global_asm!(r#"
.section .text
.align 2
.global user_prog_start
.global user_prog_end
user_prog_start:
    li   t0, 0xC0FFEE
    addi a3, sp, -256      # send buffer VA (inside mapped stack page)
    sd   t0, 0(a3)
    mv   a0, a3            # SYS_SEND(a0=buf, a1=len, a2=node_id)
    li   a1, 8
    li   a2, 0             # node 0 = LOCAL routing
    li   a7, 10
    ecall
    addi a4, sp, -512      # recv buffer VA (distinct from send)
    mv   a0, a4            # SYS_RECV(a0=dstbuf, a1=maxlen)
    li   a1, 8
    li   a7, 11
    ecall
    ld   t1, 0(a4)         # read the round-tripped word back
    li   t2, 0xC0FFEE
    li   a0, 0             # default failure code
    bne  t1, t2, 1f
    li   a0, 7             # SUCCESS: payload survived kernel round-trip
1:
    li   a7, 1             # SYS_EXIT(a0)
    ecall
2:  j 2b
user_prog_end:
"#);

extern "C" {
    static user_prog_start: u8;
    static user_prog_end: u8;
}

/// (source_address, length) of the program image in kernel .text.
pub fn image() -> (usize, usize) {
    let s = &raw const user_prog_start as *const u8 as usize;
    let e = &raw const user_prog_end   as *const u8 as usize;
    (s, e - s)
}
