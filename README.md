# nix-cargo-unit

Per-unit incremental Rust compilation in Nix using Cargo's `--unit-graph` and content-addressed derivations.

## Features

- **Per-unit caching**: Each compilation unit is a separate Nix derivation
- **Content-addressed**: Uses CA-derivations for deduplication and early cutoff
- **Workspace support**: Handles Cargo workspaces with multiple crates
- **Build scripts**: Executes `build.rs` and captures output directives
- **Proc-macros**: Properly compiles and links procedural macros

## Usage

```nix
{
  inputs.nix-cargo-unit.url = "github:andrewgazelka/nix-cargo-unit";

  outputs = { nix-cargo-unit, nixpkgs, ... }: {
    packages.default = let
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      cargoUnit = nix-cargo-unit.mkLib pkgs;
    in (cargoUnit.buildWorkspace {
      src = ./.;
      rustToolchain = pkgs.rust-bin.nightly.latest.default;
      contentAddressed = true;
    }).default;
  };
}
```

## Requirements

- Nix with `nix-command` and `flakes` enabled
- Nightly Rust toolchain (for `--unit-graph`)
- Optional: `ca-derivations` experimental feature for content-addressed outputs

## How it works

1. Runs `cargo build --unit-graph` to get the compilation DAG
2. Generates a Nix derivation for each unit with proper `--extern` and `-L` flags
3. Wires build script outputs (`cargo:rustc-cfg`, etc.) to dependent units
4. Compiles proc-macros for the host platform

## Example

See [`examples/workspace`](examples/workspace) for a complete example with build scripts and proc-macros.

## License

MIT
