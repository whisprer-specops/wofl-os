//! Layer 2 — Sv39 virtual memory (+ Layer 3/4 additions).
//!
//! PMP is M-mode-only; woflOS is S-mode, so isolation is via satp -> Sv39 page
//! tables. Sv39 = 39-bit VA, 3-level walk, 4 KiB pages (+2 MiB/1 GiB megapages).
//! PTE: bits[9:0] flags, bits[53:10] PPN. A PTE with R=W=X=0 is a pointer to the
//! next level; any of R/W/X set makes it a leaf. The U bit gates U-mode access.

use super::frame::alloc_frame;
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

const PPN_SHIFT: usize = 12;

pub const PTE_V: usize = 1 << 0; // Valid
pub const PTE_R: usize = 1 << 1; // Readable
pub const PTE_W: usize = 1 << 2; // Writable
pub const PTE_X: usize = 1 << 3; // eXecutable
pub const PTE_U: usize = 1 << 4; // User-accessible
pub const PTE_G: usize = 1 << 5; // Global
pub const PTE_A: usize = 1 << 6; // Accessed
pub const PTE_D: usize = 1 << 7; // Dirty

// A/D set eagerly on every leaf: QEMU (no Svadu) faults if a leaf is reached
// with A clear (or D clear on a store). Harmless on real Svade hardware.
const LEAF_RAM:  usize = PTE_R | PTE_W | PTE_X | PTE_A | PTE_D; // kernel DRAM, S-only
const LEAF_MMIO: usize = PTE_R | PTE_W | PTE_A | PTE_D;         // device, no-exec

static KERNEL_ROOT: AtomicUsize = AtomicUsize::new(0);

/// Physical address of the kernel root table (0 before init()).
pub fn kernel_root() -> usize {
    KERNEL_ROOT.load(Ordering::Acquire)
}

/// The CURRENTLY ACTIVE root, read from satp. This is the running process's
/// address space — the correct target for uaccess translation, since a trap
/// handler runs with satp still set to the process that trapped. Always right,
/// no matter how many processes exist.
pub fn current_root() -> usize {
    let satp: usize;
    unsafe { asm!("csrr {s}, satp", s = out(reg) satp, options(nostack, nomem)); }
    (satp & ((1usize << 44) - 1)) << PPN_SHIFT // Sv39 satp PPN = bits[43:0]
}

unsafe fn zero_frame(pa: usize) {
    let p = pa as *mut u64;
    for i in 0..512 { p.add(i).write_volatile(0); }
}

unsafe fn next_level(table_pa: usize, vpn: usize) -> usize {
    let slot = (table_pa as *mut u64).add(vpn);
    let pte = slot.read_volatile() as usize;
    if pte & PTE_V != 0 {
        (pte >> 10) << PPN_SHIFT // existing child table
    } else {
        let child = alloc_frame().expect("paging: OOM allocating page table");
        zero_frame(child);
        slot.write_volatile((((child >> PPN_SHIFT) << 10) | PTE_V) as u64);
        child
    }
}

/// Map one 2 MiB megapage (leaf at level 1). va/pa must be 2 MiB-aligned.
pub unsafe fn map_2mb(root: usize, va: usize, pa: usize, flags: usize) {
    let vpn2 = (va >> 30) & 0x1FF;
    let vpn1 = (va >> 21) & 0x1FF;
    let l1 = next_level(root, vpn2);
    let leaf = (((pa >> PPN_SHIFT) << 10) | flags | PTE_V) as u64;
    (l1 as *mut u64).add(vpn1).write_volatile(leaf);
}

/// Map one 4 KiB page (leaf at level 0).
pub unsafe fn map_4k(root: usize, va: usize, pa: usize, flags: usize) {
    let vpn2 = (va >> 30) & 0x1FF;
    let vpn1 = (va >> 21) & 0x1FF;
    let vpn0 = (va >> 12) & 0x1FF;
    let l1 = next_level(root, vpn2);
    let l0 = next_level(l1, vpn1);
    let leaf = (((pa >> PPN_SHIFT) << 10) | flags | PTE_V) as u64;
    (l0 as *mut u64).add(vpn0).write_volatile(leaf);
}

/// Replicate the kernel's fixed mappings (UART MMIO + all DRAM identity, all
/// S-only, no U bit) into `root`. Shared by init() and per-process root
/// creation so EVERY address space can trap into the kernel and the kernel can
/// reach any physical frame — while user mode still can't touch kernel memory.
pub unsafe fn map_kernel_into(root: usize) {
    map_2mb(root, 0x1000_0000, 0x1000_0000, LEAF_MMIO); // uart8250
    // PLIC (L6d): priorities/enables in the low window, per-context
    // threshold+claim at +0x20_0000. Two megapages cover both.
    map_2mb(root, 0x0c00_0000, 0x0c00_0000, LEAF_MMIO);
    map_2mb(root, 0x0c20_0000, 0x0c20_0000, LEAF_MMIO);
    let mut pa = 0x8000_0000usize;
    while pa < 0x8800_0000 {
        map_2mb(root, pa, pa, LEAF_RAM);
        pa += 2 * 1024 * 1024;
    }
}

/// Allocate a fresh Sv39 root with the kernel mappings replicated in. The
/// caller adds per-process U-pages, then switch_to()s it. (Optimization for
/// later: pointer-share the kernel's L1 sub-tables instead of replicating.)
pub unsafe fn create_root() -> usize {
    let root = alloc_frame().expect("paging: OOM allocating process root");
    zero_frame(root);
    map_kernel_into(root);
    root
}

/// Load `root` into satp (Sv39) and flush the TLB. The instruction after this
/// keeps executing because `root` maps the kernel identically.
pub unsafe fn switch_to(root: usize) {
    let satp = (8usize << 60) | (root >> PPN_SHIFT);
    asm!("csrw satp, {s}", "sfence.vma", s = in(reg) satp, options(nostack));
}

/// Build the kernel address space and turn on Sv39 paging. Returns root PA.
pub unsafe fn init() -> usize {
    let root = alloc_frame().expect("paging: OOM allocating root page table");
    zero_frame(root);
    map_kernel_into(root);
    KERNEL_ROOT.store(root, Ordering::Release);
    switch_to(root);
    root
}

/// Walk `root` for `va`. Returns (physical_addr, leaf_flags) if mapped, else
/// None. Handles a leaf at any level (4K/2M/1G). Used by L3 uaccess (against
/// current_root()), and later L4 fork / L6 DMA setup.
pub unsafe fn translate(root: usize, va: usize) -> Option<(usize, usize)> {
    let vpn = [(va >> 12) & 0x1FF, (va >> 21) & 0x1FF, (va >> 30) & 0x1FF];
    let mut table = root;
    let mut level: i32 = 2;
    while level >= 0 {
        let pte = (table as *const u64).add(vpn[level as usize]).read_volatile() as usize;
        if pte & PTE_V == 0 { return None; }
        if pte & (PTE_R | PTE_W | PTE_X) != 0 {
            let ppn = pte >> 10;
            let off_bits = 12 + 9 * (level as usize);
            let off_mask = (1usize << off_bits) - 1;
            let pa = ((ppn << 12) & !off_mask) | (va & off_mask);
            return Some((pa, pte & 0x3FF));
        }
        table = (pte >> 10) << 12;
        level -= 1;
    }
    None
}

// ---- Layer 4: address-space teardown ----

use super::frame::free_frame;

/// Recursively free one page-table level. Frees: all child PT frames (every
/// table under a per-process root is owned by that root, including the
/// replicated kernel-side tables), and leaf TARGETS only for 4 KiB U-bit
/// leaves (the process's own code/stack frames). Kernel leaves (S-only, and
/// always 2 MiB megapages in this kernel) point at memory the process merely
/// REFERENCES - freeing those targets would hand the kernel's own RAM back to
/// the allocator. Belt and braces: target-free requires level==0 AND PTE_U.
unsafe fn destroy_table(table_pa: usize, level: usize) {
    for i in 0..512 {
        let pte = (table_pa as *const u64).add(i).read_volatile() as usize;
        if pte & PTE_V == 0 { continue; }
        if pte & (PTE_R | PTE_W | PTE_X) != 0 {
            // Leaf. Free the target only if it's a user-owned 4 KiB page.
            if level == 0 && (pte & PTE_U) != 0 {
                free_frame((pte >> 10) << PPN_SHIFT);
            }
        } else {
            // Pointer to a child table: recurse (depth <= 2), then free it.
            let child = (pte >> 10) << PPN_SHIFT;
            destroy_table(child, level - 1);
            free_frame(child);
        }
    }
}

/// Tear down an entire per-process address space, returning every owned frame
/// (page tables at all levels + U-bit leaf targets) to the allocator.
///
/// ⚠️ SAFETY: `root` MUST NOT be the active satp root. Freeing the page tables
/// the MMU is currently walking = the next alloc scribbles over live
/// translation state -> a delayed fault that looks unrelated. Callers switch
/// satp AWAY first (to the next process's root, or kernel_root()).
/// Never call on kernel_root() itself.
pub unsafe fn destroy_root(root: usize) {
    debug_assert!(root != current_root(), "destroy_root: tearing down the ACTIVE root");
    debug_assert!(root != kernel_root(), "destroy_root: tearing down the KERNEL root");
    destroy_table(root, 2);
    free_frame(root);
}
