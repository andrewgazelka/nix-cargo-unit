# Rustc Transitive Dependency Resolution

## Problem

When using nix-cargo-unit built rlibs, rustc fails with `E0463: can't find crate` errors for crates that have dependencies, even when the crate file exists and the rustc version matches.

## Investigation

### Initial Symptoms

```bash
# Works - crate with no dependencies
rustc --extern cfg_if=/nix/store/.../libcfg_if.rlib lib.rs  # SUCCESS

# Fails - crate with dependencies
rustc --extern http=/nix/store/.../libhttp.rlib lib.rs  # E0463: can't find crate
```

### Key Discovery

The difference is whether the rlib was built with `--extern` dependencies:

- **cfg_if** (no deps): Built without `--extern` flags, works when used
- **bytes** (has deps): Built with `--extern serde=...`, fails when used without serde -L paths
- **http** (has deps): Built with `--extern bytes=... --extern itoa=...`, fails

### Root Cause

When rustc loads a crate, it verifies all embedded dependency references by their SVH (Stable Version Hash). This verification requires **both**:

1. `-L dependency=path` for the directory containing the dependency
2. The dependency must be locatable via filename matching (e.g., `libbytes-68b6fa70566a774f.rlib`)

The issue: rustc needs `-L dependency` paths for the **FULL transitive closure**, not just the direct dependencies.

### Proof

```bash
# FAILS - only direct dep paths
rustc --extern http=/path/to/libhttp.rlib \
      -L dependency=/path/to/bytes/lib \
      -L dependency=/path/to/itoa/lib \
      lib.rs

# WORKS - full transitive closure including serde (bytes depends on serde)
rustc --extern http=/path/to/libhttp.rlib \
      -L dependency=/path/to/bytes/lib \
      -L dependency=/path/to/itoa/lib \
      -L dependency=/path/to/serde/lib \
      -L dependency=/path/to/serde_core/lib \
      -L dependency=/path/to/serde_derive/lib \
      -L dependency=/path/to/syn/lib \
      -L dependency=/path/to/quote/lib \
      -L dependency=/path/to/proc_macro2/lib \
      -L dependency=/path/to/unicode_ident/lib \
      lib.rs
```

## How Rustc Dependency Resolution Works

1. When building crate A with `--extern B=path/to/libB.rlib`:
   - Rustc embeds B's SVH in A's metadata
   - The SVH is computed from B's source, features, and dependencies

2. When loading crate A in another compilation:
   - Rustc reads A's metadata
   - For each embedded dependency reference (B), rustc must verify:
     - B exists in an -L search path
     - B's SVH matches the embedded reference
   - This verification happens recursively for all of B's dependencies

3. Failure mode:
   - If B depends on C, and C is not in any -L path
   - Rustc cannot verify B's dependency chain
   - Error: "can't find crate for 'A'" (misleading - actual issue is missing C)

## Fix for nix-cargo-unit

The `-L dependency` paths must include the **complete transitive closure** of all dependencies, not just direct deps. This is computed correctly in `nix_gen.rs` via `transitive_deps`, but any filtering or bugs in that computation would cause this failure.

## Debugging Tips

1. Use `RUSTC_LOG=rustc_metadata=info` to trace crate loading:
   ```bash
   RUSTC_LOG=rustc_metadata=info rustc --extern foo=... lib.rs 2>&1 | grep -E "resolving|falling back"
   ```

2. Look for "falling back to a load" messages - this means rustc couldn't find a crate in already-loaded crates and is searching -L paths

3. The error message "can't find crate for X" is often misleading - the actual missing crate may be a transitive dependency of X

## Related

- Rustc source: `compiler/rustc_metadata/src/locator.rs`
- SVH computation: `compiler/rustc_hir/src/def_path_hash_map.rs`
