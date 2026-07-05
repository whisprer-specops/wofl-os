// src/plic.rs — RISC-V Platform-Level Interrupt Controller (QEMU virt)
//
// The PLIC gathers external interrupt sources (each device = one IRQ line)
// and presents at most one at a time to a hart context via claim/complete.
// QEMU virt facts:
//   PLIC base 0x0c00_0000 (spans ~4 MiB of register space)
//   virtio-mmio slot N raises IRQ N+1  ->  our NIC in slot 7 = IRQ 8
//   hart 0 has context 0 = M-mode (OpenSBI's; NEVER touch) and
//               context 1 = S-mode (ours)
//
// Register map (offsets from base):
//   0x000000 + 4*irq       priority[irq]      (0 = never deliver; we use 1)
//   0x002000 + 0x80*ctx    enable bitmap for context ctx (bit irq)
//   0x200000 + 0x1000*ctx  threshold for ctx  (deliver only prio > threshold)
//   0x200004 + 0x1000*ctx  claim (read) / complete (write) for ctx
//
// THE classic PLIC bug, pre-flagged: after servicing, you MUST write the IRQ
// number back to the complete register. Miss it and the PLIC gates that
// source forever — you get exactly ONE interrupt per boot and a mystery.
//
// Mapping: 0x0c00_0000 + 0x0c20_0000 are covered by two LEAF_MMIO megapages
// added to map_kernel_into (so every process root replicates them — same
// invariant-#7-by-construction as the virtio region).

#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

pub const PLIC_BASE: usize = 0x0c00_0000;
/// S-mode context for hart 0. Context 0 is M-mode — OpenSBI owns it.
pub const S_CONTEXT: usize = 1;
/// virtio-mmio slot 7 -> IRQ 8 on QEMU virt.
pub const IRQ_VIRTIO_NET: u32 = 8;

const PRIORITY_BASE: usize = 0x0000;
const ENABLE_BASE: usize = 0x2000;
const ENABLE_STRIDE: usize = 0x80;
const CTX_BASE: usize = 0x20_0000;
const CTX_STRIDE: usize = 0x1000;

#[inline(always)]
fn w32(addr: usize, v: u32) { unsafe { write_volatile(addr as *mut u32, v) } }
#[inline(always)]
fn r32(addr: usize) -> u32 { unsafe { read_volatile(addr as *const u32) } }

/// Enable one IRQ for our S-mode context: priority 1, enable bit set,
/// threshold 0 (deliver anything with priority > 0).
pub fn enable_irq(irq: u32) {
    w32(PLIC_BASE + PRIORITY_BASE + 4 * irq as usize, 1);
    let en = PLIC_BASE + ENABLE_BASE + ENABLE_STRIDE * S_CONTEXT + 4 * (irq as usize / 32);
    w32(en, r32(en) | (1 << (irq % 32)));
    w32(PLIC_BASE + CTX_BASE + CTX_STRIDE * S_CONTEXT, 0); // threshold
    crate::kprintln!("[L6] PLIC: IRQ {} enabled for context {} (prio 1, threshold 0)", irq, S_CONTEXT);
}

/// Ask the PLIC which pending enabled IRQ fired. 0 = none/spurious.
/// Reading ALSO claims it: the PLIC won't re-present this source until we
/// complete() it. Claim and complete come in pairs, always.
pub fn claim() -> u32 {
    r32(PLIC_BASE + CTX_BASE + CTX_STRIDE * S_CONTEXT + 4)
}

/// Tell the PLIC we're done with this IRQ. THE mandatory pair to claim().
pub fn complete(irq: u32) {
    w32(PLIC_BASE + CTX_BASE + CTX_STRIDE * S_CONTEXT + 4, irq);
}
