//! Persisted interface scale percentages; `None` keeps automatic scaling.
pub const MIN: u16 = 80;
pub const MAX: u16 = 200;
pub const STEP: u16 = 10;

pub fn clamped(percent: Option<u16>) -> Option<u16> {
    percent.map(|value| value.clamp(MIN, MAX))
}

pub fn step(percent: Option<u16>, up: bool) -> Option<u16> {
    let value = clamped(percent).unwrap_or(100);
    clamped(Some(if up {
        value.saturating_add(STEP)
    } else {
        value.saturating_sub(STEP)
    }))
}
