//! The twin engine: turn a day of positions around an airport into flights,
//! runway operations and events. Pure logic, no I/O; builds for wasm32.

pub mod classify;
pub mod config;
pub mod day;
pub mod detect;
pub mod flights;
pub mod geo;
pub mod height;
#[cfg(test)]
pub mod testkit;
