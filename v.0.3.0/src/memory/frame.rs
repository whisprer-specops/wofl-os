use super::{PAGE_SIZE, align_up};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of frames we can track (32K frames = 128MB with 4KB pages)
const MAX_FRAMES: usize = 32768;

/// Bitmap-based frame allocator
/// Each bit represents one 4KB frame: 0 = free, 1 = used
pub struct FrameAllocator {
    /// Bitmap of frame allocation status
    bitmap: [AtomicUsize; MAX_FRAMES / (core::mem::size_of::<usize>() * 8)],
    /// Start address of allocatable memory
    start_addr: usize,
    /// Total number of frames
    total_frames: usize,
    /// Next frame to check (hint for faster allocation)
    next_free: AtomicUsize,
}

/// Global frame allocator instance
static mut FRAME_ALLOCATOR: FrameAllocator = FrameAllocator {
    bitmap: [const { AtomicUsize::new(0) }; MAX_FRAMES / (core::mem::size_of::<usize>() * 8)],
    start_addr: 0,
    total_frames: 0,
    next_free: AtomicUsize::new(0),
};

impl FrameAllocator {
    /// Allocate a single physical frame (4KB page)
    /// Returns physical address of the frame, or None if out of memory
    #[allow(dead_code)]
    pub fn alloc(&self) -> Option<usize> {
        let start = self.next_free.load(Ordering::Relaxed);
        
        // Search for a free frame
        for i in 0..self.total_frames {
            let frame = (start + i) % self.total_frames;
            let word_idx = frame / (core::mem::size_of::<usize>() * 8);
            let bit_idx = frame % (core::mem::size_of::<usize>() * 8);
            
            let word = self.bitmap[word_idx].load(Ordering::Acquire);
            
            if (word & (1 << bit_idx)) == 0 {
                // Found a free frame! Try to claim it atomically
                let new_word = word | (1 << bit_idx);
                
                if self.bitmap[word_idx].compare_exchange(
                    word, 
                    new_word, 
                    Ordering::AcqRel, 
                    Ordering::Acquire
                ).is_ok() {
                    // Successfully allocated!
                    self.next_free.store((frame + 1) % self.total_frames, Ordering::Release);
                    let phys_addr = self.start_addr + (frame * PAGE_SIZE);
                    return Some(phys_addr);
                }
                // If compare_exchange failed, another core grabbed it - keep searching
            }
        }
        
        None // Out of memory!
    }
    
    /// Free a previously allocated frame. Panics LOUDLY on misuse:
    /// unaligned PA, PA outside the managed range, or double-free. A silent
    /// double-free means two owners of one frame later - a delayed, unrelated-
    /// looking corruption bug. We want the crash at the actual mistake.
    pub fn free(&self, pa: usize) {
        assert!(pa % PAGE_SIZE == 0, "free: unaligned PA");
        assert!(pa >= self.start_addr, "free: PA below managed range");
        let frame = (pa - self.start_addr) / PAGE_SIZE;
        assert!(frame < self.total_frames, "free: PA above managed range");
        let word_idx = frame / (core::mem::size_of::<usize>() * 8);
        let bit_idx = frame % (core::mem::size_of::<usize>() * 8);
        let prev = self.bitmap[word_idx].fetch_and(!(1 << bit_idx), Ordering::AcqRel);
        assert!(prev & (1 << bit_idx) != 0, "free: DOUBLE FREE detected");
        // Hint the allocator at the fresh hole for quick reuse.
        self.next_free.store(frame, Ordering::Release);
    }

    /// Get statistics about memory usage
    #[allow(dead_code)]
    pub fn stats(&self) -> (usize, usize) {
        let mut used = 0;
        
        for word in &self.bitmap {
            used += word.load(Ordering::Relaxed).count_ones() as usize;
        }
        
        (used, self.total_frames)
    }
}

/// Initialize the frame allocator
pub unsafe fn init(kernel_end: usize, memory_end: usize) {
    let allocator = &raw mut FRAME_ALLOCATOR;
    let start = align_up(kernel_end);
    let total_bytes = memory_end - start;
    let total_frames = total_bytes / PAGE_SIZE;
    
    (*allocator).start_addr = start;
    (*allocator).total_frames = total_frames.min(MAX_FRAMES);
    (*allocator).next_free.store(0, Ordering::Release);
    
    // Clear bitmap (all frames free initially)
    let bitmap_ptr = (*allocator).bitmap.as_ptr();
    for i in 0..(*allocator).bitmap.len() {
        (*bitmap_ptr.add(i)).store(0, Ordering::Release);
    }
}

/// Allocate a physical frame
#[allow(dead_code)]
pub fn alloc_frame() -> Option<usize> {
    unsafe {
        let allocator = &raw const FRAME_ALLOCATOR;
        (*allocator).alloc()
    }
}

/// Free a physical frame (panics on double-free / bad PA)
#[allow(dead_code)]
pub fn free_frame(pa: usize) {
    unsafe {
        let allocator = &raw const FRAME_ALLOCATOR;
        (*allocator).free(pa)
    }
}

/// Get memory statistics
#[allow(dead_code)]
pub fn get_stats() -> (usize, usize) {
    unsafe {
        let allocator = &raw const FRAME_ALLOCATOR;
        (*allocator).stats()
    }
}