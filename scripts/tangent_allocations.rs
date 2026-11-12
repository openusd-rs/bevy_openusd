use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
static ENABLED: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static UNMATCHED: AtomicBool = AtomicBool::new(false);

fn allocated(bytes: usize) {
    CALLS.fetch_add(1, Relaxed);
    REQUESTED.fetch_add(bytes, Relaxed);
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}

fn freed(bytes: usize) {
    if LIVE.fetch_sub(bytes, Relaxed) < bytes {
        UNMATCHED.store(true, Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if ENABLED.load(Relaxed) && !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if ENABLED.load(Relaxed) && !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ENABLED.load(Relaxed) {
            freed(layout.size());
        }
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, bytes) };
        if ENABLED.load(Relaxed) && !pointer.is_null() {
            freed(layout.size());
            allocated(bytes);
        }
        pointer
    }
}

#[derive(Debug)]
pub struct Stats {
    pub calls: usize,
    pub requested: usize,
    pub peak: usize,
}

pub fn measure(action: impl FnOnce()) -> Stats {
    assert!(!ENABLED.load(Relaxed));
    for value in [&CALLS, &REQUESTED, &LIVE, &PEAK] {
        value.store(0, Relaxed);
    }
    UNMATCHED.store(false, Relaxed);
    ENABLED.store(true, Relaxed);
    action();
    ENABLED.store(false, Relaxed);
    assert!(!UNMATCHED.load(Relaxed), "untracked storage freed");
    assert_eq!(LIVE.load(Relaxed), 0, "generation retained an allocation");
    Stats {
        calls: CALLS.load(Relaxed),
        requested: REQUESTED.load(Relaxed),
        peak: PEAK.load(Relaxed),
    }
}

pub fn self_test() {
    let result = measure(|| {
        let layout = Layout::from_size_align(128, 8).unwrap();
        let larger = Layout::from_size_align(256, 8).unwrap();
        unsafe {
            let a = std::hint::black_box(std::alloc::alloc(layout));
            assert!(!a.is_null());
            let b = std::hint::black_box(std::alloc::alloc_zeroed(layout));
            assert!(!b.is_null());
            let a = std::hint::black_box(std::alloc::realloc(a, layout, larger.size()));
            assert!(!a.is_null());
            std::alloc::dealloc(a, larger);
            std::alloc::dealloc(b, layout);
        }
    });
    assert_eq!(result.calls, 3);
    assert_eq!(result.requested, 512);
    assert_eq!(result.peak, 384);
}
