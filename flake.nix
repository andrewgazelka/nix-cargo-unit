{
  description = "Per-unit incremental Rust compilation in Nix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f system);
  in {
    packages = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
    in {
      default = pkgs.rustPlatform.buildRustPackage {
        pname = "nix-cargo-unit";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
      };
    });

    devShells = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
      rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
        extensions = ["rust-src" "rust-analyzer"];
      };
    in {
      default = pkgs.mkShell {
        packages = [
          rustToolchain
          pkgs.cargo-watch
        ];
      };
    });

    overlays.default = final: prev: {
      nix-cargo-unit = self.packages.${final.system}.default;
    };

    # Nix library for IFD-based per-unit builds
    # Usage: nix-cargo-unit.lib { inherit pkgs; }
    lib = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend self.overlays.default;
    in
      import ./nix/lib.nix {
        inherit pkgs;
        nix-cargo-unit = self.packages.${system}.default;
      });

    # Convenience function to create the library for any pkgs
    # Usage: nix-cargo-unit.mkLib pkgs
    mkLib = pkgs:
      import ./nix/lib.nix {
        inherit pkgs;
        nix-cargo-unit = self.packages.${pkgs.system}.default;
      };

    # Dynamic derivations library (no IFD, uses recursive-nix)
    # Requires: experimental-features = dynamic-derivations ca-derivations recursive-nix
    # Usage: nix-cargo-unit.libDynamic.<system> { src = ./.; rustToolchain = ...; }
    libDynamic = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend self.overlays.default;
    in
      import ./nix/dynamic.nix {
        inherit pkgs;
        nix-cargo-unit = self.packages.${system}.default;
      });

    # Convenience function to create the dynamic library for any pkgs
    # Usage: nix-cargo-unit.mkLibDynamic pkgs
    mkLibDynamic = pkgs:
      import ./nix/dynamic.nix {
        inherit pkgs;
        nix-cargo-unit = self.packages.${pkgs.system}.default;
      };
  };
}
