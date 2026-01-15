# Example Workspace for nix-cargo-unit

This workspace demonstrates end-to-end testing of nix-cargo-unit, including:

- Build scripts (build.rs) with `cargo:rustc-cfg` and `OUT_DIR` code generation
- Proc-macros compiled for the host platform
- Workspace dependency wiring
- CA-derivations for content-addressed outputs

## Structure

```
crates/
  core/           - Library with build.rs that generates code
  macros/         - Proc-macro crate (derive + function-like + attribute)
  app/            - Binary that uses both, also has its own build.rs
```

## Building with Cargo

```bash
cargo build
cargo run
cargo nextest run
```

## Building with nix-cargo-unit

```bash
# Build everything
nix build

# Build and run the app
nix run

# Check that the build works
nix flake check
```

## Generating unit graph manually

```bash
# Generate unit graph JSON
cargo +nightly build --unit-graph -Z unstable-options --release 2>/dev/null > unit-graph.json

# Convert to Nix
nix-cargo-unit -w . --content-addressed < unit-graph.json > units.nix
```

## Expected output

Running the app should print:

```
=== nix-cargo-unit Example App ===

App Version: 0.1.0
Build Number: 1

Core Library:
  Build script status: Build script was run!
  Generated value: 42
  Generated greeting: Hello from generated code!

Proc-Macros:
  Describe derive: AppConfig
  make_greeter!: Hello from app_greeter!

Core Config: Config { name: "example", value: 100 }

Doing important work...

=== All features working! ===
```
