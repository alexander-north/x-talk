#[cfg(feature = "desktop")]
pub mod app;
#[cfg(feature = "desktop")]
pub mod capture;
#[cfg(feature = "desktop")]
pub mod commands;
pub mod credentials;
pub mod cues;
pub mod diagnostics;
pub mod domain;
pub mod jobs;
pub mod media;
pub mod meetings;
pub mod providers;
pub mod realtime;
pub mod sounds;
pub mod spectrum;
pub mod storage;

#[cfg(all(feature = "test-providers", not(debug_assertions)))]
compile_error!("test-providers is restricted to debug acceptance builds");
#[cfg(all(feature = "test-providers", debug_assertions))]
pub(crate) mod test_support;

#[cfg(feature = "desktop")]
pub mod updates;
