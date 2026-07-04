//! libwofl — endpoint-aware userspace interface. Reply routing via first-class
//! endpoints: no more yield-after-send. Each program declares its reply
//! endpoint via init(); open/read/write use it as the return address.
#![allow(dead_code)]
use core::arch::asm;

pub const SYS_EXIT:  usize = 1;
pub const SYS_YIELD: usize = 2; // retained; unused now that endpoints route replies
pub const SYS_SEND:  usize = 10;
pub const SYS_RECV:  usize = 11;

// Well-known endpoints (MUST match kernel ipc.rs id scheme).
pub const EP_VFS: usize = 1;

#[inline(always)]
pub fn sys_exit(code: usize) -> ! {
    unsafe { asm!("ecall", in("a7") SYS_EXIT, in("a0") code, options(noreturn, nostack)); }
}
#[inline(always)]
pub fn sys_yield() {
    unsafe { asm!("ecall", in("a7") SYS_YIELD, options(nostack)); }
}

/// send(buf,len) to (node,dst_ep), telling the receiver our reply endpoint.
#[inline(always)]
pub fn sys_send(buf: *const u8, len: usize, node: u32, dst_ep: usize, reply_ep: usize) -> usize {
    let ret: usize;
    unsafe { asm!("ecall",
        in("a7") SYS_SEND,
        inlateout("a0") buf as usize => ret,
        in("a1") len, in("a2") node as usize, in("a3") dst_ep, in("a4") reply_ep,
        options(nostack)); }
    ret
}

/// recv on `ep` -> (bytes, sender_reply_ep). Blocks in-kernel if ep is empty.
#[inline(always)]
pub fn sys_recv(buf: *mut u8, maxlen: usize, ep: usize) -> (usize, usize) {
    let bytes: usize; let src: usize;
    unsafe { asm!("ecall",
        in("a7") SYS_RECV,
        inlateout("a0") buf as usize => bytes,
        inlateout("a1") maxlen => src,
        in("a2") ep,
        options(nostack)); }
    (bytes, src)
}

// ---- this program's reply endpoint (set once at startup) ----
static mut MY_EP: usize = 0;
pub fn init(my_ep: usize) { unsafe { *(&raw mut MY_EP) = my_ep; } }
fn my_ep() -> usize { unsafe { *(&raw const MY_EP) } }

// ---- Capability mirror (MUST byte-match kernel ipc::Capability, 40 bytes) ----
pub const CAP_SEND: u32 = 1 << 0;
pub const CAP_RECV: u32 = 1 << 1;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Capability {
    pub node_id: u32, pub service_id: u32, pub object_id: u64,
    pub permissions: u32, pub _pad: u32, pub nonce: u64, pub expiry: u64,
}
impl Capability {
    pub const fn zero() -> Self {
        Self { node_id:0, service_id:0, object_id:0, permissions:0, _pad:0, nonce:0, expiry:0 }
    }
    pub const fn new(node_id: u32, service_id: u32, perms: u32) -> Self {
        Self { node_id, service_id, object_id:0, permissions:perms, _pad:0, nonce:0, expiry:0 }
    }
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const _ as *const u8,
            core::mem::size_of::<Capability>()) }
    }
    pub unsafe fn from_bytes(b: &[u8]) -> Capability {
        let mut c = Capability::zero();
        let n = core::mem::size_of::<Capability>().min(b.len());
        core::ptr::copy_nonoverlapping(b.as_ptr(), &mut c as *mut _ as *mut u8, n);
        c
    }
}

// ---- fd layer (endpoint-routed; no yield) ----
pub const MAX_FDS: usize = 8;
pub const VFS_NODE: u32 = 0;

#[derive(Clone, Copy)]
struct FdSlot { cap: Capability, open: bool }
static mut FD_TABLE: [FdSlot; MAX_FDS] = [FdSlot { cap: Capability::zero(), open: false }; MAX_FDS];

/// Resolve `path` via the VFS: send request to EP_VFS (reply = our endpoint),
/// then recv the cap on our endpoint. The kernel's block/wake handles handoff -
/// NO yield. Returns the fd, or usize::MAX on error.
pub fn open(path: &[u8]) -> usize {
    let me = my_ep();
    sys_send(path.as_ptr(), path.len(), VFS_NODE, EP_VFS, me);
    let mut buf = [0u8; 256];
    let (n, _src) = sys_recv(buf.as_mut_ptr(), buf.len(), me);
    let cap = unsafe { Capability::from_bytes(&buf[..n]) };
    if cap.permissions == 0 { return usize::MAX; }
    unsafe {
        let t = &mut *(&raw mut FD_TABLE);
        for (i, slot) in t.iter_mut().enumerate() {
            if !slot.open { *slot = FdSlot { cap, open: true }; return i; }
        }
    }
    usize::MAX
}

fn fd_cap(fd: usize) -> Option<Capability> {
    if fd >= MAX_FDS { return None; }
    unsafe { let t = &*(&raw const FD_TABLE); if t[fd].open { Some(t[fd].cap) } else { None } }
}

/// write(fd,buf) = send routed by the fd's capability (node + service endpoint).
pub fn write(fd: usize, buf: &[u8]) -> usize {
    match fd_cap(fd) {
        Some(c) => sys_send(buf.as_ptr(), buf.len(), c.node_id, c.service_id as usize, my_ep()),
        None => usize::MAX,
    }
}

/// read(fd,buf) = recv on our endpoint.
pub fn read(fd: usize, buf: &mut [u8]) -> usize {
    match fd_cap(fd) {
        Some(_) => { let (n,_) = sys_recv(buf.as_mut_ptr(), buf.len(), my_ep()); n }
        None => usize::MAX,
    }
}

pub fn close(fd: usize) {
    if fd >= MAX_FDS { return; }
    unsafe { (&mut *(&raw mut FD_TABLE))[fd].open = false; }
}

core::arch::global_asm!(r#"
.section .text.start, "ax"
.global _start
_start:
    la   t0, __bss_start
    la   t1, __bss_end
1:  bgeu t0, t1, 2f
    sd   zero, 0(t0)
    addi t0, t0, 8
    j    1b
2:  call main
3:  j 3b
"#);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { sys_exit(255) }
