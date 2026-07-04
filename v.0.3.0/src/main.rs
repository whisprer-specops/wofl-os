#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::arch::asm;
use core::panic::PanicInfo;

mod uart;
mod memory;
mod syscall;
mod trap;
mod ipc;
mod process;
mod user_prog;

use uart::Uart;

/// Boot entry. This is the very first Rust code that runs.
#[no_mangle]
pub extern "C" fn rust_start() -> ! {
    let uart = Uart::new(0x1000_0000);
    uart.puts("[BOOT] kernel_main entered\n");

    extern "C" {
        static mut __bss_start: u8;
        static mut __bss_end: u8;
        static __kernel_end: u8;
    }

    unsafe {
        let bss_start = &raw mut __bss_start as *mut u8;
        let bss_end = &raw mut __bss_end as *mut u8;
        let bss_len = bss_end as usize - bss_start as usize;
        core::ptr::write_bytes(bss_start, 0, bss_len);
    }
    uart.puts("[BOOT] .bss cleared\n");

    let kernel_end = unsafe { &raw const __kernel_end as *const u8 as usize };
    let memory_end = 0x8800_0000; // QEMU virt, 128MB RAM top
    unsafe { memory::init(kernel_end, memory_end) };
    uart.puts("[BOOT] memory initialized\n");

    kernel_main_inner()
}

fn kernel_main_inner() -> ! {
    crate::kprintln!("");
    crate::kprintln!("============================================");
    crate::kprintln!(" __      __ ___  ___  _     ___   ___ ");
    crate::kprintln!(" \\ \\    / // _ \\| __|| |   / _ \\ / __|");
    crate::kprintln!("  \\ \\/\\/ /| (_) | _| | |__| (_) |\\__ \\");
    crate::kprintln!("   \\_/\\_/  \\___/|_|  |____|\\___/ |___/");
    crate::kprintln!("============================================");
    crate::kprintln!("[OK] woflOS v0.4.0 (Layer 2 bring-up)");

    // Arm the trap vector FIRST so any fault while enabling paging / entering
    // user mode gets one clean diagnostic line instead of a storm.
    trap::init();

    // Layer 2, step 1: enable Sv39 (kernel-only address space).
    let root = unsafe { memory::paging::init() };
    crate::kprintln!("[L2] Sv39 paging ENABLED (root PT @ {:#x})", root);
    crate::kprintln!("[L2] kernel now executing under virtual->physical translation");

    // Layer 2, step 2: stage the first user program in REAL mapped U-pages and
    // drop to U-mode. The ecall round-trip now runs through genuine page tables
    // with enforced U/S separation. Never returns.
    // Layer 4 step B1: spawn a process with its OWN Sv39 root, switch satp to
    // it, and enter. Proves per-process address space + satp switch by running
    // the self-IPC test under a fresh root instead of the shared kernel root.
    // Layer 4 step B2a: spawn TWO processes in separate address spaces.
    // A yields immediately; the cooperative switch carries execution into B,
    // which exits sentinel 5. Proves save-frame/switch-satp/restore-frame.
    let (a_src, a_len) = user_prog::image_a();
    let (b_src, b_len) = user_prog::image_b();
    unsafe {
        process::spawn(a_src, a_len);   // pid 1, slot 0
        process::spawn(b_src, b_len);   // pid 2, slot 1
        process::run_first(0)
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::kprintln!("\n[PANIC] kernel panic");
    if let Some(loc) = info.location() {
        crate::kprintln!("[PANIC] at {}:{}", loc.file(), loc.line());
    }
    loop { unsafe { asm!("wfi"); } }
}

#[alloc_error_handler]
fn alloc_error(_layout: core::alloc::Layout) -> ! {
    crate::kprintln!("\n[PANIC] allocation error");
    loop { unsafe { asm!("wfi"); } }
}

// ---- Assembly boot entry (the stack fix) ----
core::arch::global_asm!(r#"
.section .text.boot
.global _start
_start:
    la   sp, __kernel_stack_top
    tail rust_start
"#);
