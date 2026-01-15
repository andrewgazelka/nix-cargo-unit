{
  description = "Example workspace for nix-cargo-unit end-to-end testing";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    # Reference the parent flake for nix-cargo-unit
    nix-cargo-unit.url = "path:../..";
    nix-cargo-unit.inputs.nixpkgs.follows = "nixpkgs";
    nix-cargo-unit.inputs.rust-overlay.follows = "rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    nix-cargo-unit,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
  in {
    packages = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
      cargoUnit = nix-cargo-unit.mkLib pkgs;

      # Build the entire workspace using nix-cargo-unit
      # rustVersion auto-read from rust-toolchain.toml
      workspace = cargoUnit.buildWorkspace {
        src = ./.;
        # Enable CA-derivations for content-addressed outputs
        contentAddressed = true;
        # Default release profile
        profile = "release";
      };
    in {
      # The main app binary
      default = workspace.default;

      # Access individual workspace members
      # Note: library/proc-macro names use underscores (Rust convention), binary names can use hyphens
      example-app = workspace.packages."example-app" or workspace.binaries."example-app" or workspace.default;
      example-core = workspace.packages.example_core or workspace.libraries.example_core or null;
      example-macros = workspace.packages.example_macros or workspace.libraries.example_macros or null;

      # Debug: expose intermediate outputs
      unit-graph-json = workspace.unitGraphJson;
      units-nix = workspace.unitsNix;
    });

    # Dev shell for working on the example
    devShells = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
    in {
      default = pkgs.mkShell {
        packages = [
          (pkgs.rust-bin.nightly."2026-01-14".default.override {
            extensions = ["rust-src" "rust-analyzer"];
          })
          pkgs.cargo-watch
        ];
      };
    });

    # Checks to validate the workspace builds correctly
    checks = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
      cargoUnit = nix-cargo-unit.mkLib pkgs;

      # rustVersion auto-read from rust-toolchain.toml, contentAddressed defaults to true
      workspace = cargoUnit.buildWorkspace {
        src = ./.;
      };
    in {
      # Test that the app binary runs correctly
      app-runs = pkgs.runCommand "test-app-runs" {} ''
        ${workspace.default}/bin/example-app > $out
        grep "All features working" $out
      '';

      # Test that the workspace builds
      workspace-builds = workspace.default;
    });
  };
}
