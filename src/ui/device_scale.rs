//! Last device scale the renderer reported, for work that rasterizes off the UI
//! thread and needs one texel per physical pixel (Skia samples images nearest).
use std::sync::atomic::{AtomicU32, Ordering};

static OBSERVED: AtomicU32 = AtomicU32::new(0);

pub fn observe(factor: f32) {
    if factor.is_finite() && factor > 0.0 {
        OBSERVED.store(factor.to_bits(), Ordering::Relaxed);
    }
}

/// `1.0` until a frame has been rendered.
pub fn observed() -> f32 {
    match f32::from_bits(OBSERVED.load(Ordering::Relaxed)) {
        factor if factor > 0.0 => factor,
        _ => 1.0,
    }
}
