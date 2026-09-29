//! Headless core of the editor. Everything here is testable without a window or GPU.

pub mod autosave;
pub mod buffer;
pub mod cli;
pub mod config;
pub mod djot;
pub mod document;
pub mod editor;
pub mod export;
pub mod files;
pub mod finder;
pub mod format;
pub mod input;
pub mod motion;
pub mod pdf;
pub mod recent;
pub mod runs;
pub mod search;
pub mod style;
pub mod tree;
pub mod vim;
