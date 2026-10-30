//! Marshal protocol constants and a bounded reader for compiled modules.
pub mod v4;
pub mod v5;
pub mod v6;

#[cfg(feature = "read")]
pub mod reader;
