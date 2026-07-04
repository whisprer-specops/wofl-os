//! Layer 2 — Sv39 virtual memory.
//!
//! WHY Sv39 and not PMP: PMP CSRs (pmpcfg*/pmpaddr*) are M-mode-only. woflOS
//! runs in S-mode under OpenSBI, so it *cannot* write PMP — that access faults.
//! The S-mode-native isolation mechanism is the `satp` CSR (S-mode-writable)
//! pointing at an Sv39 page table. That also buys per-process address spaces,
//! which Layers 3–7 want anyway. The 16 PMP entries in OpenSBI's banner are
//! OpenSBI's, not ours; Layer 1's "open memory" was its inherited config.
//!
//! Sv39 = 39-bit VA, 3-level walk, 4 KiB base pages (+2 MiB / 1 GiB megapages):
//!   VA[38:30] = VPN2  (1 GiB stride)   -> root table index
//!   VA[29:21] = VPN1  (2 MiB stride)   -> level-1 table index
//!   VA[20:12] = VPN0  (4 KiB stride)   -> level-0 table index
//!   VA[11:0]  = page offset
//! A leaf PTE at level 1 = 2 MiB megapage; at level 0 = 4 KiB page.
//! PTE layout: bits[9:0] = flags, bits[53:10] = PPN (= phys_addr >> 12).
//! A PTE with V=1 and R=W=X=0 is a *pointer* to the next level; a PTE with
//! any of R/W/X set is a *leaf*.

use super::frame::alloc_frame;
use core::arch::asm;
use core::sync::atomic::{AtomicUsize, Ordering};

const PPN_SHIFT: usize = 12;

// PTE flag bits.
pub const PTE_V: usize = 1 << 0; // Valid
pub const PTE_R: usize = 1 << 1; // Readable
pub const PTE_W: usize = 1 << 2; // Writable
pub const PTE_X: usize = 1 << 3; // eXecutable
pub const PTE_U: usize = 1 << 4; // User-accessible (the bit that gates U-mode)
pub const PTE_G: usize = 1 << 5; // Global
pub const PTE_A: usize = 1 << 6; // Accessed
pub const PTE_D: usize = 1 << 7; // Dirty

// QEMU does not auto-update A/D unless Svadu is negotiated — if a leaf PTE is
// reached with A (or D on a store) clear, it raises a page fault. We set A/D
// eagerly on every leaf so we never trip that. Harmless on real Svade HW too.
const LEAF_RAM: usize = PTE_R | PTE_W | PTE_X | PTE_A | PTE_D; // coarse, step 1
const LEAF_MMIO: usize = PTE_R | PTE_W | PTE_A | PTE_D;        // device, no-exec

/// Physical address of the kernel root page table, stashed for step 2
/// (adding per-process user mappings on top of this same table).
static KERNEL_ROOT: AtomicUsize = AtomicUsize::new(0);

/// Physical address of the active kernel root table (0 before init()).
pub fn kernel_root() -> usize {
    KERNEL_ROOT.load(Ordering::Acquire)
}

/// Zero a freshly allocated 4 KiB table frame (512 × u64).
unsafe fn zero_frame(pa: usize) {
    let p = pa as *mut u64;
    for i in 0..512 {
        p.add(i).write_volatile(0);
    }
}

/// Return the child table's physical address for `vpn` in `table_pa`,
/// allocating + linking a fresh table if the slot is empty.
/// Assumes any existing valid PTE here is a pointer (true in our setup).
unsafe fn next_level(table_pa: usize, vpn: usize) -> usize {
    let slot = (table_pa as *mut u64).add(vpn);
    let pte = slot.read_volatile() as usize;
    if pte & PTE_V != 0 {
        ((pte >> 10) << PPN_SHIFT) // existing child table
    } else {
        let child = alloc_frame().expect("paging: OOM allocating page table");
        zero_frame(child);
        slot.write_volatile((((child >> PPN_SHIFT) << 10) | PTE_V) as u64);
        child
    }
}

/// Map one 2 MiB megapage (leaf at level 1). `va`/`pa` must be 2 MiB-aligned.
pub unsafe fn map_2mb(root: usize, va: usize, pa: usize, flags: usize) {
    let vpn2 = (va >> 30) & 0x1FF;
    let vpn1 = (va >> 21) & 0x1FF;
    let l1 = next_level(root, vpn2);
    let leaf = (((pa >> PPN_SHIFT) << 10) | flags | PTE_V) as u64;
    (l1 as *mut u64).add(vpn1).write_volatile(leaf);
}

/// Map one 4 KiB page (leaf at level 0). Used from step 2 for user pages.
pub unsafe fn map_4k(root: usize, va: usize, pa: usize, flags: usize) {
    let vpn2 = (va >> 30) & 0x1FF;
    let vpn1 = (va >> 21) & 0x1FF;
    let vpn0 = (va >> 12) & 0x1FF;
    let l1 = next_level(root, vpn2);
    let l0 = next_level(l1, vpn1);
    let leaf = (((pa >> PPN_SHIFT) << 10) | flags | PTE_V) as u64;
    (l0 as *mut u64).add(vpn0).write_volatile(leaf);
}

/// Build the kernel address space and turn on Sv39 paging.
///
/// SAFETY INVARIANT: every physical address the kernel touches *after* the
/// `csrw satp` — its own .text (incl. the trap vector & this very code), its
/// stack, heap, frame pool, and UART MMIO — must be mapped, or we fault the
/// instant translation goes live. We satisfy that by identity-mapping all DRAM
/// (so PC and sp keep resolving to the same address, now via the table) plus
/// the UART page. Because the map is identity, the instruction fetched right
/// after `csrw satp` is at the same PA, so control flow survives the switch.
///
/// Returns the root table's physical address (for the caller to log).
pub unsafe fn init() -> usize {
    let root = alloc_frame().expect("paging: OOM allocating root page table");
    zero_frame(root);

    // UART / MMIO @ 0x1000_0000 — one 2 MiB megapage, R+W, supervisor-only,
    // NO execute. (virt's uart8250 lives at 0x1000_0000.)
    map_2mb(root, 0x1000_0000, 0x1000_0000, LEAF_MMIO);

    // All DRAM identity-mapped 0x8000_0000..0x8800_0000 (64 × 2 MiB), S-only.
    // Coarse R+W+X for bring-up ONLY — no U bit anywhere, so U-mode cannot
    // touch any of it (that's what breaks & then re-proves the Layer 1 test).
    // Step 2 replaces the kernel-image span with per-section 4 KiB flags.
    // NOTE: OpenSBI's PMP still independently guards its own 0x8000_0000..
    // 0x8005_ffff; we map it here for simplicity but never touch it, so the
    // PMP∧PTE intersection never denies us. Our kernel at 0x8020_0000 sits in
    // OpenSBI's open Region03.
    let mut pa = 0x8000_0000usize;
    while pa < 0x8800_0000 {
        map_2mb(root, pa, pa, LEAF_RAM);
        pa += 2 * 1024 * 1024;
    }

    KERNEL_ROOT.store(root, Ordering::Release);

    // satp = MODE(Sv39=8)<<60 | ASID(0) | PPN(root>>12). Set it, then
    // sfence.vma (rs1=rs2=x0 → flush everything) so the next fetch uses it.
    let satp = (8usize << 60) | (root >> PPN_SHIFT);
    asm!("csrw satp, {s}", "sfence.vma", s = in(reg) satp, options(nostack));

    root
}

// ---- Layer 3 addition: page-table walk (uaccess validation, later fork/DMA) ----

/// Walk the Sv39 table `root` for virtual address `va`. Returns
/// `(physical_addr, leaf_flags)` if mapped, else `None`. Handles a leaf at any
/// level (4 KiB / 2 MiB / 1 GiB), so it works for user 4K pages AND the kernel's
/// 2M identity megapages. Reusable by L3 uaccess, L4 fork/mmap, L6 DMA setup.
///
/// SAFETY: reads page-table memory via the identity map; `root` must be a live
/// Sv39 root (it is — `kernel_root()`), single-hart so no concurrent PT edits.
pub unsafe fn translate(root: usize, va: usize) -> Option<(usize, usize)> {
    let vpn = [(va >> 12) & 0x1FF, (va >> 21) & 0x1FF, (va >> 30) & 0x1FF];
    let mut table = root;
    let mut level: i32 = 2; // Sv39 walk starts at the root (level 2)
    while level >= 0 {
        let pte = (table as *const u64).add(vpn[level as usize]).read_volatile() as usize;
        if pte & PTE_V == 0 {
            return None; // slot empty -> unmapped
        }
        if pte & (PTE_R | PTE_W | PTE_X) != 0 {
            // Leaf. Reconstruct PA: high bits from PPN, low (12 + 9*level) bits
            // are the in-page offset taken from the VA (handles megapages).
            let ppn = pte >> 10;
            let off_bits = 12 + 9 * (level as usize);
            let off_mask = (1usize << off_bits) - 1;
            let pa = ((ppn << 12) & !off_mask) | (va & off_mask);
            return Some((pa, pte & 0x3FF)); // flags = low 10 bits
        }
        // Non-leaf: descend to the child table this PTE points at.
        table = (pte >> 10) << 12;
        level -= 1;
    }
    None
}
