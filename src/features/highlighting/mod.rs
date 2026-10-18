//! Language-independent highlight classes and styles. No theme, GUI, parser or
//! server types live here, so every source of highlighting can share them.
mod model;
mod style;

pub(crate) use self::model::{HighlightId, HighlightModifiers};
pub(crate) use self::style::{HighlightStyle, StyleTable};
