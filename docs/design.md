# Incremental Cargo Builds with Nix

A proposal for true per-unit incremental Rust compilation in Nix using experimental Cargo and Nix features.

## Problem

Nix's input-addressed derivation model means any source file change rebuilds all crates in a workspace. Crane mitigates this by caching dependencies separately, but changing `crates/foo/lib.rs` still rebuilds `foo`, `bar`, `baz`, and everything downstream—even if `bar` and `baz` don't actually depend on the changed code path.

## Goal

**Per-compilation-unit caching**: Only rebuild what actually changed, with Nix-native reproducibility.

```
Change crates/foo/src/lib.rs
  → Only rebuild: foo.rlib, bins depending on foo
  → Skip: unrelated crates, unchanged units
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         cargo --unit-graph                          │
│                    (nightly, -Z unstable-options)                   │
│                                                                     │
│  Outputs JSON: every rustc invocation as a "unit" with deps        │
└─────────────────────────────────┬───────────────────────────────────┘
                                  │ JSON
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                      Nix Unit Graph Parser                          │
│                                                                     │
│  - Parse unit graph JSON                                            │
│  - Generate one derivation per unit                                 │
│  - Wire up dependencies between derivations                         │
│  - Use IFD or dynamic derivations                                   │
└─────────────────────────────────┬───────────────────────────────────┘
                                  │ Nix derivations
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     CA-Derivations + Nix Build                      │
│                                                                     │
│  - Content-addressed outputs: identical .rlib = same store path    │
│  - Only changed units trigger rebuilds                              │
│  - Downstream units skip if inputs unchanged                        │
└─────────────────────────────────────────────────────────────────────┘
```

## Experimental Features Required

### Cargo (Nightly)

| Feature | Flag | Status | Docs |
|---------|------|--------|------|
| Unit Graph | `--unit-graph -Z unstable-options` | Unstable | [cargo docs][unit-graph] |
| Build Std | `-Z build-std` | Unstable | [cargo docs][build-std] |

### Nix

| Feature | Config | Status | Docs |
|---------|--------|--------|------|
| Dynamic Derivations | `dynamic-derivations` | Experimental | [RFC 0092][rfc-dynamic], [blog][dynamic-blog] |
| CA-Derivations | `ca-derivations` | Experimental | [RFC 0062][rfc-ca] |
| Recursive Nix | `recursive-nix` | Experimental | [nix manual][recursive-nix] |

Enable in `/etc/nix/nix.conf` or flake:
```nix
nix.settings.experimental-features = [
  "nix-command"
  "flakes"
  "dynamic-derivations"
  "ca-derivations"
  "recursive-nix"
];
```

## Unit Graph Format

```bash
cargo +nightly build --unit-graph -Z unstable-options 2>/dev/null | jq .
```

Output structure ([full docs][unit-graph]):

```json
{
  "version": 1,
  "units": [
    {
      "pkg_id": "serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)",
      "target": {
        "kind": ["lib"],
        "crate_types": ["lib"],
        "name": "serde",
        "src_path": "/path/to/serde/src/lib.rs",
        "edition": "2021"
      },
      "profile": {
        "name": "release",
        "opt_level": "3",
        "debuginfo": 0
      },
      "features": ["default", "derive", "std"],
      "mode": "build",
      "dependencies": [
        { "index": 5, "extern_crate_name": "serde_derive", "public": false }
      ]
    }
  ],
  "roots": [0, 1, 2]
}
```

Key fields:
- `pkg_id`: Unique package identifier
- `target.src_path`: Entry point for this unit
- `features`: Resolved features for this compilation
- `mode`: `build`, `check`, `test`, `doc`, `run-custom-build` (build.rs)
- `dependencies[].index`: Index into `units` array for deps

## Implementation Plan

### Phase 1: Unit Graph → Nix (IFD)

Use Import From Derivation as a starting point (works today, no experimental Nix).

```nix
{ pkgs, src, ... }:

let
  # Step 1: Generate unit graph
  unitGraphJson = pkgs.runCommand "unit-graph" {
    nativeBuildInputs = [ pkgs.cargo ];
  } ''
    cd ${src}
    cargo +nightly build --unit-graph -Z unstable-options --release > $out
  '';

  # Step 2: Parse (IFD - evaluated at build time)
  unitGraph = builtins.fromJSON (builtins.readFile unitGraphJson);

  # Step 3: Build derivation for each unit
  mkUnitDrv = unit: let
    deps = map (d: unitDrvs.${toString d.index}) unit.dependencies;
  in pkgs.stdenv.mkDerivation {
    pname = unit.target.name;
    version = "0.0.0";

    # Only include this unit's source
    src = pkgs.lib.fileset.toSource {
      root = src;
      fileset = pkgs.lib.fileset.unions [
        (builtins.dirOf unit.target.src_path)
      ];
    };

    buildInputs = deps;

    buildPhase = ''
      # Reconstruct rustc invocation from unit metadata
      rustc ${mkRustcArgs unit} \
        --crate-name ${unit.target.name} \
        --edition ${unit.target.edition} \
        ${lib.concatMapStrings (d: "--extern ${d.extern_crate_name}=${d.output}/lib*.rlib ") unit.dependencies} \
        -o $out/lib${unit.target.name}.rlib \
        ${unit.target.src_path}
    '';
  };

  # Build all units
  unitDrvs = builtins.listToAttrs (
    builtins.genList (i: {
      name = toString i;
      value = mkUnitDrv (builtins.elemAt unitGraph.units i);
    }) (builtins.length unitGraph.units)
  );

in {
  # Export root units (the actual binaries/libs we want)
  outputs = map (i: unitDrvs.${toString i}) unitGraph.roots;
}
```

### Phase 2: Handle Build Scripts

Build scripts (`mode: "run-custom-build"`) must run before their dependents. They can:
- Generate code (`OUT_DIR`)
- Set `cargo:rustc-link-lib`
- Set `cargo:rustc-cfg`

```nix
mkBuildScript = unit: pkgs.runCommand "build-script-${unit.target.name}" {
  # Build script dependencies
  buildInputs = map (d: unitDrvs.${toString d.index}) unit.dependencies;
} ''
  # Compile build script
  rustc --crate-type bin -o build_script ${unit.target.src_path}

  # Run it, capture output
  mkdir -p $out
  OUT_DIR=$out/out ./build_script > $out/build-output.txt

  # Parse cargo:rustc-* directives
  grep "^cargo:rustc-" $out/build-output.txt > $out/rustc-flags.txt || true
'';
```

### Phase 3: Dynamic Derivations

Replace IFD with dynamic derivations for better performance (no evaluation pause).

```nix
# Requires: experimental-features = dynamic-derivations ca-derivations

{ pkgs, ... }:

pkgs.stdenv.mkDerivation {
  name = "cargo-workspace";

  __structuredAttrs = true;

  buildPhase = ''
    # Generate unit graph
    cargo +nightly build --unit-graph -Z unstable-options > unit-graph.json

    # Generate Nix expressions for each unit (using a helper binary)
    unit-graph-to-nix unit-graph.json > units.nix

    # Instantiate dynamic derivations
    nix-instantiate --expr "import ./units.nix" --add-root $out
  '';

  # Content-addressed output
  __contentAddressed = true;
  outputHashMode = "recursive";
}
```

### Phase 4: CA-Derivations for Deduplication

With content-addressed derivations, identical outputs share store paths:

```nix
mkUnitDrv = unit: pkgs.stdenv.mkDerivation {
  # ... same as before ...

  # Enable content-addressing
  __contentAddressed = true;
  outputHashMode = "recursive";
  outputHashAlgo = "sha256";
};
```

If `serde` compiles to the same `.rlib` (same features, same source), it gets the same store path—no rebuild for downstream units.

## Challenges & Solutions

### Challenge 1: Proc Macros

Proc macros compile for the host, not target. Unit graph shows these with separate entries.

```json
{
  "target": { "kind": ["proc-macro"], ... },
  "platform": "x86_64-unknown-linux-gnu"  // host, not target
}
```

**Solution**: Detect `kind: ["proc-macro"]`, compile with host toolchain.

### Challenge 2: Feature Unification

Cargo unifies features. `serde` might compile multiple times with different features.

```json
{ "pkg_id": "serde 1.0.0", "features": ["std"] },
{ "pkg_id": "serde 1.0.0", "features": ["std", "derive"] }
```

**Solution**: Unit graph already handles this—each entry is a unique compilation. Use full unit identity (pkg_id + features + profile) as derivation key.

### Challenge 3: Rustc Flags

Need to reconstruct exact `rustc` invocation. Use verbose cargo output as reference:

```bash
cargo build -v 2>&1 | grep "Running.*rustc"
```

Key flags to preserve:
- `--edition`
- `--crate-type`
- `-C opt-level=N`
- `-C debuginfo=N`
- `--cfg feature="..."`
- `--extern name=path`
- `-L dependency=path`

### Challenge 4: Sysroot / std

For `#![no_std]` or custom std, need `-Z build-std`:

```nix
stdUnits = pkgs.runCommand "build-std" {} ''
  cargo +nightly build --unit-graph -Z build-std=core,alloc -Z unstable-options
'';
```

## Prototype: Minimal Working Example

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };

      rustNightly = pkgs.rust-bin.nightly.latest.default;

      # Helper to parse unit graph and build
      buildWithUnitGraph = src: let
        unitGraphJson = pkgs.runCommand "unit-graph" {
          nativeBuildInputs = [ rustNightly ];
        } ''
          cd ${src}
          cargo build --unit-graph -Z unstable-options --release 2>/dev/null > $out
        '';

        unitGraph = builtins.fromJSON (builtins.readFile unitGraphJson);

        # ... rest of implementation
      in { };

    in {
      packages.${system}.default = buildWithUnitGraph ./.;
    };
}
```

## Prior Art & References

### Projects

| Project | Approach | Status |
|---------|----------|--------|
| [Crane][crane] | Deps-only caching | Active, production-ready |
| [cargo2nix][cargo2nix] | Per-crate derivations | Maintenance mode |
| [nix-ninja][nix-ninja] | Dynamic derivations for Ninja | Experimental |
| [naersk][naersk] | Cargo-driven builds | Active |

### Documentation

- [Cargo Unit Graph][unit-graph] - Official docs on `--unit-graph`
- [Cargo Build Scripts][build-scripts] - How `build.rs` works
- [Nix Dynamic Derivations RFC][rfc-dynamic] - RFC 0092
- [Nix CA-Derivations RFC][rfc-ca] - RFC 0062
- [Dynamic Derivations Blog][dynamic-blog] - Practical walkthrough

### Discussions

- [Crane #213][crane-213] - Incremental builds discussion
- [nix-ninja Discourse][nix-ninja-discourse] - Announcement thread
- [Rust Compiler Survey 2025][rust-survey] - Pain points with incremental builds

## Estimated Effort

| Phase | Scope | Effort |
|-------|-------|--------|
| Phase 1 (IFD) | Basic per-unit builds, no build.rs | 2-3 weeks |
| Phase 2 (build.rs) | Build script support | 2-3 weeks |
| Phase 3 (dynamic) | Dynamic derivations | 2-3 weeks |
| Phase 4 (polish) | Edge cases, workspace features | 2-4 weeks |

Total: ~2-3 months for production-ready implementation.

## Success Criteria

1. **Correctness**: Builds match `cargo build` output bit-for-bit
2. **Incrementality**: Changing one file rebuilds only affected units
3. **Cache hits**: Unchanged units are not rebuilt across `nix build` invocations
4. **Workspace support**: Works with multi-crate workspaces
5. **Build scripts**: Handles `build.rs` correctly

## Next Steps

1. [ ] Build CLI tool to parse unit graph and emit rustc commands
2. [ ] Prototype Phase 1 with IFD on a simple workspace
3. [ ] Validate rustc flag reconstruction against `cargo build -v`
4. [ ] Add build script support
5. [ ] Benchmark against Crane on ix workspace
6. [ ] Migrate to dynamic derivations when stable enough

---

[unit-graph]: https://doc.rust-lang.org/cargo/reference/unstable.html#unit-graph
[build-std]: https://doc.rust-lang.org/cargo/reference/unstable.html#build-std
[build-scripts]: https://doc.rust-lang.org/cargo/reference/build-scripts.html
[rfc-dynamic]: https://github.com/NixOS/rfcs/pull/92
[rfc-ca]: https://github.com/NixOS/rfcs/pull/62
[recursive-nix]: https://nix.dev/manual/nix/2.18/advanced-topics/distributed-builds
[dynamic-blog]: https://fzakaria.com/2025/03/11/nix-dynamic-derivations-a-practical-application
[crane]: https://github.com/ipetkov/crane
[cargo2nix]: https://github.com/cargo2nix/cargo2nix
[nix-ninja]: https://github.com/pdtpartners/nix-ninja
[naersk]: https://github.com/nix-community/naersk
[crane-213]: https://github.com/ipetkov/crane/discussions/213
[nix-ninja-discourse]: https://discourse.nixos.org/t/nix-ninja-ninja-compatible-incremental-build-system-for-nix/62594
[rust-survey]: https://blog.rust-lang.org/2025/09/10/rust-compiler-performance-survey-2025-results/
