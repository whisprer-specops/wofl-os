//! Preemption test images: two pure spinners that NEVER yield, block, or do
//! IPC. Each burns a counted loop then exits a distinct code (11 / 22). The
//! ONLY way execution can alternate between them is the timer - so every
//! `[L4] tick: preempt` line is involuntary multitasking, witnessed.
//! (The blocking-IPC choreography lives on under tag layer4-blocking-recv.)

core::arch::global_asm!(r#"
.section .text
.align 2

.global user_prog_spin_a_start
.global user_prog_spin_a_end
user_prog_spin_a_start:
    li   t0, 3000000000     # ~600M instructions - spans many 20ms quanta
1:  addi t0, t0, -1
    bnez t0, 1b
    li   a0, 11            # spinner A sentinel
    li   a7, 1             # SYS_EXIT
    ecall
2:  j 2b
user_prog_spin_a_end:

.align 2
.global user_prog_spin_b_start
.global user_prog_spin_b_end
user_prog_spin_b_start:
    li   t0, 300000000
1:  addi t0, t0, -1
    bnez t0, 1b
    li   a0, 22            # spinner B sentinel
    li   a7, 1             # SYS_EXIT
    ecall
2:  j 2b
user_prog_spin_b_end:
"#);

extern "C" {
    static user_prog_spin_a_start: u8;
    static user_prog_spin_a_end: u8;
    static user_prog_spin_b_start: u8;
    static user_prog_spin_b_end: u8;
}

pub fn image_spin_a() -> (usize, usize) {
    let s = &raw const user_prog_spin_a_start as *const u8 as usize;
    let e = &raw const user_prog_spin_a_end   as *const u8 as usize;
    (s, e - s)
}

pub fn image_spin_b() -> (usize, usize) {
    let s = &raw const user_prog_spin_b_start as *const u8 as usize;
    let e = &raw const user_prog_spin_b_end   as *const u8 as usize;
    (s, e - s)
}
