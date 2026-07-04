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
#[link_section = ".text.boot"]
#[no_mangle]
pub extern "C" fn _start() -> ! {
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
    crate::kprintln!("[OK] woflOS v0.4.0 (Layer 1 bring-up)");

    // Layer 1: install trap vector + enable minimal trap handling
    trap::init();

    // Layer 1: enter user mode and prove round-trip syscall works.
    let user_entry = user_test::user_main as usize;
    let user_stack_top = user_test::get_user_stack_top();

    crate::kprintln!("[L1] entering user mode: entry={:#x} stack_top={:#x}", user_entry, user_stack_top);
    let frame = trap::create_test_user_context(user_entry, user_stack_top);
    trap::enter_user_mode(&frame)
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
