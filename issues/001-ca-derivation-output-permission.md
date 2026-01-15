# CA-Derivation Output Permission Denied on Build Script Reuse

## Summary

When building with `contentAddressed = true`, build script output derivations fail with "Permission denied" errors when Nix attempts to reuse an existing CA output path. The generated build scripts try to `touch` files in an already-existing (read-only) store path.

## Environment

- **OS**: macOS (aarch64-darwin)
- **Nix version**: 2.31.2
- **nix-cargo-unit version**: 0.1.0
- **Experimental features**: `ca-derivations dynamic-derivations recursive-nix flakes nix-command`

## Steps to Reproduce

1. Enable CA-derivations in nix.conf:
   ```
   experimental-features = nix-command flakes ca-derivations
   ```

2. Build a workspace with nix-cargo-unit:
   ```nix
   cargoUnit.buildWorkspace {
     src = ./.;
     contentAddressed = true;  # Enable CA derivations
     # ... other args
   };
   ```

3. Build once (succeeds, populates store)

4. Build again or build in parallel with many derivations

## Expected Behavior

Build should succeed, reusing cached CA outputs without attempting to write to them.

## Actual Behavior

Build fails with permission denied errors when trying to write to already-existing store paths:

## Error Output

```
error: Cannot build '/nix/store/jfch7ghvwp5j8cf4kjv62ijvmgmyh9y8-aws-lc-rs-build-script-output-1.15.2.drv'.
       Reason: builder failed with exit code 1.
       Last 9 log lines:
       > Running phase: patchPhase
       > Running phase: updateAutotoolsGnuConfigScriptsPhase
       > Running phase: buildPhase
       > Unknown cargo directive: cargo:rustc-check-cfg=cfg(aws_lc_rs_docsrs)
       > Unknown cargo directive: cargo:rustc-check-cfg=cfg(disable_slow_tests)
       > touch: cannot touch '/nix/store/a7arm9dn15c1mlzm4fp3npz32g8ag462-aws-lc-rs-build-script-output-1.15.2/rustc-cfg': Permission denied
       > touch: cannot touch '/nix/store/a7arm9dn15c1mlzm4fp3npz32g8ag462-aws-lc-rs-build-script-output-1.15.2/rustc-link-lib': Permission denied
       > touch: cannot touch '/nix/store/a7arm9dn15c1mlzm4fp3npz32g8ag462-aws-lc-rs-build-script-output-1.15.2/rustc-link-search': Permission denied
       > touch: cannot touch '/nix/store/a7arm9dn15c1mlzm4fp3npz32g8ag462-aws-lc-rs-build-script-output-1.15.2/rustc-env': Permission denied
```

## Analysis

The issue is in the generated `units.nix` build script output derivations. The build phase does:

```bash
touch $out/rustc-cfg
touch $out/rustc-link-lib
# ... etc
```

With CA-derivations, if the output hash matches an existing store path, Nix reuses that path. But the build script still runs and tries to `touch` files in `$out`, which now points to a read-only store path.

The problem is that CA-derivation builds can be "short-circuited" by Nix when the output already exists, but our build scripts assume they're always writing to a fresh output directory.

## Possible Fix

Option 1: Check if output files exist before touching:
```bash
[ -f "$out/rustc-cfg" ] || touch "$out/rustc-cfg"
```

Option 2: Use `__contentAddressed` with fixed-output derivation semantics that skip the build entirely when output exists.

Option 3: Don't use `touch` at all - only write files when there's actual content from the build script. Empty files could be represented differently (e.g., absence of file means empty).

Option 4: Set `__contentAddressed = false` for build script output derivations specifically, since their outputs are typically small and deduplication benefit is minimal.
