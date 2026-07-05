// src/virtio.rs — Layer 6: in-kernel virtio-net driver (modern mmio transport)
//
// L6a: probe + handshake through FEATURES_OK, MAC read.          [proven]
// L6b: virtqueue setup (RX=0, TX=1) + DRIVER_OK.                 [this file]
// L6c+: descriptors, TX/RX, PLIC interrupt, send_remote wiring.  [next]
//
// Access rules: all registers via volatile 32-bit ops; config space bytes.
// DMA rules: any write the device may act on is preceded by `fence rw, rw`
// (dma_fence) — Rust/compiler ordering means NOTHING to a DMA engine.

#![allow(dead_code)]

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};

// ---------------------------------------------------------------------------
// QEMU virt memory map facts
// ---------------------------------------------------------------------------

pub const VIRTIO_MMIO_BASE: usize = 0x1000_1000;
pub const VIRTIO_MMIO_STRIDE: usize = 0x1000;
pub const VIRTIO_MMIO_SLOTS: usize = 8;

// ---------------------------------------------------------------------------
// Registers — MODERN layout (VersionID == 2 enforced at probe)
// ---------------------------------------------------------------------------

const R_MAGIC: usize = 0x000;
const R_VERSION: usize = 0x004;
const R_DEVICE_ID: usize = 0x008;
const R_VENDOR_ID: usize = 0x00c;
const R_DEV_FEATURES: usize = 0x010;
const R_DEV_FEATURES_SEL: usize = 0x014;
const R_DRV_FEATURES: usize = 0x020;
const R_DRV_FEATURES_SEL: usize = 0x024;
const R_QUEUE_SEL: usize = 0x030;
const R_QUEUE_NUM_MAX: usize = 0x034;
const R_QUEUE_NUM: usize = 0x038;
const R_QUEUE_READY: usize = 0x044;
const R_QUEUE_NOTIFY: usize = 0x050;
const R_INT_STATUS: usize = 0x060;
const R_INT_ACK: usize = 0x064;
const R_STATUS: usize = 0x070;
const R_QUEUE_DESC_LO: usize = 0x080;
const R_QUEUE_DESC_HI: usize = 0x084;
const R_QUEUE_AVAIL_LO: usize = 0x090; // spec: QueueDriver
const R_QUEUE_AVAIL_HI: usize = 0x094;
const R_QUEUE_USED_LO: usize = 0x0a0;  // spec: QueueDevice
const R_QUEUE_USED_HI: usize = 0x0a4;
const R_CONFIG_GEN: usize = 0x0fc;
const R_CONFIG: usize = 0x100;

const MAGIC: u32 = 0x7472_6976;
const DEVICE_ID_NET: u32 = 1;

const ST_ACK: u32 = 1;
const ST_DRIVER: u32 = 2;
const ST_DRIVER_OK: u32 = 4;
const ST_FEATURES_OK: u32 = 8;
const ST_NEEDS_RESET: u32 = 64;
const ST_FAILED: u32 = 128;

const F_VERSION_1: u64 = 1 << 32;
const NET_F_MAC: u64 = 1 << 5;

// ---------------------------------------------------------------------------
// Virtqueue layout — ONE 4 KiB frame per queue, identity-mapped DRAM (PA==VA)
//
//   +0     descriptor table   64 × 16 B = 1024 B   (align 16 ✓ frame is 4096)
//   +1024  avail ring         6 + 64×2  =  134 B   (align 2  ✓)
//   +2048  used ring          6 + 64×8  =  518 B   (align 4  ✓)
//
// QUEUE_SIZE=64 chosen over device max (256): legal (pow2 ≤ max), ample for
// capability messages, small enough to eyeball in a dump when debugging.
// ---------------------------------------------------------------------------

pub const QUEUE_SIZE: usize = 64;
const VQ_AVAIL_OFF: usize = QUEUE_SIZE * 16; // 1024
const VQ_USED_OFF: usize = 2048;

pub const VQ_RX: u32 = 0; // device -> driver
pub const VQ_TX: u32 = 1; // driver -> device

/// Descriptor entry (spec §2.6.5). Device reads these via DMA.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VqDesc {
    pub addr: u64,  // buffer PA (== VA under identity map)
    pub len: u32,
    pub flags: u16, // 1=NEXT, 2=WRITE (device-writable, i.e. RX buffers)
    pub next: u16,
}

pub const DESC_F_NEXT: u16 = 1;
pub const DESC_F_WRITE: u16 = 2;

pub struct Virtqueue {
    pub frame: usize,     // PA of the queue's 4 KiB frame
    pub size: usize,      // QUEUE_SIZE
    pub last_used: u16,   // our cursor into the used ring (L6c consumes)
}

impl Virtqueue {
    #[inline(always)]
    pub fn desc_ptr(&self, i: usize) -> *mut VqDesc {
        (self.frame + i * 16) as *mut VqDesc
    }
    #[inline(always)]
    pub fn avail_flags(&self) -> *mut u16 { (self.frame + VQ_AVAIL_OFF) as *mut u16 }
    #[inline(always)]
    pub fn avail_idx(&self) -> *mut u16 { (self.frame + VQ_AVAIL_OFF + 2) as *mut u16 }
    #[inline(always)]
    pub fn avail_ring(&self, i: usize) -> *mut u16 {
        (self.frame + VQ_AVAIL_OFF + 4 + i * 2) as *mut u16
    }
    #[inline(always)]
    pub fn used_idx(&self) -> *const u16 { (self.frame + VQ_USED_OFF + 2) as *const u16 }
    #[inline(always)]
    pub fn used_elem(&self, i: usize) -> *const u32 {
        // used elem = { id: u32, len: u32 } — 8 B each, ring starts at +4
        (self.frame + VQ_USED_OFF + 4 + i * 8) as *const u32
    }
}

/// The one ordering primitive that matters here. Compiler/atomic ordering is
/// invisible to the device's DMA engine; only the ISA fence orders our stores
/// against its loads. Called before every write the device can act on.
#[inline(always)]
pub fn dma_fence() {
    unsafe { asm!("fence rw, rw", options(nostack, preserves_flags)) }
}

// ---------------------------------------------------------------------------
// MMIO primitives
// ---------------------------------------------------------------------------

#[inline(always)]
fn reg_read(base: usize, off: usize) -> u32 {
    unsafe { read_volatile((base + off) as *const u32) }
}
#[inline(always)]
fn reg_write(base: usize, off: usize, val: u32) {
    unsafe { write_volatile((base + off) as *mut u32, val) }
}
#[inline(always)]
fn cfg_read_u8(base: usize, off: usize) -> u8 {
    unsafe { read_volatile((base + R_CONFIG + off) as *const u8) }
}

// ---------------------------------------------------------------------------
// Driver state — single-hart static, same justification + same lock debt as
// ipc::ENDPOINTS (kernel critical sections non-preemptible). Ledger item.
// ---------------------------------------------------------------------------

pub struct VirtioNet {
    pub base: usize,
    pub mac: [u8; 6],
    pub negotiated: u64,
    pub rx: Virtqueue,
    pub tx: Virtqueue,
}

pub static mut NIC: Option<VirtioNet> = None;

// ---------------------------------------------------------------------------
// L6a: probe + handshake (proven; unchanged in substance)
// ---------------------------------------------------------------------------

pub fn probe_net() -> Option<usize> {
    for slot in 0..VIRTIO_MMIO_SLOTS {
        let base = VIRTIO_MMIO_BASE + slot * VIRTIO_MMIO_STRIDE;
        if reg_read(base, R_MAGIC) != MAGIC { continue; }
        let version = reg_read(base, R_VERSION);
        let dev_id = reg_read(base, R_DEVICE_ID);
        if dev_id == 0 { continue; }
        crate::kprintln!(
            "[L6] virtio slot {}: base=0x{:x} version={} device_id={} vendor=0x{:x}",
            slot, base, version, dev_id, reg_read(base, R_VENDOR_ID)
        );
        if dev_id == DEVICE_ID_NET {
            if version != 2 {
                crate::kprintln!(
                    "[L6] net device is LEGACY (v{}). QEMU line is missing \
                     -global virtio-mmio.force-legacy=false — fix invocation.",
                    version
                );
                return None;
            }
            return Some(base);
        }
    }
    None
}

fn handshake(base: usize) -> Result<(u64, [u8; 6]), &'static str> {
    reg_write(base, R_STATUS, 0);
    let mut spins = 0u32;
    while reg_read(base, R_STATUS) != 0 {
        spins += 1;
        if spins > 1_000_000 { return Err("device refused reset"); }
    }
    reg_write(base, R_STATUS, ST_ACK);
    reg_write(base, R_STATUS, ST_ACK | ST_DRIVER);

    reg_write(base, R_DEV_FEATURES_SEL, 0);
    let lo = reg_read(base, R_DEV_FEATURES) as u64;
    reg_write(base, R_DEV_FEATURES_SEL, 1);
    let hi = reg_read(base, R_DEV_FEATURES) as u64;
    let offered = (hi << 32) | lo;
    crate::kprintln!("[L6] device features: 0x{:016x}", offered);

    if offered & F_VERSION_1 == 0 { return Err("no VIRTIO_F_VERSION_1"); }
    let wanted = F_VERSION_1 | (offered & NET_F_MAC);
    reg_write(base, R_DRV_FEATURES_SEL, 0);
    reg_write(base, R_DRV_FEATURES, wanted as u32);
    reg_write(base, R_DRV_FEATURES_SEL, 1);
    reg_write(base, R_DRV_FEATURES, (wanted >> 32) as u32);

    reg_write(base, R_STATUS, ST_ACK | ST_DRIVER | ST_FEATURES_OK);
    let st = reg_read(base, R_STATUS);
    if st & ST_FEATURES_OK == 0 {
        reg_write(base, R_STATUS, ST_FAILED);
        return Err("device rejected feature subset");
    }
    if st & (ST_NEEDS_RESET | ST_FAILED) != 0 {
        return Err("NEEDS_RESET/FAILED during handshake");
    }

    let mut mac = [0u8; 6];
    if wanted & NET_F_MAC != 0 {
        loop {
            let g = reg_read(base, R_CONFIG_GEN);
            for (i, b) in mac.iter_mut().enumerate() { *b = cfg_read_u8(base, i); }
            if reg_read(base, R_CONFIG_GEN) == g { break; }
        }
    }
    Ok((wanted, mac))
}

// ---------------------------------------------------------------------------
// L6b: virtqueue setup + DRIVER_OK
// ---------------------------------------------------------------------------

fn setup_queue(base: usize, idx: u32) -> Result<Virtqueue, &'static str> {
    reg_write(base, R_QUEUE_SEL, idx);

    let max = reg_read(base, R_QUEUE_NUM_MAX);
    if max == 0 { return Err("queue does not exist (NumMax=0)"); }
    if (max as usize) < QUEUE_SIZE { return Err("device max below our QUEUE_SIZE"); }
    if reg_read(base, R_QUEUE_READY) != 0 { return Err("queue already ready (stale state?)"); }

    let frame = crate::memory::frame::alloc_frame().ok_or("OOM allocating virtqueue frame")?;
    // Zero the WHOLE frame: desc table, both rings, and the slack. The device
    // will read this memory; it must never see stale allocator contents.
    unsafe { core::ptr::write_bytes(frame as *mut u8, 0, 4096); }

    reg_write(base, R_QUEUE_NUM, QUEUE_SIZE as u32);

    let desc = frame as u64;
    let avail = (frame + VQ_AVAIL_OFF) as u64;
    let used = (frame + VQ_USED_OFF) as u64;
    reg_write(base, R_QUEUE_DESC_LO, desc as u32);
    reg_write(base, R_QUEUE_DESC_HI, (desc >> 32) as u32);
    reg_write(base, R_QUEUE_AVAIL_LO, avail as u32);
    reg_write(base, R_QUEUE_AVAIL_HI, (avail >> 32) as u32);
    reg_write(base, R_QUEUE_USED_LO, used as u32);
    reg_write(base, R_QUEUE_USED_HI, (used >> 32) as u32);

    // Publish: zeroed rings + addresses must be globally visible BEFORE the
    // device is told the queue is live. This fence is load-bearing.
    dma_fence();
    reg_write(base, R_QUEUE_READY, 1);

    crate::kprintln!(
        "[L6] queue {} ({}): max={} size={} frame @0x{:x} (desc/avail/used +0/+{}/+{})",
        idx, if idx == VQ_RX { "RX" } else { "TX" }, max, QUEUE_SIZE, frame,
        VQ_AVAIL_OFF, VQ_USED_OFF
    );
    Ok(Virtqueue { frame, size: QUEUE_SIZE, last_used: 0 })
}

// ---------------------------------------------------------------------------
// Bring-up entry — call BEFORE any process spawns (roots snapshot kernel maps;
// also keeps the frame-baseline print meaningful: baseline is now 5, not 3,
// because the two queue frames are driver-owned for the kernel's lifetime).
// ---------------------------------------------------------------------------

pub fn l6_bringup() {
    let base = match probe_net() {
        Some(b) => b,
        None => { crate::kprintln!("[L6] no virtio-net device found"); return; }
    };
    let (negotiated, mac) = match handshake(base) {
        Ok(x) => x,
        Err(e) => { crate::kprintln!("[L6] handshake FAILED: {}", e); return; }
    };
    crate::kprintln!(
        "[L6] MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let rx = match setup_queue(base, VQ_RX) {
        Ok(q) => q,
        Err(e) => { crate::kprintln!("[L6] RX queue setup FAILED: {}", e); return; }
    };
    let tx = match setup_queue(base, VQ_TX) {
        Ok(q) => q,
        Err(e) => { crate::kprintln!("[L6] TX queue setup FAILED: {}", e); return; }
    };

    // Queues stand — DRIVER_OK is now legal (spec §3.1.1). Device goes LIVE.
    reg_write(base, R_STATUS, ST_ACK | ST_DRIVER | ST_FEATURES_OK | ST_DRIVER_OK);
    let st = reg_read(base, R_STATUS);
    if st & ST_DRIVER_OK == 0 || st & (ST_NEEDS_RESET | ST_FAILED) != 0 {
        crate::kprintln!("[L6] DRIVER_OK FAILED: status=0x{:x}", st);
        return;
    }
    crate::kprintln!("[L6] DRIVER_OK set — status=0x{:x}, device is LIVE", st);
    crate::kprintln!("[L6] L6b complete — TX one frame is L6c");

    unsafe { NIC = Some(VirtioNet { base, mac, negotiated, rx, tx }); }
}
