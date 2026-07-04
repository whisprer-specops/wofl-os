// src/trap.rs - Layer 1 context switching + Layer 2 trusted-trap-stack hardening

use core::arch::asm;
use crate::syscall::*;

#[repr(C)]
pub struct TrapFrame {
    pub regs: [usize; 31],
    pub sepc: usize,
    pub sstatus: usize,
}

impl TrapFrame {
    pub const fn zero() -> Self { Self { regs: [0; 31], sepc: 0, sstatus: 0 } }
}

// ---- Dedicated kernel trap stack ----
// The handler runs HERE, never on the user's stack. sscratch holds this top
// while we're in U-mode; the vector swaps it into sp on trap entry. Separate
// from the boot stack so a trap never resets sp into the middle of the kernel's
// own live call chain. 16 KiB is ample for the 264-byte frame + kprintln depth.
const TRAP_STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct TrapStack([u8; TRAP_STACK_SIZE]);

static mut KERNEL_TRAP_STACK: TrapStack = TrapStack([0; TRAP_STACK_SIZE]);

#[inline]
fn trap_stack_top() -> usize {
    let base = &raw const KERNEL_TRAP_STACK as *const u8 as usize;
    base + TRAP_STACK_SIZE
}

pub fn init() {
    unsafe {
        extern "C" { fn _trap_vector(); }
        asm!("csrw stvec, {h}", h = in(reg) _trap_vector as *const () as usize);
        // S-mode convention: sscratch == 0 means "currently executing in S-mode".
        // Set explicitly because OpenSBI does not guarantee its reset value; a
        // kernel-origin fault before the first user entry must take the S-path.
        asm!("csrw sscratch, x0");
        // Interrupts stay OFF for now (return in Layer 4 with a proper timer
        // rearm). The old `csrsi sstatus,0x2` armed SIE with no STIP rearm -> storm.
    }
    crate::kprintln!("[TRAP] trap vector installed + trusted kernel trap stack armed");
}

pub fn create_test_user_context(entry: usize, stack: usize) -> TrapFrame {
    let mut frame = TrapFrame::zero();
    frame.sepc = entry;
    frame.regs[1] = stack; // x2 / sp
    frame
}

pub fn enter_user_mode(frame: &TrapFrame) -> ! {
    // Park the trusted trap stack top in sscratch BEFORE we drop to U-mode, so
    // the very first user trap already lands on kernel memory. Every subsequent
    // U-return re-arms sscratch from inside the vector, keeping this invariant.
    unsafe { asm!("csrw sscratch, {t}", t = in(reg) trap_stack_top()); }

    // Derive user sstatus from the live one (keep FS etc); set SPP=0 (return to
    // U) and SPIE=0 (ints off after sret). We deliberately DO NOT set SUM: with
    // the trap-stack switch the handler never touches user pages, so S-mode has
    // no business reaching into U memory. That closes the trust hole entirely.
    let mut sstatus: usize;
    unsafe { asm!("csrr {s}, sstatus", s = out(reg) sstatus); }
    sstatus &= !(1usize << 8); // SPP  = 0
    sstatus &= !(1usize << 5); // SPIE = 0
    unsafe {
        asm!(
            "csrw sepc, {sepc}",
            "csrw sstatus, {sstatus}",
            "mv tp, {frame}",
            "ld ra, 0(tp)",
            "ld sp, 8(tp)",
            "ld gp, 16(tp)",
            "ld t0, 32(tp)",
            "ld t1, 40(tp)",
            "ld t2, 48(tp)",
            "ld s0, 56(tp)",
            "ld s1, 64(tp)",
            "ld a0, 72(tp)",
            "ld a1, 80(tp)",
            "ld a2, 88(tp)",
            "ld a3, 96(tp)",
            "ld a4, 104(tp)",
            "ld a5, 112(tp)",
            "ld a6, 120(tp)",
            "ld a7, 128(tp)",
            "ld s2, 136(tp)",
            "ld s3, 144(tp)",
            "ld s4, 152(tp)",
            "ld s5, 160(tp)",
            "ld s6, 168(tp)",
            "ld s7, 176(tp)",
            "ld s8, 184(tp)",
            "ld s9, 192(tp)",
            "ld s10, 200(tp)",
            "ld s11, 208(tp)",
            "ld t3, 216(tp)",
            "ld t4, 224(tp)",
            "ld t5, 232(tp)",
            "ld t6, 240(tp)",
            "ld tp, 24(tp)",
            "sret",
            sepc = in(reg) frame.sepc,
            sstatus = in(reg) sstatus,
            frame = in(reg) frame as *const TrapFrame as usize,
            options(noreturn)
        );
    }
}

#[no_mangle]
pub extern "C" fn trap_handler(frame: &mut TrapFrame) {
    let scause: usize; let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }
    let is_interrupt = (scause >> 63) != 0;
    let code = scause & 0x7FFF_FFFF_FFFF_FFFF;
    if is_interrupt { handle_interrupt(code, frame); }
    else { handle_exception(code, stval, frame); }
}

fn handle_interrupt(code: usize, _frame: &mut TrapFrame) {
    crate::kprintln!("[TRAP] interrupt {} (unexpected, ints off) - halting", code);
    loop { unsafe { asm!("wfi"); } }
}

fn handle_exception(code: usize, stval: usize, frame: &mut TrapFrame) {
    match code {
        8 => handle_syscall(frame),
        _ => {
            crate::kprintln!(
                "[TRAP] Unhandled exception: code={} stval={:#x} sepc={:#x} - halting",
                code, stval, frame.sepc
            );
            loop { unsafe { asm!("wfi"); } }
        }
    }
}

fn handle_syscall(frame: &mut TrapFrame) {
    let n = frame.regs[16]; // a7
    crate::kprintln!("[SYSCALL] {} ({})", syscall_name(n), n);
    match n {
        SYS_TEST => {
            crate::kprintln!("[SYSCALL] Test syscall from user mode - SUCCESS!");
            frame.regs[9] = 42; // a0
        }
        SYS_EXIT => {
            let code = frame.regs[9]; // a0
            crate::kprintln!("[SYSCALL] User process exit (code: {})", code);
            crate::kprintln!("");
            crate::kprintln!("*** woflOS: user isolated in its own pages, ran, exited cleanly ***");
            crate::kprintln!("Layer 2 (memory protection) HARDENED - trap runs on trusted stack, SUM off.");
            crate::kprintln!("Ready for Layer 3 (IPC & capabilities).");
            loop { unsafe { asm!("wfi"); } }
        }
        SYS_SEND => { crate::ipc::sys_send(frame); }
        SYS_RECV => { crate::ipc::sys_recv(frame); }
        SYS_SEND_REMOTE | SYS_RECV_REMOTE | SYS_NODE_DISCOVER => {
            crate::kprintln!("[SYSCALL] Distributed op not yet implemented (Layer 6)");
            frame.regs[9] = usize::MAX;
        }
        _ => {
            crate::kprintln!("[SYSCALL] Unknown syscall: {}", n);
            frame.regs[9] = usize::MAX;
        }
    }
    frame.sepc += 4;
}

// ---- Trap vector: trusted-stack switch on entry, mode-aware restore on exit ----
core::arch::global_asm!(
    r#"
.section .text
.align 4
.global _trap_vector
_trap_vector:
    # --- switch onto a trusted kernel stack ---
    # Invariant: sscratch = kernel trap-stack top when in U-mode, 0 when in S-mode.
    csrrw sp, sscratch, sp      # atomic swap sp <-> sscratch
    bnez  sp, 1f                # sp != 0 -> from U-mode (sscratch held kstack top)
    csrrw sp, sscratch, sp      # from S-mode: sp was 0; undo -> sp=kern sp, sscratch=0
1:
    addi sp, sp, -264
    sd   t0, 32(sp)             # stash t0 (x5) so we can use it as scratch
    csrr t0, sscratch           # U-origin: user sp ; S-origin: 0
    bnez t0, 2f
    addi t0, sp, 264            # S-origin: trapped sp = kstack top = sp + 264
2:
    sd   t0, 8(sp)              # regs[1] = trapped sp (x2), correct for either origin
    csrw sscratch, x0           # mark "in S-mode" while we handle (nested S-fault safe)

    sd ra, 0(sp)
    sd gp, 16(sp)
    sd tp, 24(sp)
    sd t1, 40(sp)
    sd t2, 48(sp)
    sd s0, 56(sp)
    sd s1, 64(sp)
    sd a0, 72(sp)
    sd a1, 80(sp)
    sd a2, 88(sp)
    sd a3, 96(sp)
    sd a4, 104(sp)
    sd a5, 112(sp)
    sd a6, 120(sp)
    sd a7, 128(sp)
    sd s2, 136(sp)
    sd s3, 144(sp)
    sd s4, 152(sp)
    sd s5, 160(sp)
    sd s6, 168(sp)
    sd s7, 176(sp)
    sd s8, 184(sp)
    sd s9, 192(sp)
    sd s10, 200(sp)
    sd s11, 208(sp)
    sd t3, 216(sp)
    sd t4, 224(sp)
    sd t5, 232(sp)
    sd t6, 240(sp)
    csrr t0, sepc
    sd t0, 248(sp)
    csrr t0, sstatus
    sd t0, 256(sp)

    mv a0, sp
    call trap_handler

    # --- restore ---
    ld t0, 256(sp)
    csrw sstatus, t0            # t0 = the sstatus we return with; SPP = bit 8
    andi t1, t0, 0x100          # t1 = SPP
    bnez t1, 3f                 # SPP=1 -> returning to S-mode, leave sscratch = 0
    addi t1, sp, 264           # returning to U-mode: re-arm sscratch = trap-stack top
    csrw sscratch, t1
3:
    ld t0, 248(sp)
    csrw sepc, t0
    ld ra, 0(sp)
    ld gp, 16(sp)
    ld tp, 24(sp)
    ld t0, 32(sp)
    ld t1, 40(sp)
    ld t2, 48(sp)
    ld s0, 56(sp)
    ld s1, 64(sp)
    ld a0, 72(sp)
    ld a1, 80(sp)
    ld a2, 88(sp)
    ld a3, 96(sp)
    ld a4, 104(sp)
    ld a5, 112(sp)
    ld a6, 120(sp)
    ld a7, 128(sp)
    ld s2, 136(sp)
    ld s3, 144(sp)
    ld s4, 152(sp)
    ld s5, 160(sp)
    ld s6, 168(sp)
    ld s7, 176(sp)
    ld s8, 184(sp)
    ld s9, 192(sp)
    ld s10, 200(sp)
    ld s11, 208(sp)
    ld t3, 216(sp)
    ld t4, 224(sp)
    ld t5, 232(sp)
    ld t6, 240(sp)
    ld sp, 8(sp)               # restore trapped sp LAST (user sp for U-return)
    sret
    "#
);
