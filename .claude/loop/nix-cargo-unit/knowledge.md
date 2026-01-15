# Codebase Knowledge

Workers read this FIRST before exploring.

## File Map

| Path | Purpose |
|------|---------|
| `@src/main.rs` | CLI entry point - reads unit-graph JSON from stdin, outputs Nix or JSON |
| `@src/unit_graph.rs` | Unit graph types (UnitGraph, Unit, Target, Profile, Dependency) |
| `@src/lib.rs` | Library exports - exposes unit_graph and rustc_flags modules |
| `@src/rustc_flags.rs` | RustcFlags builder - generates rustc CLI args from unit metadata |
| `@src/source_filter.rs` | Source location parsing, path remapping, Nix fileset generation |
| `@src/nix_gen.rs` | Structured Nix derivation builder with proper escaping |
| `@src/build_script.rs` | Build script detection, compile/run derivation generation |
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

## From feature #5

### Nix String Escaping
`NixString::new(s)` properly escapes for Nix double-quoted strings:
- `\\` -> `\`
- `\"` -> `"`
- `\n`, `\r`, `\t` for whitespace
- `$` always escaped to prevent interpolation

`escape_nix_multiline(s)` for `''...''` strings:
- `''` -> `'''` (escape delimiter)
- `${` -> `''${` (escape interpolation)

### NixAttrSet Builder
Type-safe attribute set construction:
```rust
let mut attrs = NixAttrSet::new();
attrs.string("pname", "my-crate");      // Quoted: pname = "my-crate";
attrs.expr("deps", "[ dep1 dep2 ]");    // Raw: deps = [ dep1 dep2 ];
attrs.bool("dontUnpack", true);         // dontUnpack = true;
attrs.string_list("features", &feats); // features = [ "std" "alloc" ];
attrs.multiline("buildPhase", script);  // buildPhase = ''...script...'';
attrs.render(2)                         // Render with 2-level indent
```

### UnitDerivation Builder
`UnitDerivation::from_unit(&unit, workspace_root)` creates a derivation:
- Uses `unit.derivation_name()` for unique names
- Remaps source paths via `remap_source_path()`
- Generates build phase with rustc flags
- Handles bin vs lib vs proc-macro output paths

### NixGenerator
Main entry point for Nix generation:
```rust
let config = NixGenConfig {
    workspace_root: "/workspace".to_string(),
    content_addressed: false,
};
let generator = NixGenerator::new(config);
let nix = generator.generate(&graph);
```

Output structure:
```nix
{ pkgs, rustToolchain, src }:
let
  mkUnit = attrs: pkgs.stdenv.mkDerivation (attrs // { ... });
  units = {
    "crate-0.1.0-abc123" = mkUnit { ... };
    "_idx_0" = units."crate-0.1.0-abc123"; # index alias
  };
in {
  inherit units;
  roots = [ ... ];
  default = ...;
}
```

### CLI Workspace Root Flag
CLI now accepts `--workspace-root` / `-w` for source path remapping:
```bash
cargo --unit-graph | nix-cargo-unit -w /path/to/workspace
```

## From feature #6

### Content-Addressed Derivations
`--content-addressed` CLI flag enables CA-derivation attributes in generated Nix:
```bash
cargo --unit-graph | nix-cargo-unit --content-addressed -w /path/to/workspace
```

Generated derivations include:
```nix
__contentAddressed = true;
outputHashMode = "recursive";
outputHashAlgo = "sha256";
```

### NixGenConfig.content_addressed
Pass `content_addressed: true` to `NixGenConfig` to enable CA attributes:
```rust
let config = NixGenConfig {
    workspace_root: "/workspace".to_string(),
    content_addressed: true,
};
```

### UnitDerivation.from_unit signature
`UnitDerivation::from_unit(unit, workspace_root, content_addressed)` now takes a third parameter to control CA attributes.

## From feature #7

### DepRef Struct
`DepRef` tracks dependency information for Nix derivation wiring:
```rust
pub struct DepRef {
    pub nix_var: String,           // e.g., "units.\"serde-1.0.0-abc123\""
    pub extern_crate_name: String, // e.g., "serde"
    pub derivation_name: String,   // e.g., "serde-1.0.0-abc123"
    pub is_proc_macro: bool,       // whether this is a proc-macro dependency
}
```

### Dependency Wiring in NixGenerator
`NixGenerator::generate()` now wires up dependencies:
- Pre-computes derivation names for all units
- For each unit, creates `DepRef` for each dependency
- `DepRef.nix_var` references the dependency derivation

### Generated Build Phase with Dependencies
Build phase now includes:
1. `-L dependency=$dep/lib` for each dependency (library search path)
2. `--extern name=$dep/lib/libname.rlib` for regular deps
3. `--extern name="$(find $dep/lib -name 'libname.*' | head -1)"` for proc-macros

### Crate Name Normalization
Library file names normalize hyphens to underscores:
- Crate `serde-derive` -> `libserde_derive.rlib`
- Uses `name.replace('-', "_")` for the file name

### Proc-Macro Handling
Proc-macros are shared libraries (.so on Linux, .dylib on macOS).
Use `find` to locate the correct file since extension varies by platform.

### Output Directory Creation
Build phase creates the correct output directory:
- `mkdir -p $out/bin` for binaries
- `mkdir -p $out/lib` for libraries/proc-macros

## From feature #8

### Build Script Detection
`BuildScriptInfo::from_unit(&unit, workspace_root, content_addressed)` detects build scripts via:
- `unit.mode == "run-custom-build"` - execution phase
- `unit.target.kind.contains("custom-build")` - compilation target

Helper functions:
```rust
is_build_script_unit(unit)    // Either mode or kind
is_build_script_run(unit)     // mode == "run-custom-build"
is_build_script_compile(unit) // kind contains "custom-build"
```

### Two-Derivation Model
Build scripts produce two derivations:
1. **Compile derivation** (`{pkg}-build-script-{version}-{hash}`):
   - Compiles build.rs to `$out/bin/build-script`
   - Uses standard rustc flags from unit

2. **Run derivation** (`{pkg}-build-script-run-{version}-{hash}`):
   - Depends on compile derivation
   - Sets cargo environment (OUT_DIR, CARGO_PKG_*, CARGO_FEATURE_*)
   - Executes binary, parses stdout for `cargo:` directives
   - Outputs structured files: `$out/rustc-cfg`, `$out/rustc-link-lib`, etc.

### Build Script Environment
Run derivation sets these env vars:
- `OUT_DIR=$out/out-dir` - for generated files
- `CARGO_MANIFEST_DIR`, `CARGO_PKG_NAME`, `CARGO_PKG_VERSION`
- `CARGO_FEATURE_{FEATURE}=1` for each enabled feature (uppercase, hyphens to underscores)
- `TARGET`, `HOST`, `PROFILE`

### Parsed Cargo Directives
Run derivation parses these directives to files:
- `cargo:rustc-cfg=...` -> `$out/rustc-cfg` (one per line)
- `cargo:rustc-link-lib=...` -> `$out/rustc-link-lib`
- `cargo:rustc-link-search=...` -> `$out/rustc-link-search`
- `cargo:rustc-env=...` -> `$out/rustc-env` (KEY=VALUE per line)
- `cargo:rustc-cdylib-link-arg=...` -> `$out/rustc-cdylib-link-arg`
- `cargo:warning=...` -> stderr
- `cargo:rerun-if-*` -> ignored (CA handles cache invalidation)

### RustcFlags now Clone
`RustcFlags` derives `Clone` to support `BuildScriptInfo::Clone`.

### escape_nix_multiline is public
`nix_gen::escape_nix_multiline(s)` is now public for use by build_script module.

## From feature #9

### BuildScriptOutput Struct
`BuildScriptOutput` parses the structured output files from build script run derivations:
```rust
pub struct BuildScriptOutput {
    pub rustc_cfgs: Vec<String>,           // From rustc-cfg file
    pub rustc_link_libs: Vec<String>,      // From rustc-link-lib file
    pub rustc_link_searches: Vec<String>,  // From rustc-link-search file
    pub rustc_envs: Vec<(String, String)>, // From rustc-env file (KEY=VALUE)
    pub rustc_cdylib_link_args: Vec<String>, // From rustc-cdylib-link-arg file
}
```

### Parsing Methods
Individual file parsers for each directive type:
- `BuildScriptOutput::parse_cfgs(contents)` - parses cfg expressions
- `BuildScriptOutput::parse_link_libs(contents)` - parses `[KIND=]NAME`
- `BuildScriptOutput::parse_link_searches(contents)` - parses `[KIND=]PATH`
- `BuildScriptOutput::parse_envs(contents)` - parses `KEY=VALUE` pairs
- `BuildScriptOutput::parse_cdylib_link_args(contents)` - parses linker args

All parsers skip empty lines and trim whitespace.

### Converting to rustc Flags
`output.to_rustc_args()` generates rustc CLI arguments:
- `--cfg` for each cfg
- `-l` for each link lib
- `-L` for each link search
- `-C link-arg=` for each cdylib link arg

Note: `rustc_envs` are NOT included in rustc args (they're environment vars, not flags).

### Nix Shell Script Generation
`BuildScriptOutput::generate_nix_flag_reader(var)` generates shell script to read build script outputs at derivation build time:
- Reads each output file and appends to `BUILD_SCRIPT_FLAGS`
- Sets `OUT_DIR` to the out-dir subdirectory
- Handles missing files gracefully with `if [ -f ]` checks

### Nix Expression Generation
`BuildScriptOutput::generate_nix_expr_reader(var)` generates a Nix expression that reads outputs using `builtins.readFile` - useful for structuredAttrs or eval-time access.

## From feature #10

### BuildScriptRef Struct
`BuildScriptRef` tracks build script derivation references for dependent units:
```rust
pub struct BuildScriptRef {
    pub run_drv_var: String,      // e.g., "units.\"my-crate-build-script-run-...\""
    pub compile_drv_name: String, // e.g., "my-crate-build-script-..."
    pub run_drv_name: String,     // e.g., "my-crate-build-script-run-..."
}
```

### Build Script Wiring in NixGenerator
`NixGenerator::generate()` now:
1. First pass: identifies all `mode == "run-custom-build"` units
2. Generates compile and run derivations for each build script
3. Creates a map from unit index to `BuildScriptRef`
4. Second pass: for regular units, checks if any dependency is a build script
5. Skips build script dependencies from `--extern` flags (they're not linkable crates)
6. Sets `build_script_ref` on units that depend on build scripts

### Build Phase Integration
Units with a `build_script_ref`:
1. Include the run derivation in `buildInputs`
2. Initialize `BUILD_SCRIPT_FLAGS=""`
3. Call `BuildScriptOutput::generate_nix_flag_reader()` to read output files
4. Append `$BUILD_SCRIPT_FLAGS` to the rustc command

### Build Script Dependency Detection
Build scripts are identified by `unit.mode == "run-custom-build"`. When a regular unit has a dependency with this mode, it gets the build script's outputs wired into its rustc invocation.

### Important: Build Scripts Are Not Extern Dependencies
Build script execution units should NOT be added as `--extern` dependencies. They produce configuration, not linkable artifacts. The generated code correctly filters them out from the regular dependency wiring.

## From feature #11

### Proc-Macro Module
`src/proc_macro.rs` handles proc-macro specific logic:
- `ProcMacroInfo::from_unit()` - extracts proc-macro details from unit
- `requires_host_toolchain(unit)` - returns true for proc-macros AND build scripts
- `is_proc_macro_unit(unit)` - checks if unit is a proc-macro
- `platform_library_extension(platform)` - returns `so`/`dylib`/`dll` based on platform

### Host Toolchain for Cross-Compilation
Proc-macros and build scripts must compile for the HOST platform:
```rust
if self.cross_compiling && crate::proc_macro::requires_host_toolchain(unit) {
    "hostRustToolchain"
} else {
    "rustToolchain"
}
```

### Generated Nix Function Signature
With cross-compilation enabled:
```nix
{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src }:
```
Default `hostRustToolchain` to `rustToolchain` for native builds.

### CLI Cross-Compilation Flags
```bash
nix-cargo-unit --cross-compile --host-platform aarch64-apple-darwin --target-platform x86_64-unknown-linux-gnu
```

### NixGenConfig Changes
```rust
pub struct NixGenConfig {
    pub workspace_root: String,
    pub content_addressed: bool,
    pub cross_compiling: bool,      // New
    pub target_platform: Option<String>,  // New
    pub host_platform: Option<String>,    // New
}
```

### UnitDerivation Changes
Added `toolchain_var` field to store which toolchain to use (`rustToolchain` or `hostRustToolchain`).
Added `is_proc_macro` field for tracking.

### Platform Library Extensions
- Linux/Unix: `.so`
- macOS: `.dylib`
- Windows: `.dll`

Determined by `platform_library_extension(platform)` based on platform triple.

## From feature #12

### Nix Library Structure
`nix/lib.nix` provides IFD-based per-unit builds:
- `generateUnitGraph { src, rustToolchain, profile, cargoArgs }` - runs `cargo --unit-graph`
- `generateNixFromUnitGraph { unitGraphJson, workspaceRoot, ... }` - converts JSON to Nix
- `buildWorkspace { src, rustToolchain, ... }` - main entry point

### IFD Pattern (Import From Derivation)
```nix
# Step 1: Generate JSON at build time
unitGraphJson = pkgs.runCommand "unit-graph.json" {...} ''
  cargo build --unit-graph -Z unstable-options > $out
'';

# Step 2: Generate Nix from JSON
unitsNix = pkgs.runCommand "units.nix" {...} ''
  nix-cargo-unit < ${unitGraphJson} > $out
'';

# Step 3: Import at eval time (IFD - the magic step)
units = import unitsNix { inherit pkgs rustToolchain src; };
```

### Flake Outputs
- `lib.<system>` - pre-configured library for each system
- `mkLib pkgs` - function to create library for custom pkgs
- `overlays.default` - overlay that adds `nix-cargo-unit` to pkgs

### Usage Pattern
```nix
let
  cargoUnit = nix-cargo-unit.mkLib pkgs;
  result = cargoUnit.buildWorkspace {
    src = ./.;
    rustToolchain = pkgs.rust-bin.nightly.latest.default;
    contentAddressed = true;  # Enable CA-derivations
  };
in {
  inherit (result) default roots units;
  # result.unitGraphJson and result.unitsNix for debugging
}
```

### Nightly Rust Required
`cargo --unit-graph` requires nightly and `-Z unstable-options`. Always use a nightly toolchain for generating the unit graph.

### Environment Variables in Unit Graph Generation
The runCommand sets:
- `CARGO_HOME` for cargo cache
- `SSL_CERT_FILE` for crates.io downloads (uses `pkgs.cacert`)

## From feature #13

### Fileset-Based Source Filtering
`nix/lib.nix` now provides source filtering using `lib.fileset.toSource`:
- Reduces input hash by excluding non-Rust files (docs, CI configs, etc.)
- Changes to irrelevant files don't invalidate builds

### filterRustSource Function
Filters a source tree to only Rust-relevant files:
```nix
filterRustSource {
  src = ./.;
  extraPaths = [ "proto" "assets" ];  # Optional extra paths to include
}
```

Includes by default:
- `Cargo.toml`, `Cargo.lock`
- `src/`, `crates/`, `tests/`, `benches/`, `examples/`
- `build.rs` at root
- All `.rs` and `.toml` files (via fileFilter)

### filterCrateSource Function
More aggressive filtering for a single crate within a workspace:
```nix
filterCrateSource {
  src = ./.;
  cratePath = "crates/core";  # Relative path to crate
}
```

Only includes:
- Root `Cargo.toml` and `Cargo.lock`
- The entire crate directory

### buildWorkspace Source Filtering Parameters
```nix
buildWorkspace {
  src = ./.;
  filterSource = true;      # Enable filtering (default: true)
  extraSourcePaths = [ ];   # Additional paths to include when filtering
  # ...
}
```

### filteredSrc in Output
The filtered source is exposed in the output for debugging:
```nix
let result = cargoUnit.buildWorkspace { src = ./.; };
in result.filteredSrc  # The filtered source derivation
```

### lib.fileset.maybeMissing
Used for optional paths that may not exist:
```nix
optionalPath = path: lib.fileset.maybeMissing path;
```
Returns null for non-existent paths, which are filtered out before creating unions.
