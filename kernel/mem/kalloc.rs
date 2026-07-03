//! Kernel heap: address-ordered first-fit free list with coalescing,
//! registered as the crate's `#[global_allocator]` so `Box`, `Vec`,
//! `String` et al. work inside the kernel.
//!
//! Blocks are 16-byte aligned and at least 16 bytes (one `Block`
//! header) long. Free blocks carry their header in-band; allocated
//! blocks carry no header (`dealloc` re-derives the size from the
//! `Layout`, which `GlobalAlloc` guarantees to match).
#![allow(unsafe_code)] // a heap allocator is raw pointer surgery by nature

use super::layout::PAGE_SIZE;
use super::page;
use crate::lock::SpinLock;
use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

const MIN_BLOCK: usize = 16;
/// Initial heap: 4 MiB carved from the page allocator at `init()`.
const INITIAL_HEAP_PAGES: usize = 1024;

#[repr(C)]
struct Block {
    size: usize,
    next: *mut Block,
}

struct FreeList {
    head: *mut Block,
}

// Safety: FreeList is only reachable through the SpinLock below.
unsafe impl Send for FreeList {}

/// Lock invariant: protects the entire free list. The grow path calls
/// `page::page_alloc` while holding it; that is safe because the page
/// allocator never allocates from this heap (no lock-order cycle).
static HEAP: SpinLock<FreeList> = SpinLock::new(FreeList {
    head: ptr::null_mut(),
});

struct KernelAllocator;

#[global_allocator]
static GLOBAL: KernelAllocator = KernelAllocator;

/// Effective block size for a layout: padded to 16 bytes, min 16.
fn block_size(layout: Layout) -> usize {
    core::cmp::max(MIN_BLOCK, (layout.size() + MIN_BLOCK - 1) & !(MIN_BLOCK - 1))
}

impl FreeList {
    /// Insert a free block, keeping the list address-ordered and
    /// merging with adjacent neighbours.
    unsafe fn insert(&mut self, addr: usize, size: usize) {
        let mut prev: *mut Block = ptr::null_mut();
        let mut cur = self.head;
        while !cur.is_null() && (cur as usize) < addr {
            prev = cur;
            cur = (*cur).next;
        }

        let block = addr as *mut Block;
        (*block).size = size;
        (*block).next = cur;

        // Merge forward.
        if !cur.is_null() && addr + size == cur as usize {
            (*block).size += (*cur).size;
            (*block).next = (*cur).next;
        }

        if prev.is_null() {
            self.head = block;
        } else if (prev as usize) + (*prev).size == addr {
            // Merge backward.
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        } else {
            (*prev).next = block;
        }
    }

    /// First-fit search for `size` bytes at `align` (both multiples of
    /// 16). Splits front padding and tail remainder back into the list.
    unsafe fn take(&mut self, size: usize, align: usize) -> *mut u8 {
        let mut prev: *mut Block = ptr::null_mut();
        let mut cur = self.head;
        while !cur.is_null() {
            let start = cur as usize;
            let aligned = (start + align - 1) & !(align - 1);
            let pad = aligned - start;
            if (*cur).size >= pad + size {
                let total = (*cur).size;
                let next = (*cur).next;
                // Unlink cur.
                if prev.is_null() {
                    self.head = next;
                } else {
                    (*prev).next = next;
                }
                // Return front padding and tail remainder.
                if pad > 0 {
                    self.insert(start, pad);
                }
                let tail = total - pad - size;
                if tail > 0 {
                    self.insert(aligned + size, tail);
                }
                return aligned as *mut u8;
            }
            prev = cur;
            cur = (*cur).next;
        }
        ptr::null_mut()
    }
}

unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = block_size(layout);
        let align = core::cmp::max(layout.align(), MIN_BLOCK);
        let mut heap = HEAP.lock();
        let p = heap.take(size, align);
        if !p.is_null() {
            return p;
        }
        // Grow: pull enough whole pages from the frame allocator.
        let grow_pages = core::cmp::max((size + align) / PAGE_SIZE + 1, 16);
        match page::page_alloc(grow_pages) {
            Some(mem) => {
                heap.insert(mem as usize, grow_pages * PAGE_SIZE);
                heap.take(size, align)
            }
            None => ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HEAP.lock().insert(ptr as usize, block_size(layout));
    }
}

/// Seed the heap with an initial region so early boot allocations do
/// not each hit the page allocator. Call once, after `page::init`.
pub fn init() {
    let mem = page::page_alloc(INITIAL_HEAP_PAGES)
        .expect("kalloc: not enough physical memory for initial kernel heap");
    unsafe {
        HEAP.lock().insert(mem as usize, INITIAL_HEAP_PAGES * PAGE_SIZE);
    }
}
