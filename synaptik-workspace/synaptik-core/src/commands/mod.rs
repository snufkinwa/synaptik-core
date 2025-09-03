// src/commands/mod.rs
pub mod init;           
mod api;                


pub use api::{Commands, EthosReport};

pub use init::{InitReport, ensure_initialized_once};
