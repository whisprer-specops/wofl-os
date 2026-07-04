//! Minimal SBI (Supervisor Binary Interface) calls into OpenSBI.
//!
//! S-mode cannot write the M-mode timer comparator; SBI set_timer is the
//! legal path. Per the SBI spec, set_timer also CLEARS pending STIP - which
//! is exactly the rearm the legacy `csrsi sstatus,0x2` storm was missing.

use core::arch::asm;

/// Tick interval: QEMU virt timebase = 10 MHz, so 200_000 = 20 ms real time.
/// (Was 100 ms - too coarse: the spinners finished inside a single quantum,
/// so the first deadline was never reached and NO tick fired. 20 ms lands
/// several deadlines inside each spinner run.) Rearm is relative to read_time()
/// at handling, so a slow handler can never pile up ticks -> no storm.
pub const TIMER_INTERVAL: u64 = 200_000;

/// Read the free-running time CSR. Requires mcounteren.TM (OpenSBI sets it
/// on QEMU virt). If a platform doesn't: unhandled exception code=2
/// (illegal instruction) right here - that's the diagnostic fingerprint.
pub fn read_time() -> u64 {
    let t: u64;
    unsafe { asm!("rdtime {t}", t = out(reg) t, options(nostack, nomem)); }
    t
}

/// SBI TIME extension (EID 0x54494D45 "TIME", FID 0): program the next timer
/// deadline. a0/a1 are clobbered by the SBI return (error/value).
pub fn set_timer(stime: u64) {
    unsafe {
        asm!("ecall",
            in("a7") 0x5449_4D45usize, // EID: TIME
            in("a6") 0usize,           // FID: set_timer
            in("a0") stime as usize,
            lateout("a0") _, lateout("a1") _,
            options(nostack));
    }
}
