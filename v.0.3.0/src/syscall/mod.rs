//! Syscall definitions for Layer 1 bring-up.
//!
//! woflOS is “distributed-native” from day 1, but Layer 1 only needs a
//! *minimal* syscall surface so we can prove:
//!
//! Boot → enter U-mode → `ecall` → trap → handle syscall → return → exit.
//!
//! Everything else (IPC, capabilities, networking) will be layered on later.

/// Layer 1 core syscalls
pub const SYS_TEST: usize = 0; // return 42
pub const SYS_EXIT: usize = 1; // halt for now
pub const SYS_YIELD: usize = 2; // cooperative: save frame, switch root, resume next

/// Reserved for Layer 3 IPC
pub const SYS_SEND: usize = 10;
pub const SYS_RECV: usize = 11;

/// Reserved for Layer 6+ distributed operations
pub const SYS_SEND_REMOTE: usize = 1000;
pub const SYS_RECV_REMOTE: usize = 1001;
pub const SYS_NODE_DISCOVER: usize = 1010;

pub fn syscall_name(n: usize) -> &'static str {
    match n {
        SYS_TEST => "SYS_TEST",
        SYS_EXIT => "SYS_EXIT",
        SYS_YIELD => "SYS_YIELD",
        SYS_SEND => "SYS_SEND",
        SYS_RECV => "SYS_RECV",
        SYS_SEND_REMOTE => "SYS_SEND_REMOTE",
        SYS_RECV_REMOTE => "SYS_RECV_REMOTE",
        SYS_NODE_DISCOVER => "SYS_NODE_DISCOVER",
        _ => "SYS_UNKNOWN",
    }
}
