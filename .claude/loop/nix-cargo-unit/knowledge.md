# Codebase Knowledge

Workers read this FIRST before exploring.

## File Map

| Path | Purpose |
|------|---------|
| `@src/main.rs` | CLI entry point - reads unit-graph JSON from stdin, outputs Nix or JSON |
| `@src/unit_graph.rs` | Unit graph types (UnitGraph, Unit, Target, Profile, Dependency) and Nix codegen |
| `@src/lib.rs` | Library exports (to be created) |
| `@src/rustc_flags.rs` | Rustc flag reconstruction from unit metadata (to be created) |
| `@src/source_filter.rs` | Source file filtering per crate (to be created) |
| `@src/nix_gen.rs` | Nix derivation code generation (to be created) |
| `@src/build_script.rs` | Build script handling (to be created) |
| `@src/proc_macro.rs` | Proc-macro host compilation (to be created) |
| `@flake.nix` | Nix flake with packages, devShells, overlays |
| `@nix/lib.nix` | Nix library for IFD-based builds (to be created) |
| `@nix/dynamic.nix` | Dynamic derivations mode (to be created) |
| `@Cargo.toml` | Rust package manifest - edition 2024 |

## Patterns

### Error Handling
```rust
// Use color_eyre with wrap_err for context
let data = std::fs::read_to_string(&path)
    .wrap_err_with(|| format!("failed to read {path:?}"))?;
```

### Inline Imports
```rust
// Always use full paths, never top-level imports
fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    // ...
}
```

### Serde for Unit Graph
```rust
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Unit {
    pub pkg_id: String,
    // ...
}
```

### CLI with clap derive
```rust
#[derive(clap::Parser)]
struct Cli {
    #[arg(short, long, default_value = "nix")]
    format: String,
}
```

## Cargo Unit Graph Format

Key fields from `cargo --unit-graph`:
- `pkg_id`: `"name version (source)"` - unique package identifier
- `target.kind`: `["lib"]`, `["bin"]`, `["proc-macro"]`, `["custom-build"]`
- `target.src_path`: Absolute path to entry point
- `mode`: `"build"`, `"check"`, `"test"`, `"doc"`, `"run-custom-build"`
- `dependencies[].index`: Index into units array
- `dependencies[].extern_crate_name`: Name to use in `--extern`
- `platform`: Host platform for proc-macros (null for target platform)

## Nix Experimental Features Required

```nix
# Enable in nix.conf or flake
experimental-features = [
  "nix-command"
  "flakes"
  "dynamic-derivations"
  "ca-derivations"
  "recursive-nix"
]
```

## CA-Derivations Pattern

```nix
mkDerivation {
  # ... normal attrs ...

  # Content-addressed output
  __contentAddressed = true;
  outputHashMode = "recursive";
  outputHashAlgo = "sha256";
}
```

## IFD Pattern

```nix
let
  # Step 1: Generate JSON at build time
  jsonDrv = pkgs.runCommand "unit-graph" {} ''
    cargo --unit-graph > $out
  '';

  # Step 2: Import at eval time (IFD)
  graph = builtins.fromJSON (builtins.readFile jsonDrv);
in
  # Step 3: Generate derivations from graph
  ...
```

## Gotchas

### Unit Identity
Same package can appear multiple times with different features. Use hash of (pkg_id + features + profile + mode) as derivation key.

### Build Scripts
`mode: "run-custom-build"` units must execute before their dependents. They output `cargo:rustc-*` directives to stdout.

### Proc Macros
`target.kind: ["proc-macro"]` compiles for HOST, not target. Check `platform` field.

### Source Paths
`target.src_path` is absolute on the machine that ran cargo. In Nix, remap to `${src}/relative/path`.

### Extern Crate Names
Dependency's `extern_crate_name` may differ from crate name (e.g., `serde_derive` -> `serde`). Always use the provided name.

## From feature #1

### Polymorphic JSON Fields
Cargo's unit-graph JSON has fields that can be multiple types:
- `lto`: bool (`false`/`true`) OR string (`"thin"`/`"fat"`/`"off"`)
- `debuginfo`: int (`0`/`1`/`2`) OR string (`"none"`/`"limited"`/`"full"`/`"line-tables-only"`)
- `strip`: bool OR string (`"none"`/`"debuginfo"`/`"symbols"`)

Use custom `serde::Deserialize` with `deserialize_any` and visitor pattern:
```rust
impl<'de> serde::Deserialize<'de> for LtoSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        struct LtoVisitor;
        impl serde::de::Visitor<'_> for LtoVisitor {
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> { ... }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> { ... }
        }
        deserializer.deserialize_any(LtoVisitor)
    }
}
```

### Unit Helper Methods
`Unit` has helpers for detection:
- `is_build_script()` - checks `mode == "run-custom-build"` or `kind.contains("custom-build")`
- `is_proc_macro()` - checks `kind.contains("proc-macro")`
- `is_lib()` / `is_bin()` / `is_test()` - check target kind
- `package_name()` / `package_version()` - parse from `pkg_id`

### Default Values
- `target.test`, `target.doctest`, `target.doc` default to `true`
- `profile.panic` defaults to `Unwind`
- Most profile options default to `false` or `None`
