//! Verification entry point for the same domain sources, without the application or GUI.
#![allow(dead_code)]
mod isolated;
pub(crate) use self::isolated::domain;
