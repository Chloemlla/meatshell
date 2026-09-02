// Examples are separate crates, so include the shared allocator type and wrap
// it with byte counters. The global allocator declaration below belongs only
// to this example binary; the application declares its own in main.rs.
use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "../src/allocator/mod.rs"]
mod allocator;

use allocator::{allocator_name, Allocator as SelectedAllocator};

struct TrackingAllocator<A>(A);

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static TOTAL_ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static TOTAL_DEALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.0.alloc(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            TOTAL_ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.0.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            TOTAL_ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(ptr, layout) };
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        TOTAL_DEALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { self.0.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            match new_size.cmp(&layout.size()) {
                std::cmp::Ordering::Greater => {
                    let delta = new_size - layout.size();
                    LIVE_BYTES.fetch_add(delta, Ordering::Relaxed);
                    TOTAL_ALLOCATED.fetch_add(delta, Ordering::Relaxed);
                }
                std::cmp::Ordering::Less => {
                    let delta = layout.size() - new_size;
                    LIVE_BYTES.fetch_sub(delta, Ordering::Relaxed);
                    TOTAL_DEALLOCATED.fetch_add(delta, Ordering::Relaxed);
                }
                std::cmp::Ordering::Equal => {}
            }
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL: TrackingAllocator<SelectedAllocator> = TrackingAllocator(SelectedAllocator);

fn tracked_bytes() -> usize {
    LIVE_BYTES.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::{TOTAL_ALLOCATED, TOTAL_DEALLOCATED};
    use std::sync::atomic::Ordering;

    #[test]
    fn allocates_and_releases_memory() {
        let allocated_before = TOTAL_ALLOCATED.load(Ordering::Relaxed);
        let deallocated_before = TOTAL_DEALLOCATED.load(Ordering::Relaxed);
        let mut values = Vec::with_capacity(1024);
        values.extend(0..1024);
        assert_eq!(values[0], 0);
        assert_eq!(values[1023], 1023);
        let allocated = TOTAL_ALLOCATED.load(Ordering::Relaxed) - allocated_before;
        assert!(allocated >= 4096);
        drop(values);
        let deallocated = TOTAL_DEALLOCATED.load(Ordering::Relaxed) - deallocated_before;
        assert!(deallocated >= 4096);
    }
}

fn main() {
    let before = tracked_bytes();
    let allocated_before = TOTAL_ALLOCATED.load(Ordering::Relaxed);
    let mut values = Vec::with_capacity(1024);
    values.extend(0..1024);
    let during = tracked_bytes();
    let allocated = TOTAL_ALLOCATED.load(Ordering::Relaxed) - allocated_before;
    drop(values);
    let after_drop = tracked_bytes();
    let released = TOTAL_DEALLOCATED.load(Ordering::Relaxed);
    println!("allocator={}", allocator_name());
    println!("live_bytes_before={before}");
    println!("live_bytes_during={during}");
    println!("live_bytes_after_drop={after_drop}");
    println!("allocated_bytes={allocated}");
    println!("total_released_bytes={released}");
}
