//! nix-cargo-unit: Per-unit incremental Rust compilation in Nix.
//!
//! This library provides tools for parsing cargo's unit graph and generating
//! Nix derivations for each compilation unit, enabling fine-grained caching.

pub mod build_script;
pub mod nix_gen;
pub mod proc_macro;
pub mod rustc_flags;
pub mod shell;
pub mod source_filter;
pub mod unit_graph;
