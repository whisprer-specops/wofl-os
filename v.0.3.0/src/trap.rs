// src/trap.rs - Layer 1 Context Switching Foundation

use core::arch::asm;
use crate::syscall::*;

/// Trap frame: saved register state for context switching
#[repr(C)]
pub struct TrapFrame {
    pub regs: [usize; 31],  // x1-x31 (x0 is hardwired zero)
    pub sepc: usize,        // Saved program counter
    pub sstatus: usize,     // Saved status register
}

impl TrapFrame {
    pub const fn zero() -> Self {
        Self {
            regs: [0; 31],
            sepc: 0,
            sstatus: 0,
        }
    }
}

/// Initialize trap handling for Layer 1
pub fn init() {
    unsafe {
        // Set trap vector to our handler
        extern "C" {
            fn _trap_vector();
        }
        asm!(
            "csrw stvec, {handler}",
            handler = in(reg) _trap_vector as usize,
        );
        
        // Enable supervisor interrupts (we'll need timer later)
        asm!("csrsi sstatus, 0x2"); // SIE bit
        
    }

    crate::kprintln!("[TRAP] Layer 1 initialized - context switching ready");
}

// No exception delegation: OpenSBI boots us into S-mode and the kernel owns
// every trap via stvec. A U-mode ecall traps to S-mode by default — the old
// sedeleg/sideleg regs were part of the never-ratified "N" extension (removed
// from the spec), which is why the assembler rejected them.

/// Create a fake user context for testing
pub fn create_test_user_context(entry: usize, stack: usize) -> TrapFrame {
    let mut frame = TrapFrame::zero();
    
    // Set program counter to user function
    frame.sepc = entry;
    
    // Set user stack pointer (x2/sp)
    // regs[0] is x1 (ra), regs[1] is x2 (sp)
    frame.regs[1] = stack;
    
    // Set sstatus for user mode:
    // - SPP=0 (return to user mode)
    // - SPIE=1 (enable interrupts after sret)
    frame.sstatus = 0x20; // SPIE bit
    
    frame
}

/// Jump to user mode with the given context
/// NEVER RETURNS - transfers control to user mode
pub fn enter_user_mode(frame: &TrapFrame) -> ! {
    unsafe {
        // Restore context and sret to user mode
        asm!(
            // Load sepc and sstatus
            "csrw sepc, {sepc}",
            "csrw sstatus, {sstatus}",
            
            // Restore all general-purpose registers (x1-x31)
            // Use tp (x4) as temporary frame pointer
            "mv tp, {frame}",
            "ld ra, 0(tp)",      // x1
            "ld sp, 8(tp)",      // x2
            "ld gp, 16(tp)",     // x3
            // Skip x4 (tp itself), restore it last
            "ld t0, 32(tp)",     // x5
            "ld t1, 40(tp)",     // x6
            "ld t2, 48(tp)",     // x7
            "ld s0, 56(tp)",     // x8
            "ld s1, 64(tp)",     // x9
            "ld a0, 72(tp)",     // x10
            "ld a1, 80(tp)",     // x11
            "ld a2, 88(tp)",     // x12
            "ld a3, 96(tp)",     // x13
            "ld a4, 104(tp)",    // x14
            "ld a5, 112(tp)",    // x15
            "ld a6, 120(tp)",    // x16
            "ld a7, 128(tp)",    // x17
            "ld s2, 136(tp)",    // x18
            "ld s3, 144(tp)",    // x19
            "ld s4, 152(tp)",    // x20
            "ld s5, 160(tp)",    // x21
            "ld s6, 168(tp)",    // x22
            "ld s7, 176(tp)",    // x23
            "ld s8, 184(tp)",    // x24
            "ld s9, 192(tp)",    // x25
            "ld s10, 200(tp)",   // x26
            "ld s11, 208(tp)",   // x27
            "ld t3, 216(tp)",    // x28
            "ld t4, 224(tp)",    // x29
            "ld t5, 232(tp)",    // x30
            "ld t6, 240(tp)",    // x31
            "ld tp, 24(tp)",     // x4 (tp) - restore last
            
            // Return to user mode
            "sret",
            
            sepc = in(reg) frame.sepc,
            sstatus = in(reg) frame.sstatus,
            frame = in(reg) frame as *const TrapFrame as usize,
            options(noreturn)
        );
    }
}

/// The trap handler (called from assembly vector)
#[no_mangle]
pub extern "C" fn trap_handler(frame: &mut TrapFrame) {
    // Read trap cause
    let scause: usize;
    let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }
    
    // Decode cause
    let is_interrupt = (scause >> 63) != 0; // MSB indicates interrupt
    let code = scause & 0x7FFFFFFFFFFFFFFF;
    
    if is_interrupt {
        handle_interrupt(code, frame);
    } else {
        handle_exception(code, stval, frame);
    }
}

fn handle_interrupt(code: usize, _frame: &mut TrapFrame) {
    match code {
        5 => { // Supervisor timer interrupt
            crate::kprintln!("[TRAP] Timer interrupt");
            // Acknowledge by setting next timer (we'll do this properly in Layer 3)
        }
        _ => {
            crate::kprintln!("[TRAP] Unknown interrupt: {}", code);
        }
    }
}

fn handle_exception(code: usize, stval: usize, frame: &mut TrapFrame) {
    match code {
        8 => { // Environment call from U-mode (ecall)
            handle_syscall(frame);
        }
        9 => { // Environment call from S-mode
            panic!("Unexpected ecall from S-mode");
        }
        12 => { // Instruction page fault
            panic!("Instruction page fault at {:#x}", stval);
        }
        13 => { // Load page fault
            panic!("Load page fault at {:#x}", stval);
        }
        15 => { // Store page fault
            panic!("Store page fault at {:#x}", stval);
        }
        _ => {
            panic!("Unhandled exception: code={}, stval={:#x}", code, stval);
        }
    }
}

fn handle_syscall(frame: &mut TrapFrame) {
    // Syscall number in a7 (regs[16])
    // Arguments in a0-a5 (regs[9-14])
    // Return value goes in a0 (regs[9])
    let syscall_num = frame.regs[16]; // a7
    
    crate::kprintln!("[SYSCALL] {} ({})", syscall_name(syscall_num), syscall_num);
    
    match syscall_num {
        SYS_TEST => {
            // Test syscall - just print success and return value
            crate::kprintln!("[SYSCALL] Test syscall from user mode - SUCCESS! 🐺");
            frame.regs[9] = 42; // Return value in a0
        }
        
        SYS_EXIT => {
            // Exit syscall - for now, just halt
            let exit_code = frame.regs[9]; // a0
            crate::kprintln!("[SYSCALL] User process exit (code: {})", exit_code);
            crate::kprintln!("\n🎉 Layer 1 Context Switching: OPERATIONAL ✓");
            crate::kprintln!("Ready to proceed to Layer 2 (PMP)!\n");
            loop {
                unsafe { asm!("wfi"); }
            }
        }
        
        // Future syscalls (Layer 3+)
        SYS_SEND | SYS_RECV => {
            crate::kprintln!("[SYSCALL] IPC not yet implemented (Layer 3 feature)");
            frame.regs[9] = usize::MAX; // Error return
        }
        
        // Distributed syscalls (Layer 6+)
        SYS_SEND_REMOTE | SYS_RECV_REMOTE | SYS_NODE_DISCOVER => {
            crate::kprintln!("[SYSCALL] Distributed operation not yet implemented (Layer 6 feature)");
            frame.regs[9] = usize::MAX; // Error return
        }
        
        _ => {
            crate::kprintln!("[SYSCALL] Unknown syscall: {}", syscall_num);
            frame.regs[9] = usize::MAX; // Error return
        }
    }
    
    // Advance sepc past the ecall instruction (4 bytes)
    frame.sepc += 4;
}

// The actual trap vector (assembly trampoline)
core::arch::global_asm!(
    r#"
.section .text
.align 4
.global _trap_vector
_trap_vector:
    # Save context to kernel stack
    # For now, simplified: assume we have space on current stack
    # Layer 2 will use sscratch CSR for proper kernel/user stack separation
    addi sp, sp, -264  # sizeof(TrapFrame) = 33*8 bytes

    # Save original (user) sp value into the TrapFrame slot for x2.
    # After the subtraction above, original_sp == sp + 264.
    addi t0, sp, 264
    
    # Save all registers
    sd ra, 0(sp)
    sd t0, 8(sp)       # Save original user stack pointer
    sd gp, 16(sp)
    sd tp, 24(sp)
    sd t0, 32(sp)
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
    
    # Save sepc and sstatus
    csrr t0, sepc
    sd t0, 248(sp)
    csrr t0, sstatus
    sd t0, 256(sp)
    
    # Call Rust handler
    mv a0, sp          # Pass frame pointer as argument
    call trap_handler
    
    # Restore context
    ld t0, 256(sp)
    csrw sstatus, t0
    ld t0, 248(sp)
    csrw sepc, t0
    
    # Restore registers
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

    # Deallocate TrapFrame; this restores sp back to the original user value.
    addi sp, sp, 264
    
    sret
    "#
);