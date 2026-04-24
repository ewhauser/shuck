//! Lightweight allocation counters for benchmark examples.

use std::cell::RefCell;

/// Maximum nested measurement depth supported by [`measure`].
const MAX_MEASURE_DEPTH: usize = 8;

/// Allocation counters collected for one measured region.
#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    /// Number of successful allocation calls.
    pub allocation_count: u64,
    /// Number of successful reallocation calls.
    pub reallocation_count: u64,
    /// Sum of requested allocation sizes.
    pub total_allocated_bytes: u64,
    /// Sum of requested new reallocation sizes.
    pub total_reallocated_bytes: u64,
    /// Bytes still live at the end of the measured region.
    pub current_live_bytes: i64,
    /// Highest live byte count observed during the measured region.
    pub peak_live_bytes: u64,
}

impl Frame {
    fn on_alloc(&mut self, size: usize) {
        self.allocation_count += 1;
        self.total_allocated_bytes += size as u64;
        self.current_live_bytes += size as i64;
        self.peak_live_bytes = self
            .peak_live_bytes
            .max(self.current_live_bytes.max(0) as u64);
    }

    fn on_dealloc(&mut self, size: usize) {
        self.current_live_bytes -= size as i64;
    }

    fn on_realloc(&mut self, old_size: usize, new_size: usize) {
        self.reallocation_count += 1;
        self.total_reallocated_bytes += new_size as u64;
        self.current_live_bytes += new_size as i64 - old_size as i64;
        self.peak_live_bytes = self
            .peak_live_bytes
            .max(self.current_live_bytes.max(0) as u64);
    }
}

#[derive(Debug, Default)]
struct CounterState {
    depth: usize,
    frames: [Frame; MAX_MEASURE_DEPTH],
}

thread_local! {
    static COUNTER_STATE: RefCell<CounterState> = RefCell::new(CounterState::default());
}

/// Global allocator wrapper that records allocations made inside [`measure`].
pub struct CountingAllocator<A>(pub A);

unsafe impl<A: std::alloc::GlobalAlloc> std::alloc::GlobalAlloc for CountingAllocator<A> {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { self.0.alloc(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        record_dealloc(layout.size());
        unsafe { self.0.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { self.0.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            record_realloc(layout.size(), new_size);
        }
        new_ptr
    }
}

fn record_alloc(size: usize) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        if depth == 0 {
            return;
        }
        for frame in &mut state.frames[1..=depth] {
            frame.on_alloc(size);
        }
    });
}

fn record_dealloc(size: usize) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        if depth == 0 {
            return;
        }
        for frame in &mut state.frames[1..=depth] {
            frame.on_dealloc(size);
        }
    });
}

fn record_realloc(old_size: usize, new_size: usize) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        if depth == 0 {
            return;
        }
        for frame in &mut state.frames[1..=depth] {
            frame.on_realloc(old_size, new_size);
        }
    });
}

/// Measure allocations made while running `f`.
pub fn measure<T>(f: impl FnOnce() -> T) -> (Frame, T) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert!(
            state.depth + 1 < MAX_MEASURE_DEPTH,
            "measurement nesting too deep"
        );
        state.depth += 1;
        let depth = state.depth;
        state.frames[depth] = Frame::default();
    });

    let result = f();

    let frame = COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        let frame = state.frames[depth];
        state.frames[depth] = Frame::default();
        state.depth -= 1;
        frame
    });

    (frame, result)
}
