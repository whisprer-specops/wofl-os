//! Layer 3/5 — IPC & capabilities, now with FIRST-CLASS ENDPOINTS.
//!
//! Messages are ADDRESSED: `dst_cap.node_id` routes to a node (0 local, >0
//! remote), `dst_cap.service_id` selects one of N per-node endpoint queues.
//! The sender's reply address rides in `src_cap.service_id` (a local endpoint
//! it owns); the receiver reads it to answer the right client. This kills the
//! shared-FIFO self-delivery problem and enables multi-client services - and it
//! is EXACTLY the addressing Layer 6 needs for remote replies, one layer early.

use crate::memory::paging::{self, PTE_U, PTE_R, PTE_W};
use crate::memory::PAGE_SIZE;
use crate::trap::TrapFrame;

pub const IPC_PAYLOAD_MAX: usize = 256;
const ENDPOINT_DEPTH: usize = 4;

/// Number of per-node endpoint queues. Well-known ids for bring-up:
///   0 = null/reserved, 1 = VFS service, 2/3 = client reply endpoints,
///   4 = echo service. DEBT: userspace currently PICKS its own ids (no cap
///   enforcement) - Layer 7 makes holding the endpoint cap the authority.
pub const N_ENDPOINTS: usize = 8;

pub const CAP_SEND: u32 = 1 << 0;
pub const CAP_RECV: u32 = 1 << 1;
pub const MSG_DATA: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    pub node_id: u32,
    pub service_id: u32,
    pub object_id: u64,
    pub permissions: u32,
    pub _pad: u32,
    pub nonce: u64,
    pub expiry: u64,
}
impl Capability {
    pub const fn zero() -> Self {
        Self { node_id:0, service_id:0, object_id:0, permissions:0, _pad:0, nonce:0, expiry:0 }
    }
    /// Address a (node, endpoint) with permissions. The load-bearing constructor.
    pub const fn endpoint(node_id: u32, service_id: u32, perms: u32) -> Self {
        Self { node_id, service_id, object_id:0, permissions:perms, _pad:0, nonce:0, expiry:0 }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IPCMessage {
    pub version: u32,
    pub msg_type: u32,
    pub src_cap: Capability,   // sender's REPLY endpoint (the return address)
    pub dst_cap: Capability,   // destination (node, endpoint)
    pub payload_len: u32,
    pub _pad: u32,
    pub payload: [u8; IPC_PAYLOAD_MAX],
    pub checksum: u64,
}
impl IPCMessage {
    pub const fn zero() -> Self {
        Self { version:1, msg_type:0, src_cap:Capability::zero(), dst_cap:Capability::zero(),
               payload_len:0, _pad:0, payload:[0u8; IPC_PAYLOAD_MAX], checksum:0 }
    }
}

fn compute_checksum(msg: &IPCMessage) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut fold = |b: u8| { h ^= b as u64; h = h.wrapping_mul(0x1_0000_0001_b3); };
    for b in msg.msg_type.to_le_bytes()    { fold(b); }
    for b in msg.payload_len.to_le_bytes() { fold(b); }
    let n = (msg.payload_len as usize).min(IPC_PAYLOAD_MAX);
    for &b in &msg.payload[..n] { fold(b); }
    h
}

#[derive(Clone, Copy)]
pub enum IpcError {
    Unmapped, NotUser, BadLength, NoPermission, QueueFull, QueueEmpty, RemoteUnreachable, BadEndpoint,
}
impl IpcError {
    fn as_ret(self) -> usize {
        let code = match self {
            IpcError::Unmapped=>1, IpcError::NotUser=>2, IpcError::BadLength=>3,
            IpcError::NoPermission=>4, IpcError::QueueFull=>5, IpcError::QueueEmpty=>6,
            IpcError::RemoteUnreachable=>7, IpcError::BadEndpoint=>8,
        };
        usize::MAX - code + 1
    }
}

// ---- uaccess (unchanged: validate U ownership; SUM stays off) ----
unsafe fn copy_from_user(dst: &mut [u8], user_va: usize, len: usize) -> Result<(), IpcError> {
    if len > dst.len() { return Err(IpcError::BadLength); }
    let root = paging::current_root();
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
unsafe fn copy_to_user(user_va: usize, src: &[u8], len: usize) -> Result<(), IpcError> {
    if len > src.len() { return Err(IpcError::BadLength); }
    let root = paging::current_root();
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

// ---- Endpoint table: N fixed, pointer-free rings ----
struct Endpoint {
    msgs: [IPCMessage; ENDPOINT_DEPTH],
    head: usize, tail: usize, count: usize,
}
impl Endpoint {
    const fn new() -> Self {
        Self { msgs: [const { IPCMessage::zero() }; ENDPOINT_DEPTH], head:0, tail:0, count:0 }
    }
}
static mut ENDPOINTS: [Endpoint; N_ENDPOINTS] = [const { Endpoint::new() }; N_ENDPOINTS];
// Single-hart, interrupts off -> plain static mut is safe (kernel critical
// sections are non-preemptible). Needs a per-endpoint lock at SMP.

/// True if endpoint `ep` currently holds no messages. OOB -> treated non-empty
/// so the caller's bounds check (not a block) handles the error path.
pub fn endpoint_empty(ep: usize) -> bool {
    if ep >= N_ENDPOINTS { return false; }
    unsafe { (*(&raw const ENDPOINTS))[ep].count == 0 }
}

fn send_local(ep: usize, msg: &IPCMessage) -> Result<(), IpcError> {
    if ep >= N_ENDPOINTS { return Err(IpcError::BadEndpoint); }
    unsafe {
        let e = &mut (*(&raw mut ENDPOINTS))[ep];
        if e.count == ENDPOINT_DEPTH { return Err(IpcError::QueueFull); }
        let t = e.tail;
        e.msgs[t] = *msg;
        e.tail = (t + 1) % ENDPOINT_DEPTH;
        e.count += 1;
    }
    Ok(())
}

fn recv_local(ep: usize, out: &mut IPCMessage) -> Result<(), IpcError> {
    if ep >= N_ENDPOINTS { return Err(IpcError::BadEndpoint); }
    unsafe {
        let e = &mut (*(&raw mut ENDPOINTS))[ep];
        if e.count == 0 { return Err(IpcError::QueueEmpty); }
        let h = e.head;
        *out = e.msgs[h];
        e.head = (h + 1) % ENDPOINT_DEPTH;
        e.count -= 1;
    }
    Ok(())
}

fn send_remote(node_id: u32, msg: &IPCMessage) -> Result<(), IpcError> {
    // L6e: the seam closes. IPCMessage is repr(C), fixed-size, pointer-free -
    // its byte image IS the wire format. No serialization layer needed; this
    // was the design bet made at Layer 3, cashing out now.
    let bytes = unsafe {
        core::slice::from_raw_parts(msg as *const IPCMessage as *const u8,
                                    core::mem::size_of::<IPCMessage>())
    };
    match crate::virtio::net_send_ipc(bytes) {
        Ok(()) => {
            crate::kprintln!("[IPC] send_remote: {} bytes -> node {} ON THE WIRE",
                bytes.len(), node_id);
            Ok(())
        }
        Err(e) => {
            crate::kprintln!("[IPC] send_remote: node {} unreachable ({})", node_id, e);
            Err(IpcError::RemoteUnreachable)
        }
    }
}

/// L6e: called from the RX interrupt path when a WOFL frame carries a
/// serialized IPCMessage. Deliver to the local endpoint it addresses.
///
/// SAFETY (touching ENDPOINTS from IRQ context): single hart, and the only
/// S-mode code that ever runs with SIE open is the boot/listen wfi loop,
/// which touches NO IPC state - so this cannot interleave with a half-done
/// endpoint operation. If a future SIE window ever wraps IPC-touching kernel
/// code, this needs a lock FIRST. (Ledger: same entry as ENDPOINTS/TABLE.)
pub fn deliver_remote(bytes: &[u8]) {
    const MSG: usize = core::mem::size_of::<IPCMessage>();
    const TAG: usize = crate::attest::TAG_LEN;
    if bytes.len() < MSG + TAG {
        crate::kprintln!("[IPC] remote: short frame ({} bytes, need {}) - dropped",
            bytes.len(), MSG + TAG);
        return;
    }
    // L6f: HMAC verify BEFORE anything else. Runs in IRQ context (SIE=0);
    // constant-time compare inside attest::verify. Outermost gate deliberately
    // - fail-open ordering would leak "node accepted frame far enough to X".
    // Mismatch is silent to the requester (no reply) and noisy locally.
    let msg_bytes = &bytes[..MSG];
    let mut received_tag = [0u8; TAG];
    received_tag.copy_from_slice(&bytes[MSG..MSG + TAG]);
    if !crate::attest::verify(msg_bytes, &received_tag) {
        crate::kprintln!("[IPC] remote: HMAC verify FAILED - dropped ({} bytes)",
            bytes.len());
        return;
    }
    let mut msg = IPCMessage::zero();
    unsafe {
        core::ptr::copy_nonoverlapping(msg_bytes.as_ptr(),
            &mut msg as *mut IPCMessage as *mut u8, MSG);
    }
    // mcast is a party line: everyone hears everything. Not ours -> silence.
    if msg.dst_cap.node_id != crate::NODE_ID { return; }

    // Checksum was computed on the sender with the field still ZERO, then
    // stored into it. Recomputing naively would include the populated field
    // and ALWAYS mismatch - zero, recompute, compare. (Works whether or not
    // compute_checksum skips the field internally.)
    let received = msg.checksum;
    msg.checksum = 0;
    let computed = compute_checksum(&msg);
    msg.checksum = received;
    if computed != received {
        crate::kprintln!("[IPC] remote: checksum mismatch - dropped");
        return;
    }

    let ep = msg.dst_cap.service_id as usize;
    match send_local(ep, &msg) {
        Ok(()) => {
            crate::process::wake_endpoint(ep);
            let n = core::cmp::min(msg.payload_len as usize, 24);
            if let Ok(s) = core::str::from_utf8(&msg.payload[..n]) {
                crate::kprintln!(
                    "[IPC] REMOTE DELIVERED: {} bytes -> ep {} (from node {} ep {}) payload \"{}\"",
                    msg.payload_len, ep, msg.src_cap.node_id, msg.src_cap.service_id, s);
            } else {
                crate::kprintln!("[IPC] REMOTE DELIVERED: {} bytes -> ep {} (from node {} ep {})",
                    msg.payload_len, ep, msg.src_cap.node_id, msg.src_cap.service_id);
            }
        }
        Err(_) => crate::kprintln!("[IPC] remote: ep {} rejected (full/bad) - dropped", ep),
    }
}

fn route_send(msg: &IPCMessage) -> Result<(), IpcError> {
    if msg.dst_cap.node_id == 0 {
        send_local(msg.dst_cap.service_id as usize, msg)
    } else {
        send_remote(msg.dst_cap.node_id, msg)
    }
}

// ---- Syscall handlers ----
// ABI: a0=regs[9], a1=regs[10], a2=regs[11], a3=regs[12], a4=regs[13].

/// SYS_SEND(a0=buf, a1=len, a2=dst_node, a3=dst_endpoint, a4=reply_endpoint) -> 0|err
pub fn sys_send(frame: &mut TrapFrame) {
    let buf_va = frame.regs[9];
    let len    = frame.regs[10];
    let node   = frame.regs[11] as u32;
    let dst_ep = frame.regs[12] as u32;
    let rep_ep = frame.regs[13] as u32;

    let ret = (|| -> Result<usize, IpcError> {
        if len > IPC_PAYLOAD_MAX { return Err(IpcError::BadLength); }
        let mut msg = IPCMessage::zero();
        msg.version  = 1;
        msg.msg_type = MSG_DATA;
        msg.src_cap  = Capability::endpoint(crate::NODE_ID, rep_ep, CAP_SEND | CAP_RECV); // return address (REAL node id - L6e)
        msg.dst_cap  = Capability::endpoint(node, dst_ep, CAP_SEND | CAP_RECV);
        msg.payload_len = len as u32;
        if msg.dst_cap.permissions & CAP_SEND == 0 { return Err(IpcError::NoPermission); }
        unsafe { copy_from_user(&mut msg.payload, buf_va, len)?; }
        msg.checksum = compute_checksum(&msg);
        route_send(&msg)?;
        // Local delivery: wake anyone blocked on the DESTINATION endpoint.
        if node == 0 { crate::process::wake_endpoint(dst_ep as usize); }
        Ok(0)
    })();

    frame.regs[9] = match ret {
        Ok(v)  => { crate::kprintln!("[IPC] SEND ok: {} bytes -> node {} ep {} (reply ep {})",
                        len, node, dst_ep, rep_ep); v }
        Err(_) => { crate::kprintln!("[IPC] SEND failed (node {} ep {})", node, dst_ep);
                    IpcError::RemoteUnreachable.as_ret() }
    };
}

/// SYS_RECV(a0=buf, a1=maxlen, a2=endpoint) -> a0=bytes, a1=src_reply_endpoint | err.
/// Returns true if the caller BLOCKED (frame now holds the NEXT process).
pub fn sys_recv(frame: &mut TrapFrame) -> bool {
    let my_ep = frame.regs[11] as usize; // a2

    if my_ep >= N_ENDPOINTS {
        frame.regs[9] = IpcError::BadEndpoint.as_ret();
        return false;
    }
    // Block BEFORE touching frame result regs (block swaps in the next process).
    if endpoint_empty(my_ep) {
        crate::kprintln!("[IPC] RECV: endpoint {} empty - blocking caller", my_ep);
        crate::process::block_current(frame, my_ep);
        return true; // frame = NEXT process now. Touch NOTHING.
    }

    let dst_va  = frame.regs[9];
    let max_len = frame.regs[10];
    let mut src_ep: usize = 0;
    let ret = (|| -> Result<usize, IpcError> {
        let mut msg = IPCMessage::zero();
        recv_local(my_ep, &mut msg)?;
        if msg.dst_cap.permissions & CAP_RECV == 0 { return Err(IpcError::NoPermission); }
        if compute_checksum(&msg) != msg.checksum { return Err(IpcError::BadLength); }
        src_ep = msg.src_cap.service_id as usize; // sender's reply endpoint
        let n = core::cmp::min(msg.payload_len as usize, max_len).min(IPC_PAYLOAD_MAX);
        unsafe { copy_to_user(dst_va, &msg.payload, n)?; }
        Ok(n)
    })();

    match ret {
        Ok(n) => {
            crate::kprintln!("[IPC] RECV ok: {} bytes on ep {} (from ep {})", n, my_ep, src_ep);
            frame.regs[9]  = n;      // a0 = bytes
            frame.regs[10] = src_ep; // a1 = source reply endpoint
        }
        Err(e) => {
            crate::kprintln!("[IPC] RECV failed on ep {}", my_ep);
            frame.regs[9] = e.as_ret();
        }
    }
    false
}
