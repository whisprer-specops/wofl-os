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
mod user_test;

use uart::Uart;

/// Boot entry. This is the very first Rust code that runs.
///
/// Keep it brutally small:
/// - UART online
/// - .bss cleared
/// - memory subsystem initialized (frame + heap)
/// - jump to `kernel_main()`
#[no_mangle]
pub extern "C" fn rust_start() -> ! {
    let uart = Uart::new(0x1000_0000);
    uart.puts("[BOOT] kernel_main entered\n");

    // Clear .bss (uninitialized globals)
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

    // Initialize memory system (frame allocator + heap)
    let kernel_end = unsafe { &raw const __kernel_end as *const u8 as usize };
    let memory_end = 0x8800_0000; // QEMU virt, 128MB RAM top
    unsafe { memory::init(kernel_end, memory_end) };
    uart.puts("[BOOT] memory initialized\n");

    // Continue with the real kernel
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

    // Layer 1: install the trap vector FIRST. If enabling paging below faults
    // on a mapping we got wrong, the hardened handler prints ONE diagnostic
    // line and halts instead of storming. Arm the net before the high-wire act.
    trap::init();

    // ---- Layer 2, step 1: enable Sv39 paging (kernel-only address space) ----
    // No U bit anywhere yet, so the Layer 1 user test is GATED OFF below. This
    // step proves the satp / sfence.vma / TLB machinery works in isolation:
    // the kernel keeps running -- printing, using its stack -- but now every
    // address it touches goes through a page table it walks in hardware.
    let root = unsafe { memory::paging::init() };
    crate::kprintln!("[L2] Sv39 paging ENABLED (root PT @ {:#x})", root);
    crate::kprintln!("[L2] kernel now executing under virtual->physical translation");

    // ---- Layer 1 user test: GATED OFF for step 1 ----
    // Re-enabled in step 2, once user code + stack get real U-bit page mappings.
    // let user_entry = user_test::user_main as usize;
    // let user_stack_top = user_test::get_user_stack_top();
    // crate::kprintln!("[L1] entering user mode: entry={:#x} stack_top={:#x}", user_entry, user_stack_top);
    // let frame = trap::create_test_user_context(user_entry, user_stack_top);
    // trap::enter_user_mode(&frame)

    crate::kprintln!("");
    crate::kprintln!("*** Layer 2 Step 1: PAGING OPERATIONAL ***");
    crate::kprintln!("Kernel survived the satp switch. Ready for user mappings.");
    loop { unsafe { asm!("wfi"); } }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::kprintln!("\n[PANIC] kernel panic");
    if let Some(loc) = info.location() {
        crate::kprintln!("[PANIC] at {}:{}", loc.file(), loc.line());
    }
    loop {
        unsafe { asm!("wfi"); }
    }
}

#[alloc_error_handler]
fn alloc_error(_layout: core::alloc::Layout) -> ! {
    crate::kprintln!("\n[PANIC] allocation error");
    loop {
        unsafe { asm!("wfi"); }
    }
}

// ---- Assembly boot entry (the stack fix) ----
// This is the linker's ENTRY point. It sets sp to our reserved kernel stack
// BEFORE any Rust runs, then tail-calls rust_start. Without this we ran on
// OpenSBI's leftover sp, which points into its own PMP-protected memory -> the
// first stack store faulted forever (the storm we've been chasing).
core::arch::global_asm!(r#"
.section .text.boot
.global _start
_start:
    la   sp, __kernel_stack_top
    tail rust_start
"#);
