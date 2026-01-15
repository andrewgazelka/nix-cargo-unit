# nix-cargo-unit Development Notes

## CA Derivations - NEVER DISABLE
NEVER remove `contentAddressed = true` as a workaround. Always debug and fix the root cause.
We have a custom Nix at `~/Projects/nix` that we control - keep modifying it until CA works.

Current CA issue: ownership check fails in multi-user mode because outputs are owned by
one nixbld user but validated by another. Fix is in `src/libstore/unix/build/derivation-builder.cc`.

## Reference Code
Useful:
~/Projects/rust
~/Projects/cargo
~/Projects/nix (custom nix with CA fixes)
