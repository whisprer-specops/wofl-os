//! Layer 3 — IPC & capabilities (the distributed keystone).
//!
//! Distributed-native from day one. Two shapes carry the whole thesis:
//!   * `Capability` has `node_id` (0 = local, >0 = remote) so a remote cap is
//!     later just "same struct, node_id>0, HMAC over these bytes" — no refactor.
//!   * `IPCMessage` is fixed-size and pointer-free, so the SAME copy code moves
//!     it between address spaces today and across a wire at Layer 6.
//!
//! Routing branches on `dst_cap.node_id`: 0 -> `send_local`, >0 -> `send_remote`
//! (a present-but-halting stub L6 fills in). The branch existing NOW is what
//! makes L6 a fill-in rather than a rewrite.
//!
//! uaccess note: SUM is OFF (zero-trust). The kernel reaches user bytes via the
//! DRAM identity map after `paging::translate()` both (a) resolves the VA->PA
//! and (b) proves every page carries the U bit — so the kernel only ever touches
//! memory the user itself owns. A user pointer into kernel space fails the U
//! check and is rejected, never dereferenced.

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W};
use crate::memory::PAGE_SIZE;
use crate::trap::TrapFrame;

/// Max inline payload. Design doc says 4096; 256 keeps the static queue small
/// and avoids 4 KiB by-value copies for bring-up. Single knob — bump freely;
/// the serialization property does not depend on N.
pub const IPC_PAYLOAD_MAX: usize = 256;

/// Endpoint queue depth (fixed — no heap, no pointers).
const ENDPOINT_DEPTH: usize = 4;

// Capability permission bits.
pub const CAP_SEND: u32 = 1 << 0;
pub const CAP_RECV: u32 = 1 << 1;

// Message types (room for control/data/etc later).
pub const MSG_DATA: u32 = 1;

/// Well-known local service id for the bring-up endpoint.
const SERVICE_ECHO: u32 = 1;

/// Unforgeable authority token. `#[repr(C)]`, fixed-size, no pointers ->
/// serializable as-is. Locally the kernel is sole minter (no crypto yet);
/// `nonce`/`expiry` exist now so L6 can HMAC the exact byte layout unchanged.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    pub node_id: u32,     // 0 = this node; >0 = remote (routing key)
    pub service_id: u32,  // which service/endpoint
    pub object_id: u64,   // object within the service
    pub permissions: u32, // CAP_SEND | CAP_RECV | ...
    pub _pad: u32,        // explicit pad -> deterministic layout across nodes
    pub nonce: u64,       // anti-replay (cosmetic locally; enforced at L6)
    pub expiry: u64,      // 0 = never; tick-based expiry later
}

impl Capability {
    pub const fn zero() -> Self {
        Self { node_id: 0, service_id: 0, object_id: 0,
               permissions: 0, _pad: 0, nonce: 0, expiry: 0 }
    }
    /// Mint a local endpoint cap (kernel is the authority).
    pub const fn local_endpoint(perms: u32) -> Self {
        Self { node_id: 0, service_id: SERVICE_ECHO, object_id: 0,
               permissions: perms, _pad: 0, nonce: 0, expiry: 0 }
    }
}

/// Fixed-size, pointer-free, serializable message. The load-bearing shape.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IPCMessage {
    pub version: u32,
    pub msg_type: u32,
    pub src_cap: Capability,
    pub dst_cap: Capability,
    pub payload_len: u32,
    pub _pad: u32,
    pub payload: [u8; IPC_PAYLOAD_MAX],
    pub checksum: u64,
}

impl IPCMessage {
    pub const fn zero() -> Self {
        Self {
            version: 1, msg_type: 0,
            src_cap: Capability::zero(), dst_cap: Capability::zero(),
            payload_len: 0, _pad: 0,
            payload: [0u8; IPC_PAYLOAD_MAX], checksum: 0,
        }
    }
}

/// Integrity check over the fields that matter. NOT crypto — corruption
/// detection only; L6 replaces this with an HMAC over the on-wire bytes.
fn compute_checksum(msg: &IPCMessage) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a basis
    let mut fold = |b: u8| { h ^= b as u64; h = h.wrapping_mul(0x1_0000_0001_b3); };
    for b in msg.msg_type.to_le_bytes()    { fold(b); }
    for b in msg.payload_len.to_le_bytes() { fold(b); }
    let n = (msg.payload_len as usize).min(IPC_PAYLOAD_MAX);
    for &b in &msg.payload[..n] { fold(b); }
    h
}

#[derive(Clone, Copy)]
pub enum IpcError {
    Unmapped, NotUser, BadLength, NoPermission, QueueFull, QueueEmpty, RemoteUnreachable,
}
impl IpcError {
    /// Encode as the syscall's usize return (high-bit-set = error).
    fn as_ret(self) -> usize {
        let code = match self {
            IpcError::Unmapped => 1, IpcError::NotUser => 2, IpcError::BadLength => 3,
            IpcError::NoPermission => 4, IpcError::QueueFull => 5, IpcError::QueueEmpty => 6,
            IpcError::RemoteUnreachable => 7,
        };
        usize::MAX - code + 1 // -code as usize
    }
}

// ---- uaccess: touch user memory only after validating U ownership ----

/// Copy `len` bytes FROM a user VA into a kernel buffer, via the identity map.
/// Rejects any page lacking U|R. SUM stays off throughout.
unsafe fn copy_from_user(dst: &mut [u8], user_va: usize, len: usize) -> Result<(), IpcError> {
    if len > dst.len() { return Err(IpcError::BadLength); }
    let root = paging::kernel_root();
    let mut done = 0;
    while done < len {
        let va = user_va + done;
        let (pa, flags) = paging::translate(root, va).ok_or(IpcError::Unmapped)?;
        if flags & (PTE_U | PTE_R) != (PTE_U | PTE_R) { return Err(IpcError::NotUser); }
        let off = va & (PAGE_SIZE - 1);
        let n = core::cmp::min(len - done, PAGE_SIZE - off);
        core::ptr::copy_nonoverlapping(pa as *const u8, dst.as_mut_ptr().add(done), n);
        done += n;
    }
    Ok(())
}

/// Copy `len` bytes TO a user VA from a kernel buffer, via the identity map.
/// Rejects any page lacking U|W. SUM stays off throughout.
unsafe fn copy_to_user(user_va: usize, src: &[u8], len: usize) -> Result<(), IpcError> {
    if len > src.len() { return Err(IpcError::BadLength); }
    let root = paging::kernel_root();
    let mut done = 0;
    while done < len {
        let va = user_va + done;
        let (pa, flags) = paging::translate(root, va).ok_or(IpcError::Unmapped)?;
        if flags & (PTE_U | PTE_W) != (PTE_U | PTE_W) { return Err(IpcError::NotUser); }
        let off = va & (PAGE_SIZE - 1);
        let n = core::cmp::min(len - done, PAGE_SIZE - off);
        core::ptr::copy_nonoverlapping(src.as_ptr().add(done), pa as *mut u8, n);
        done += n;
    }
    Ok(())
}

// ---- Endpoint: a fixed, pointer-free ring of messages ----

struct Endpoint {
    msgs: [IPCMessage; ENDPOINT_DEPTH],
    head: usize, // dequeue here
    tail: usize, // enqueue here
    count: usize,
}

static mut ENDPOINT: Endpoint = Endpoint {
    msgs: [const { IPCMessage::zero() }; ENDPOINT_DEPTH],
    head: 0, tail: 0, count: 0,
};
// Single-hart, interrupts off -> plain static mut is safe for step A. When L4
// adds preemption/SMP this needs a proper lock; flagged, not forgotten.

fn send_local(msg: &IPCMessage) -> Result<(), IpcError> {
    unsafe {
        let ep = &raw mut ENDPOINT;
        if (*ep).count == ENDPOINT_DEPTH { return Err(IpcError::QueueFull); }
        let t = (*ep).tail;
        (*ep).msgs[t] = *msg;
        (*ep).tail = (t + 1) % ENDPOINT_DEPTH;
        (*ep).count += 1;
    }
    Ok(())
}

fn recv_local(out: &mut IPCMessage) -> Result<(), IpcError> {
    unsafe {
        let ep = &raw mut ENDPOINT;
        if (*ep).count == 0 { return Err(IpcError::QueueEmpty); }
        let h = (*ep).head;
        *out = (*ep).msgs[h];
        (*ep).head = (h + 1) % ENDPOINT_DEPTH;
        (*ep).count -= 1;
    }
    Ok(())
}

/// The distributed routing decision — present from day one.
fn send_remote(node_id: u32, _msg: &IPCMessage) -> Result<(), IpcError> {
    crate::kprintln!("[IPC] send_remote: node {} unreachable - networking is Layer 6", node_id);
    Err(IpcError::RemoteUnreachable)
}

fn route_send(msg: &IPCMessage) -> Result<(), IpcError> {
    if msg.dst_cap.node_id == 0 {
        send_local(msg)
    } else {
        send_remote(msg.dst_cap.node_id, msg)
    }
}

// ---- Syscall handlers (wired from trap::handle_syscall) ----
// ABI: a0=x10=regs[9], a1=x11=regs[10], a2=x12=regs[11]. Return in a0=regs[9].
// These set ONLY regs[9]; the outer handle_syscall advances sepc once.

/// SYS_SEND(a0=payload_va, a1=payload_len, a2=dst_node_id) -> 0 ok | err
pub fn sys_send(frame: &mut TrapFrame) {
    let buf_va = frame.regs[9];
    let len    = frame.regs[10];
    let node   = frame.regs[11] as u32;

    let ret = (|| -> Result<usize, IpcError> {
        if len > IPC_PAYLOAD_MAX { return Err(IpcError::BadLength); }

        let mut msg = IPCMessage::zero();
        msg.version  = 1;
        msg.msg_type = MSG_DATA;
        msg.src_cap  = Capability::local_endpoint(CAP_SEND);
        msg.dst_cap  = Capability::local_endpoint(CAP_SEND | CAP_RECV);
        msg.dst_cap.node_id = node; // routing key: 0 local, >0 remote
        msg.payload_len = len as u32;

        // Capability check: sender must hold SEND on the destination.
        if msg.dst_cap.permissions & CAP_SEND == 0 { return Err(IpcError::NoPermission); }

        // Pull the payload from validated user memory (U-owned pages only).
        unsafe { copy_from_user(&mut msg.payload, buf_va, len)?; }
        msg.checksum = compute_checksum(&msg);

        route_send(&msg)?;
        Ok(0)
    })();

    frame.regs[9] = match ret {
        Ok(v)  => { crate::kprintln!("[IPC] SEND ok: {} bytes -> node {} (queued)", len, node); v }
        Err(e) => { crate::kprintln!("[IPC] SEND failed"); e.as_ret() }
    };
}

/// SYS_RECV(a0=dst_va, a1=max_len) -> bytes_received | err
pub fn sys_recv(frame: &mut TrapFrame) {
    let dst_va  = frame.regs[9];
    let max_len = frame.regs[10];

    let ret = (|| -> Result<usize, IpcError> {
        let mut msg = IPCMessage::zero();
        recv_local(&mut msg)?;

        // Receiver must hold RECV on the endpoint.
        if msg.dst_cap.permissions & CAP_RECV == 0 { return Err(IpcError::NoPermission); }

        // Integrity check before handing bytes back.
        if compute_checksum(&msg) != msg.checksum { return Err(IpcError::BadLength); }

        let n = core::cmp::min(msg.payload_len as usize, max_len).min(IPC_PAYLOAD_MAX);
        unsafe { copy_to_user(dst_va, &msg.payload, n)?; }
        Ok(n)
    })();

    frame.regs[9] = match ret {
        Ok(n)  => { crate::kprintln!("[IPC] RECV ok: {} bytes delivered to user", n); n }
        Err(e) => { crate::kprintln!("[IPC] RECV failed (queue empty or bad msg)"); e.as_ret() }
    };
}
