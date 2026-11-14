//! memhub-mcp: persistent note memory with hybrid search, served over MCP.
#![deny(missing_docs)]

pub mod adapters;
pub mod app;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod domain;
pub mod error;
pub mod ipc;
pub mod mcp;
pub mod ports;
pub mod relay;
pub mod vectors;
pub mod wiring;
