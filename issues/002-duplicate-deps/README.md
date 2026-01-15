# Issue 002: Duplicate Derivations for Same Crate

## Problem

When the same crate version appears multiple times in the unit graph with different
dependency chains, nix-cargo-unit creates separate derivations for each occurrence.
This causes "can't find crate" errors because rustc cannot reconcile the SVH mismatches.

## Root Cause

The identity hash includes dependency hashes (correctly), but Cargo's unit graph can
contain multiple entries for the same (pkg_id, features, profile) tuple if they appear
through different dependency paths. Each entry gets a different identity hash because
their dependency indices differ.

When generating `-L dependency=` paths, all these separate derivations are included,
creating multiple rlib files for the same crate version with different metadata hashes.
When rustc tries to load a crate, it reads the SVH values of dependencies from the
rlib metadata and tries to find matching dependencies in the `-L` paths. If multiple
versions exist, rustc may pick the wrong one or fail entirely.

## Example Error

```
error[E0463]: can't find crate for `axum`
 --> src/extract.rs:4:5
  |
4 | use axum::{
  |     ^^^^ can't find crate
```

Even though `--extern axum=/nix/store/...-axum-0.8.8/lib/libaxum-....rlib` points to
a valid file, rustc cannot load it because the transitive dependencies have mismatched
SVH values.

## Evidence

From the async_graphql_axum build log:

- bytes-1.11.0 appears 4 times with different store paths
- serde_json-1.0.148 appears 4 times
- axum-0.8.8 appears 3 times
- tokio-1.48.0 appears 3 times

Each occurrence has a different store hash because the identity hash includes
dependency hashes, and each occurrence has a different dependency chain.

## Solution

Deduplicate units before computing identity hashes. Units with the same:
- pkg_id
- target (name, kind, crate_types)
- features (sorted)
- profile
- mode
- platform

Should be merged into a single entry, with their dependency sets unioned.

Alternatively, when generating `-L` paths for transitive deps, ensure we only
include the exact derivations that were used when compiling each direct dependency,
not all possible versions.
