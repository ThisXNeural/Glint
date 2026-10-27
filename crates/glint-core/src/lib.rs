//! Platform-independent core of the search tool.
//!
//! - [`apps`]: small list of installed apps with fuzzy, usage-aware ranking.
//! - [`files`]: Everything-style compact index of every file and folder name.

pub mod apps;
pub mod files;
pub mod rank;

pub use apps::{App, AppIndex};
pub use files::{FileIndex, FileHit, Kind};
