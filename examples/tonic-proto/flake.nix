{
  description = "Tonic protobuf example for nix-cargo-unit";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

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

      # rustVersion auto-read from rust-toolchain.toml
      workspace = cargoUnit.buildWorkspace {
        src = ./.;
        contentAddressed = false;
        profile = "release";

        # Include proto files in source
        extraSourcePaths = [ "proto" ];

        # Native build inputs needed for tonic-build
        nativeBuildInputs = [ pkgs.protobuf ];
      };
    in {
      default = workspace.default;
      unit-graph-json = workspace.unitGraphJson;
      units-nix = workspace.unitsNix;
    });

    checks = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system}.extend rust-overlay.overlays.default;
      cargoUnit = nix-cargo-unit.mkLib pkgs;

      # rustVersion auto-read from rust-toolchain.toml
      workspace = cargoUnit.buildWorkspace {
        src = ./.;
        contentAddressed = false;
        extraSourcePaths = [ "proto" ];
        nativeBuildInputs = [ pkgs.protobuf ];
      };
    in {
      app-runs = pkgs.runCommand "test-tonic-proto-runs" {} ''
        ${workspace.default}/bin/tonic-proto-example > $out
        grep "Tonic proto codegen works" $out
      '';

      workspace-builds = workspace.default;
    });
  };
}
