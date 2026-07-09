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
    pub tx_buf: usize, // L6e: persistent TX staging frame
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
// L6c: transmit one hand-built Ethernet frame (polled, no interrupts yet)
// ---------------------------------------------------------------------------

/// virtio-net header (spec §5.1.6). With VERSION_1 this is ALWAYS 12 bytes —
/// num_buffers is present even though we declined MRG_RXBUF. Sending the
/// 10-byte legacy header shifts the whole frame by 2 and garbles the wire.
pub const VNET_HDR_LEN: usize = 12;

/// Our EtherType: IEEE 802 local-experimental 1. woflOS's slot on the wire.
pub const ETHERTYPE_WOFL: u16 = 0x88B5;
/// L7a control plane: HELLO discovery frames. Separate ethertype keeps the
/// L6f IPC frame byte-identical; rx_parse branches on this before the IPC path.
pub const ETHERTYPE_HELLO: u16 = 0x88B6;

/// Build hdr+frame into `buf` (PA==VA, device-reachable). Returns total len.
/// Layout: [12B zeroed vnet hdr][6B dst][6B src][2B ethertype BE][payload]
fn build_tx(buf: usize, src_mac: &[u8; 6], ethertype: u16, payload: &[u8]) -> usize {
    unsafe {
        core::ptr::write_bytes(buf as *mut u8, 0, VNET_HDR_LEN); // no offloads
        let eth = (buf + VNET_HDR_LEN) as *mut u8;
        for i in 0..6 { eth.add(i).write(0xFF); }                 // dst: broadcast
        for i in 0..6 { eth.add(6 + i).write(src_mac[i]); }       // src: our MAC
        eth.add(12).write((ethertype >> 8) as u8);              // ethertype,
        eth.add(13).write(ethertype as u8);                       // big-endian (caller picks)
        for (i, b) in payload.iter().enumerate() {
            eth.add(14 + i).write(*b);
        }
    }
    // Ethernet min payload is 46B (frame 60B before FCS); virtio devices pad,
    // but we pad ourselves so the wire length is OURS, not device behavior.
    let eth_len = core::cmp::max(14 + payload.len(), 60);
    VNET_HDR_LEN + eth_len
}

/// TX one frame and poll the used ring for completion. Returns used len.
pub fn tx_one(nic: &mut VirtioNet, tx_buf: usize, ethertype: u16, payload: &[u8]) -> Result<u32, &'static str> {
    let total = build_tx(tx_buf, &nic.mac, ethertype, payload);

    // Descriptor 0: the whole buffer, device READS it (no WRITE flag), no chain.
    unsafe {
        nic.tx.desc_ptr(0).write_volatile(VqDesc {
            addr: tx_buf as u64,
            len: total as u32,
            flags: 0,
            next: 0,
        });
        // Publish: descriptor contents + ring slot must be globally visible
        // BEFORE the avail index moves — the device DMAs the instant it sees
        // the bump. Two fences, both load-bearing.
        let idx = nic.tx.avail_idx().read_volatile();
        nic.tx.avail_ring(idx as usize % nic.tx.size).write_volatile(0); // desc id 0
        dma_fence();
        nic.tx.avail_idx().write_volatile(idx.wrapping_add(1));
        dma_fence();
    }
    reg_write(nic.base, R_QUEUE_NOTIFY, VQ_TX);

    // Poll the used ring — bounded, so a dead device fails loudly not silently.
    let mut spins = 0u64;
    loop {
        let used = unsafe { nic.tx.used_idx().read_volatile() };
        if used != nic.tx.last_used { break; }
        spins += 1;
        if spins > 100_000_000 { return Err("TX: used ring never advanced"); }
    }
    // Device wrote the used elem before bumping used_idx; fence our read side
    // so we observe them in that order too.
    dma_fence();
    let slot = nic.tx.last_used as usize % nic.tx.size;
    let (id, len) = unsafe {
        (nic.tx.used_elem(slot).read_volatile(),
         nic.tx.used_elem(slot).add(1).read_volatile())
    };
    nic.tx.last_used = nic.tx.last_used.wrapping_add(1);
    crate::kprintln!("[L6] TX complete: used id={} len={} ({} spins)", id, len, spins);
    Ok(len)
}


// ---------------------------------------------------------------------------
// L6d: RX path — pre-posted device-writable buffers + interrupt-driven drain
// ---------------------------------------------------------------------------

use core::sync::atomic::{AtomicU32, Ordering};

/// Frames received & parsed. Atomic because the IRQ handler increments it
/// while the boot thread polls it across the wfi window.
pub static RX_SEEN: AtomicU32 = AtomicU32::new(0);

pub const RX_BUFS: usize = 8;
pub const RX_BUF_LEN: usize = 512; // 8 x 512 = one 4 KiB frame exactly

/// Post RX_BUFS device-WRITABLE buffers. MUST run before any frame can
/// arrive (i.e. before our TX in the mcast-echo test) - an RX queue with no
/// posted buffers means the device silently drops incoming frames.
pub fn post_rx(nic: &mut VirtioNet, rx_frame: usize) {
    for i in 0..RX_BUFS {
        unsafe {
            nic.rx.desc_ptr(i).write_volatile(VqDesc {
                addr: (rx_frame + i * RX_BUF_LEN) as u64,
                len: RX_BUF_LEN as u32,
                flags: DESC_F_WRITE, // device writes INTO these - the TX mirror
                next: 0,
            });
            nic.rx.avail_ring(i).write_volatile(i as u16);
        }
    }
    dma_fence();
    unsafe { nic.rx.avail_idx().write_volatile(RX_BUFS as u16); }
    dma_fence();
    reg_write(nic.base, R_QUEUE_NOTIFY, VQ_RX);
    crate::kprintln!("[L6] RX: {} buffers posted ({}B each, frame @0x{:x})",
        RX_BUFS, RX_BUF_LEN, rx_frame);
}

/// Parse one received buffer: skip the 12-byte vnet header (present on RX
/// exactly as on TX), print the Ethernet triple, and if it is our EtherType,
/// the payload text.
fn rx_parse(nic: &mut VirtioNet, buf: usize, len: usize) {
    if len < VNET_HDR_LEN + 14 {
        crate::kprintln!("[L6] RX: runt ({} bytes) - ignored", len);
        return;
    }
    let eth = (buf + VNET_HDR_LEN) as *const u8;
    unsafe {
        let ethertype = ((eth.add(12).read() as u16) << 8) | eth.add(13).read() as u16;
        // L7a: source MAC's last byte is the sender's node id (QEMU sets
        // 52:54:00:00:00:0X per -device mac=; build_tx copies nic.mac into src).
        // UNTRUSTED HINT ONLY - it merely selects which session key to try;
        // verify_from still gates trust, so a lying MAC just fails verify.
        let src_node = eth.add(11).read() as usize;
        // L7a: the mcast socket loops our OWN frames back to us. A self-echoed
        // IPC frame is tagged under a peer key we don't hold for "ourself"
        // (on_hello rejects src_node==me), so verify_from would correctly drop
        // it - but noisily, as a "verify FAILED" line that looks like an attack
        // during later debugging. Drop self-origin frames HERE, before any
        // branch: self-echo is a link property, not an IPC concern. After this,
        // a "verify FAILED" line means a REAL auth failure - which is exactly
        // what the L7a negative test wants it to mean. HELLO self-echo is
        // already handled inside on_hello, but dropping here covers both planes
        // uniformly and saves the wasted parse.
        if src_node == crate::NODE_ID as usize {
            return;
        }
        crate::kprintln!(
            "[L6] RX: {} bytes dst={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} \
src={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} type=0x{:04x}",
            len - VNET_HDR_LEN,
            eth.read(), eth.add(1).read(), eth.add(2).read(),
            eth.add(3).read(), eth.add(4).read(), eth.add(5).read(),
            eth.add(6).read(), eth.add(7).read(), eth.add(8).read(),
            eth.add(9).read(), eth.add(10).read(), eth.add(11).read(),
            ethertype
        );
        // L7a: HELLO discovery frames (control plane). Handled before the IPC
        // branch. Payload [subtype 1B][pubkey 32B]. on_hello runs the allowlist
        // check + DH; if it says this was an allowlisted REQUEST, we REPLY on
        // the SAME nic borrow (no NIC re-grab - that would alias &mut = UB).
        if ethertype == ETHERTYPE_HELLO && len >= VNET_HDR_LEN + 14 + 1 + 32 {
            let pl = eth.add(14);
            let subtype = pl.read();
            let mut pubkey = [0u8; 32];
            for i in 0..32 { pubkey[i] = pl.add(1 + i).read(); }
            if crate::attest::on_hello(subtype, src_node, &pubkey) {
                if let Err(e) = hello_tx(nic, crate::attest::HELLO_REPLY) {
                    crate::kprintln!("[L7a] HELLO reply TX failed: {}", e);
                }
            }
            RX_SEEN.fetch_add(1, Ordering::SeqCst);
            return;
        }
        // L6e: magic-prefixed frames carry a serialized IPCMessage.
        if ethertype == ETHERTYPE_WOFL && len >= VNET_HDR_LEN + 14 + 4 + crate::attest::TAG_LEN + core::mem::size_of::<crate::ipc::IPCMessage>() {
            let pl = eth.add(14);
            const MSG: usize = core::mem::size_of::<crate::ipc::IPCMessage>();
            const TAG: usize = crate::attest::TAG_LEN;
            if pl.read() == b'W' && pl.add(1).read() == b'O'
                && pl.add(2).read() == b'F' && pl.add(3).read() == b'L'
                && len - VNET_HDR_LEN - 14 - 4 >= MSG + TAG {
                // L6f: body now carries [IPCMessage MSG][BLAKE3 tag TAG] -
                // deliver_remote splits the two internally.
                let body = core::slice::from_raw_parts(pl.add(4), MSG + TAG);
                crate::ipc::deliver_remote(src_node, body);
                RX_SEEN.fetch_add(1, Ordering::SeqCst);
                return;
            }
        }
        if ethertype == ETHERTYPE_WOFL {
            let pl = eth.add(14);
            let pl_len = core::cmp::min(len - VNET_HDR_LEN - 14, 32);
            let mut txt = [0u8; 32];
            for i in 0..pl_len { txt[i] = pl.add(i).read(); }
            // trim trailing padding zeros for the print
            let end = txt.iter().position(|&b| b == 0).unwrap_or(pl_len);
            if let Ok(s) = core::str::from_utf8(&txt[..end]) {
                crate::kprintln!("[L6] RX payload: \"{}\"", s);
            }
        }
    }
    RX_SEEN.fetch_add(1, Ordering::SeqCst);
}

/// Called from the trap handler's code-9 arm, BETWEEN plic::claim() and
/// plic::complete(). Ack the device, drain the RX used ring, repost each
/// buffer so the ring never starves.
pub fn handle_irq() {
    unsafe {
        let nic = match NIC.as_mut() { Some(n) => n, None => return };
        // Device-level ack (distinct from the PLIC-level complete): read
        // InterruptStatus, write it back to InterruptACK. Skip this and the
        // device's line stays asserted -> re-fires forever.
        let int = reg_read(nic.base, R_INT_STATUS);
        reg_write(nic.base, R_INT_ACK, int);

        loop {
            dma_fence(); // order our used-ring reads after device's writes
            let used = nic.rx.used_idx().read_volatile();
            if used == nic.rx.last_used { break; }
            let slot = nic.rx.last_used as usize % nic.rx.size;
            let id = nic.rx.used_elem(slot).read_volatile() as usize;
            let len = nic.rx.used_elem(slot).add(1).read_volatile() as usize;
            nic.rx.last_used = nic.rx.last_used.wrapping_add(1);

            let buf = nic.rx.desc_ptr(id).read_volatile().addr as usize;
            rx_parse(nic, buf, len);

            // Repost the same descriptor - ring stays full forever.
            let idx = nic.rx.avail_idx().read_volatile();
            nic.rx.avail_ring(idx as usize % nic.rx.size).write_volatile(id as u16);
            dma_fence();
            nic.rx.avail_idx().write_volatile(idx.wrapping_add(1));
            dma_fence();
            reg_write(nic.base, R_QUEUE_NOTIFY, VQ_RX);
        }
    }
}


// ---------------------------------------------------------------------------
// L6e: IPCMessage on the wire - [eth 0x88B5][b"WOFL"][message bytes]
// ---------------------------------------------------------------------------

/// Wire fit: vnet hdr + eth hdr + magic + message must fit one RX buffer.
/// Bumping IPC_PAYLOAD_MAX (design target 4096) FAILS THIS BUILD instead of
/// silently truncating DMA - grow RX_BUF_LEN/buffer scheme together with it.
const _: () = assert!(
    VNET_HDR_LEN + 14 + 4 + core::mem::size_of::<crate::ipc::IPCMessage>() + crate::attest::TAG_LEN <= RX_BUF_LEN
);

/// L7a: TX one HELLO discovery frame. Payload [subtype 1B][my pubkey 32B] at
/// ETHERTYPE_HELLO. Takes the ALREADY-HELD nic borrow (never re-grabs NIC -
/// two &mut to one static is UB; this is why rx_parse/handle_irq thread nic
/// down instead). REQUEST solicits+announces; REPLY answers and is terminal.
pub fn hello_tx(nic: &mut VirtioNet, subtype: u8) -> Result<(), &'static str> {
    if nic.tx_buf == 0 { return Err("TX buffer not allocated"); }
    let pk = crate::attest::my_pubkey_bytes();
    let mut payload = [0u8; 1 + 32];
    payload[0] = subtype;
    payload[1..].copy_from_slice(&pk);
    let buf = nic.tx_buf;
    tx_one(nic, buf, ETHERTYPE_HELLO, &payload).map(|_| ())
}

/// L7a: send one HELLO from BOOT context - i.e. when you are NOT already
/// holding the nic borrow. Grabs NIC.as_mut() for one scoped statement and
/// drops it. CALLER MUST HAVE SIE=0 when calling this: with interrupts off no
/// handle_irq can be mid-borrow, so this transient &mut is provably alone (two
/// &mut to the NIC static would be UB). The reply path in handle_irq does NOT
/// use this - it already holds nic, so it calls hello_tx directly. This is the
/// single sanctioned not-holding-nic entry point; that division is the whole
/// borrow-safety story for the HELLO path.
pub fn hello_tx_scoped(subtype: u8) -> Result<(), &'static str> {
    unsafe {
        let nic = NIC.as_mut().ok_or("NIC not initialised")?;
        hello_tx(nic, subtype)
    }
}

/// TX a serialized IPCMessage. Called from the syscall path (send_remote),
/// so it runs on the kernel trap stack - the ~370B staging array is fine
/// against 16 KiB. Polls TX completion like tx_one (interrupt-driven TX is
/// an optimisation for later; correctness first).
pub fn net_send_ipc(dst_node: usize, bytes: &[u8]) -> Result<(), &'static str> {
    const MSG: usize = core::mem::size_of::<crate::ipc::IPCMessage>();
    const TAG: usize = crate::attest::TAG_LEN;
    if bytes.len() != MSG { return Err("bad IPCMessage size"); }
    // L7a wire layout unchanged from L6f: ["WOFL" 4B][IPCMessage MSG][tag TAG].
    // The DIFFERENCE is the key: tag_for selects this peer's SESSION key, not a
    // global one. No session yet -> refuse to put an un-authenticated frame on
    // the wire (caller surfaces the error; discovery must precede remote IPC).
    let tag = match crate::attest::tag_for(dst_node, bytes) {
        Some(t) => t,
        None => return Err("no session key for dst node (HELLO not yet exchanged)"),
    };
    let mut payload = [0u8; 4 + MSG + TAG];
    payload[0..4].copy_from_slice(b"WOFL");
    payload[4..4 + MSG].copy_from_slice(bytes);
    payload[4 + MSG..].copy_from_slice(&tag);
    unsafe {
        let nic = NIC.as_mut().ok_or("NIC not initialised")?;
        if nic.tx_buf == 0 { return Err("TX buffer not allocated"); }
        let buf = nic.tx_buf;
        tx_one(nic, buf, ETHERTYPE_WOFL, &payload).map(|_| ())
    }
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

    unsafe { NIC = Some(VirtioNet { base, mac, negotiated, rx, tx, tx_buf: 0 }); }

    // L6d sequence. ORDER IS LOAD-BEARING:
    //   1. post RX buffers   (before any frame can arrive)
    //   2. PLIC + SEIE       (before the TX whose echo we want to hear)
    //   3. TX the hello      (mcast netdev loops it back at us)
    //   4. scoped SIE window (the ONLY S-mode moment interrupts may land)
    let tx_buf = match crate::memory::frame::alloc_frame() {
        Some(f) => f,
        None => { crate::kprintln!("[L6] OOM allocating TX buffer"); return; }
    };
    let rx_frame = match crate::memory::frame::alloc_frame() {
        Some(f) => f,
        None => { crate::kprintln!("[L6] OOM allocating RX buffers"); return; }
    };
    unsafe {
        if let Some(nic) = NIC.as_mut() {
            nic.tx_buf = tx_buf; // persist: send_remote TXes long after boot
            post_rx(nic, rx_frame);                          // 1
            crate::plic::enable_irq(crate::plic::IRQ_VIRTIO_NET); // 2
            crate::trap::enable_external();
            match tx_one(nic, tx_buf, ETHERTYPE_WOFL, b"woflOS node 0 says hello") { // 3
                Ok(_) => {}
                Err(e) => { crate::kprintln!("[L6] TX FAILED: {}", e); return; }
            }
        } else { return; }
    }
    // 4. Scoped listening window. The kernel-non-preemptible invariant says
    // sstatus.SIE stays 0 in S-mode - this is a DELIBERATE, BOUNDED, LOCAL
    // exception for the boot test only (no processes exist yet, so U-mode
    // delivery is not available). SIE on -> wfi until RX or bound -> SIE off.
    // In steady state RX interrupts land from U-mode like the timer does.
    {
        let mut waits = 0u32;
        unsafe { core::arch::asm!("csrs sstatus, {b}", b = in(reg) 1usize << 1); } // SIE ON
        while RX_SEEN.load(Ordering::SeqCst) == 0 {
            unsafe { core::arch::asm!("wfi"); }
            waits += 1;
            if waits > 10_000 { break; }
        }
        unsafe { core::arch::asm!("csrc sstatus, {b}", b = in(reg) 1usize << 1); } // SIE OFF - window closed
        if RX_SEEN.load(Ordering::SeqCst) > 0 {
            crate::kprintln!("[L6] L6d complete - kernel heard the wire (window: {} wfi wakes)", waits);
        } else {
            crate::kprintln!("[L6] L6d FAILED - no RX within window (is -netdev socket,mcast=... on the line?)");
        }
    }
}
