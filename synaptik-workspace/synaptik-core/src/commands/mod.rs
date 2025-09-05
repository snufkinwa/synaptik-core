// src/commands/mod.rs
pub mod init;           
mod api;                

use serde::Serialize;


#[derive(Debug, Clone, Copy)]
pub enum Prefer {
    Auto,    
    Hot,
    Archive,
    Dag,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HitSource {
    Hot,
    Archive,
    Dag,
}

#[derive(Debug, Serialize)]
pub struct RecallResult {
    pub memory_id: String,
    pub content: String,
    pub source: HitSource,
}

#[inline]
pub(crate) fn bytes_to_string_owned(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
    }
}

pub use api::{Commands, EthosReport};

pub use init::{InitReport, ensure_initialized_once};
