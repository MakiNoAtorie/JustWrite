pub mod cloud;
pub mod core;
pub mod jw;
pub mod mobile;
#[cfg(not(target_os = "ios"))]
pub mod export;
#[cfg(not(target_os = "ios"))]
pub mod fonts;
pub mod history;
#[cfg(not(target_os = "ios"))]
pub mod import;
pub mod platform;
#[cfg(not(target_os = "ios"))]
pub mod render;
#[cfg(not(target_os = "ios"))]
pub mod ui;

pub const APP_NAME: &str = "JustWrite";
