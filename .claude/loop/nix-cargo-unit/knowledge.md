# Codebase Knowledge

Workers read this FIRST before exploring.

## File Map

| Path | Purpose |
|------|---------|
| `@src/main.rs` | CLI entry point - reads unit-graph JSON from stdin, outputs Nix or JSON |
| `@src/unit_graph.rs` | Unit graph types (UnitGraph, Unit, Target, Profile, Dependency) and Nix codegen |
| `@src/lib.rs` | Library exports - exposes unit_graph and rustc_flags modules |
| `@src/rustc_flags.rs` | RustcFlags builder - generates rustc CLI args from unit metadata |
| `@src/source_filter.rs` | Source location parsing, path remapping, Nix fileset generation |
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

## From feature #2

### Unit Identity Hash
`Unit::identity_hash()` computes a SHA-256 hash (first 8 bytes, 16 hex chars) from:
- `pkg_id` - package identity
- `target.name` + `crate_types` - distinguishes multiple targets in same pkg
- `features` (sorted) - different feature sets = different compilation
- `profile.name`, `opt_level`, `lto`, `debuginfo`, `panic`, `debug_assertions`, `overflow_checks`, `codegen_units`
- `mode` - build vs test vs check
- `platform` - host platform for proc-macros

### Derivation Naming
`Unit::derivation_name()` returns `{crate_name}-{version}-{identity_hash}`.
Example: `serde-1.0.219-a1b2c3d4e5f67890`

This ensures unique derivation names even when the same crate appears multiple times with different features/profiles.

## From feature #3

### RustcFlags Builder
`RustcFlags::from_unit(&unit)` generates all rustc flags from unit metadata:
- `--crate-name`, `--edition`, `--crate-type`
- `-C opt-level`, `-C debuginfo`, `-C lto`, `-C panic`, `-C strip`
- `-C debug-assertions`, `-C overflow-checks`, `-C codegen-units`
- `--cfg feature="..."` for each feature
- `--test` flag for test mode

Does NOT include `--extern` or `-L` flags - those must be added separately based on dependency graph.

### Adding Dependencies
```rust
let mut flags = RustcFlags::from_unit(&unit);
flags.add_extern("serde", "/nix/store/abc/lib/libserde.rlib");
flags.add_lib_path("/nix/store/abc/lib");
flags.add_source(&unit.target.src_path);
flags.add_output("$out/lib.rlib");
```

### Shell Output
`flags.to_shell_string()` returns a properly-escaped shell command string.
Arguments with spaces/quotes are single-quoted with proper escaping.

## From feature #4

### Source Location Parsing
`SourceLocation::from_unit(&unit)` extracts source information from pkg_id and src_path:
- `name`, `version` - parsed from pkg_id
- `source` - `SourceType::Path`, `SourceType::Registry`, or `SourceType::Git`
- `crate_root` - absolute path to crate directory (contains Cargo.toml)
- `entry_point` - relative path to entry point from crate root (e.g., "src/lib.rs")

### pkg_id Format
Format: `"name version (source-type+url)"`
- Path: `"my-crate 0.1.0 (path+file:///home/user/project)"`
- Registry: `"serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)"`
- Git: `"dep 0.1.0 (git+https://github.com/user/repo?rev=abc123#abc123def)"`

### Path Remapping
`remap_source_path(src_path, workspace_root, nix_src_var)` converts absolute paths to Nix expressions:
```rust
remap_source_path("/workspace/crates/foo/src/lib.rs", "/workspace", "src")
// Returns: "${src}/crates/foo/src/lib.rs"
```

### Source Type Detection
```rust
loc.is_path()     // Local path source (workspace crates)
loc.is_registry() // crates.io or other registry
loc.is_git()      // Git dependency
```

### Nix Fileset Generation
`loc.to_nix_fileset("src", include_cargo_toml)` generates minimal source expressions:
```nix
lib.fileset.toSource {
  root = ${src};
  fileset = lib.fileset.unions [
    (${src}/src)
    (${src}/Cargo.toml)
  ];
}
```
