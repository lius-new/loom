//! Last device scale the renderer reported, for work that rasterizes off the UI
//! thread and needs one texel per physical pixel (Skia samples images nearest).
use std::cell::Cell;

// UI workers receive the scale in request snapshots, like font metrics.
// Separate UI/test threads must not overwrite each other's rendering context.
thread_local! {
    static OBSERVED: Cell<f32> = const { Cell::new(1.0) };
}

pub fn observe(factor: f32) {
    if factor.is_finite() && factor > 0.0 {
        OBSERVED.set(factor);
    }
}

/// `1.0` until a frame has been rendered.
pub fn observed() -> f32 {
    OBSERVED.get()
}
