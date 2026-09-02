//! Subcommand implementations.

pub mod auth;
pub mod completions;
pub mod doctor;
pub mod erase;
pub mod hydrate;
pub mod init;
pub mod ls;
pub mod prune;
pub mod rm;
pub mod run;
pub mod snapshot;
pub mod status;
pub mod uninstall;

#[cfg(test)]
mod json_contract;
