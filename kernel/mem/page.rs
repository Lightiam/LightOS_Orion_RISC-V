//! Page-frame allocator: a bitmap over all 4 KiB frames between
//! `_heap_start` and `_memory_end`.
#![allow(unsafe_code)] // hands out raw frame pointers and zeroes them

use super::layout::{page_up, PAGE_SIZE, RAM_SIZE, RAM_START};
use crate::lock::SpinLock;

/// One bit per 4 KiB frame of RAM (128 MiB / 4 KiB = 32768 frames).
const MAX_FRAMES: usize = RAM_SIZE / PAGE_SIZE;
const BITMAP_WORDS: usize = MAX_FRAMES / 64;

/// Global page allocator. Lock invariant: protects the bitmap and the
/// counters; held only while scanning/flipping bits, never while the
/// caller uses the returned frames.
static ALLOCATOR: SpinLock<PageAllocator> = SpinLock::new(PageAllocator {
    bitmap: [u64::MAX; BITMAP_WORDS], // everything "used" until init()
    first_frame: 0,
    free_frames: 0,
});

struct PageAllocator {
    /// Bit set = frame in use (or reserved/kernel image).
    bitmap: [u64; BITMAP_WORDS],
    /// Frame index of the first allocatable page.
    first_frame: usize,
    free_frames: usize,
}

impl PageAllocator {
    fn frame_to_addr(&self, frame: usize) -> usize {
        RAM_START + frame * PAGE_SIZE
    }

    fn addr_to_frame(&self, addr: usize) -> usize {
        debug_assert!(addr >= RAM_START, "address below RAM");
        (addr - RAM_START) / PAGE_SIZE
    }

    fn is_used(&self, frame: usize) -> bool {
        self.bitmap[frame / 64] & (1 << (frame % 64)) != 0
    }

    fn set_used(&mut self, frame: usize) {
        self.bitmap[frame / 64] |= 1 << (frame % 64);
    }

    fn set_free(&mut self, frame: usize) {
        self.bitmap[frame / 64] &= !(1 << (frame % 64));
    }

    fn alloc(&mut self, n: usize) -> Option<usize> {
        if n == 0 || self.free_frames < n {
            return None;
        }
        // First-fit scan for n contiguous free frames.
        let mut run = 0;
        let mut start = self.first_frame;
        for frame in self.first_frame..MAX_FRAMES {
            if self.is_used(frame) {
                run = 0;
                start = frame + 1;
            } else {
                run += 1;
                if run == n {
                    for f in start..start + n {
                        self.set_used(f);
                    }
                    self.free_frames -= n;
                    return Some(self.frame_to_addr(start));
                }
            }
        }
        None
    }

    fn free(&mut self, addr: usize, n: usize) {
        let start = self.addr_to_frame(addr);
        for frame in start..start + n {
            debug_assert!(self.is_used(frame), "double free of page frame");
            self.set_free(frame);
        }
        self.free_frames += n;
    }
}

/// Mark every frame from `heap_start` to end of RAM as free. Everything
/// below (kernel image, boot stacks) stays reserved forever.
pub fn init(heap_start: usize) {
    let mut a = ALLOCATOR.lock();
    let first = a.addr_to_frame(page_up(heap_start));
    a.first_frame = first;
    for frame in first..MAX_FRAMES {
        a.set_free(frame);
    }
    a.free_frames = MAX_FRAMES - first;
}

/// Allocates `n` contiguous, zeroed 4 KiB pages.
/// Returns `None` if physical memory is exhausted.
pub fn page_alloc(n: usize) -> Option<*mut u8> {
    let addr = ALLOCATOR.lock().alloc(n)?;
    let ptr = addr as *mut u8;
    unsafe { core::ptr::write_bytes(ptr, 0, n * PAGE_SIZE) };
    Some(ptr)
}

/// Returns `n` pages starting at `ptr` to the allocator.
///
/// Safety contract (checked with debug_assert): `ptr` must come from
/// `page_alloc(n)` and not be freed twice.
pub fn page_free(ptr: *mut u8, n: usize) {
    ALLOCATOR.lock().free(ptr as usize, n);
}

/// Number of currently free 4 KiB frames (diagnostics).
pub fn free_frames() -> usize {
    ALLOCATOR.lock().free_frames
}
