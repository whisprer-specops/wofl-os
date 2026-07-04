pub mod frame;
pub mod heap;
pub mod paging;

/// Page size for RISC-V (4KB)
pub const PAGE_SIZE: usize = 4096;

/// Align address up to page boundary
pub const fn align_up(addr: usize) -> usize {
    (addr + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// Initialize the memory subsystem
pub unsafe fn init(kernel_end: usize, memory_end: usize) {
    // Initialize frame allocator
    frame::init(kernel_end, memory_end);
    
    // Initialize heap
    heap::init();
}