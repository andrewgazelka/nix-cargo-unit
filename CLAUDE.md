# nix-cargo-unit Development Notes

## CA Derivations - NEVER DISABLE
NEVER remove `contentAddressed = true` as a workaround. Always debug and fix the root cause.
We have a custom Nix at `~/Projects/nix` that we control - keep modifying it until CA works.

### Known CA Issues

**Ownership check failure (macOS multi-user)**: Outputs owned by one nixbld user but validated
by another. Fix is in `src/libstore/unix/build/derivation-builder.cc`.

**CA hash mismatch across Nix versions** (GitHub #9397): Different Nix versions compute different
CA hashes due to self-hash rewriting changes in commit 3ebe134 (Nix 2.17.0+). Symptoms:
- "ca hash mismatch importing path" errors
- Cached outputs from older Nix versions incompatible with newer versions
- Solution: Clean cache with `nix-collect-garbage -d` or use consistent Nix version

**Nightly rustc version mismatch**: When using nightly Rust with CA derivations, stale cached
outputs from previous nightlies can cause "can't find crate" or E0514 errors. This happens because:
- Rust nightly embeds the exact commit hash in rlib metadata
- Rustc refuses to load rlibs compiled with different nightly commits
- CA caching may serve old outputs that were built with yesterday's nightly
- The error "can't find crate for X" is often a misleading way of saying "X was compiled with
  incompatible rustc version"
- Solution: `nix-collect-garbage -d` then rebuild

**Rust nightly date vs build date**: `nightly-2026-01-12` contains the toolchain built on
2026-01-11 (nightlies are named by release date, not build date). The commit shown in
`rustc --version` will be from the day before the nightly name.

## Rustc Dependency Resolution

When generating rustc invocations:
- `--extern name=path` tells rustc which crate to use for direct `use foo::...` imports
- `-L dependency=path` provides search paths for transitive dependencies
- Direct dependencies ALWAYS need `--extern` - never skip it even if there are version conflicts
- Transitive deps (those only needed by your deps) are resolved via `-L` search based on SVH
- Rustc uses SVH (Stable Version Hash) embedded in rlib metadata to match dependencies

## Reference Code
Useful:
- ~/Projects/rust
- ~/Projects/cargo
- ~/Projects/nix (custom nix with CA fixes; make your own fix if it is not right)
