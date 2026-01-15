{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    nix-cargo-unit.url = "path:..";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    nix-cargo-unit,
  }: let
    system = "aarch64-darwin";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [(import rust-overlay)];
    };
    rustToolchain = pkgs.rust-bin.nightly.latest.default;
    cargoUnit = import ../nix/lib.nix {
      inherit pkgs;
      nix-cargo-unit = nix-cargo-unit.packages.${system}.default;
    };
    workspace = cargoUnit.buildWorkspace {
      src = ./.;
      inherit rustToolchain;
      profile = "release";
    };
  in {
    packages.${system} = {
      default = workspace.packages.dep-repro or workspace.default;
      inherit (workspace) units;
    };
  };
}
