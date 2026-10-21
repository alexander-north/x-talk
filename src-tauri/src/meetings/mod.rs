//! Meeting data and processing are deliberately independent of dictation history.
#[cfg(feature = "desktop")]
pub mod capture;
#[cfg(feature = "desktop")]
pub mod commands;
pub mod domain;
pub mod export;
pub mod jobs;
pub mod media;
pub mod providers;
pub mod store;
