# Dynamic Derivations mode for nix-cargo-unit
#
# This module uses Nix's experimental "dynamic-derivations" feature to avoid
# Import From Derivation (IFD). Instead of importing a generated Nix file
# at evaluation time, we create a derivation that:
# 1. Runs cargo --unit-graph
# 2. Runs nix-cargo-unit to generate Nix
# 3. Uses recursive nix to instantiate and build the generated derivations
#
# Benefits:
# - No IFD: evaluation doesn't block on builds
# - Better caching: unit graph generation is cached separately
# - Works in restricted evaluation mode (where IFD is disabled)
#
# Requirements:
# - experimental-features = dynamic-derivations ca-derivations recursive-nix
# - Nix daemon must have recursive-nix enabled
#
# Usage:
#   let
#     cargoUnit = import ./nix/dynamic.nix { inherit pkgs; };
#   in
#     cargoUnit.buildWorkspaceDynamic {
#       src = ./.;
#       rustToolchain = pkgs.rust-bin.nightly.latest.default;
#     }
{
  pkgs,
  nix-cargo-unit ? pkgs.nix-cargo-unit or (throw "nix-cargo-unit not found in pkgs, pass it explicitly"),
}:
let
  inherit (pkgs) lib;

  # Import the base library for shared utilities
  baseLib = import ./lib.nix {
    inherit pkgs nix-cargo-unit;
  };

  # Build a Rust workspace using dynamic derivations (no IFD)
  #
  # This creates a single derivation that:
  # 1. Generates the unit graph with cargo
  # 2. Converts it to Nix with nix-cargo-unit
  # 3. Uses recursive nix-build to build the generated derivations
  #
  # The key difference from buildWorkspace is that this doesn't use IFD -
  # the Nix generation and building happens inside the derivation's build phase.
  #
  # Arguments:
  #   src: Path to the cargo workspace
  #   rustToolchain: Rust toolchain to use (must be nightly for unit-graph)
  #   hostRustToolchain: Host toolchain for proc-macros in cross-compilation
  #   profile: Build profile ("release" or "dev")
  #   cargoArgs: Additional args to pass to cargo
  #   contentAddressed: Enable CA-derivations (required for dynamic derivations)
  #   crossCompile: Enable cross-compilation mode
  #   hostPlatform: Host platform triple
  #   targetPlatform: Target platform triple
  #   filterSource: Enable source filtering (default: true)
  #   extraSourcePaths: Additional paths to include when filtering
  #   targets: Which targets to build (default: all roots)
  buildWorkspaceDynamic =
    {
      src,
      rustToolchain,
      hostRustToolchain ? rustToolchain,
      profile ? "release",
      cargoArgs ? "",
      contentAddressed ? true, # Required for dynamic derivations
      crossCompile ? false,
      hostPlatform ? null,
      targetPlatform ? null,
      filterSource ? true,
      extraSourcePaths ? [ ],
      targets ? [ "default" ], # Which targets to build: ["default"], ["all"], or list of names
    }:
    let
      # Apply source filtering if enabled
      filteredSrc =
        if filterSource then
          baseLib.filterRustSource {
            src = src;
            extraPaths = extraSourcePaths;
          }
        else
          src;

      # Build flags for nix-cargo-unit
      nixCargoUnitFlags = lib.concatStringsSep " " (
        [
          "-w $PWD"
        ]
        ++ lib.optional contentAddressed "--content-addressed"
        ++ lib.optional crossCompile "--cross-compile"
        ++ lib.optional (hostPlatform != null) "--host-platform ${hostPlatform}"
        ++ lib.optional (targetPlatform != null) "--target-platform ${targetPlatform}"
      );

      # Determine which attribute to build from the generated Nix
      targetAttr =
        if targets == [ "default" ] then
          "default"
        else if targets == [ "all" ] then
          "roots"
        else
          "packages";

      # Script to build specific targets
      buildTargetsScript =
        if targets == [ "default" ] then
          ''
            # Build the default target
            nix-build units.nix \
              --arg pkgs "import <nixpkgs> {}" \
              --arg rustToolchain "$RUST_TOOLCHAIN_DRV" \
              --arg hostRustToolchain "$HOST_RUST_TOOLCHAIN_DRV" \
              --arg src "$PWD" \
              -A default \
              -o $out
          ''
        else if targets == [ "all" ] then
          ''
            # Build all roots and symlink them
            mkdir -p $out/bin $out/lib

            for i in $(nix-instantiate units.nix \
              --arg pkgs "import <nixpkgs> {}" \
              --arg rustToolchain "$RUST_TOOLCHAIN_DRV" \
              --arg hostRustToolchain "$HOST_RUST_TOOLCHAIN_DRV" \
              --arg src "$PWD" \
              -A roots --eval --json | jq -r '.[]'); do
              result=$(nix-build "$i")
              # Link outputs
              if [ -d "$result/bin" ]; then
                for f in "$result/bin/"*; do
                  ln -sf "$f" "$out/bin/"
                done
              fi
              if [ -d "$result/lib" ]; then
                for f in "$result/lib/"*; do
                  ln -sf "$f" "$out/lib/"
                done
              fi
            done
          ''
        else
          ''
            # Build specific targets
            mkdir -p $out/bin $out/lib

            ${lib.concatMapStringsSep "\n" (target: ''
              result=$(nix-build units.nix \
                --arg pkgs "import <nixpkgs> {}" \
                --arg rustToolchain "$RUST_TOOLCHAIN_DRV" \
                --arg hostRustToolchain "$HOST_RUST_TOOLCHAIN_DRV" \
                --arg src "$PWD" \
                -A 'packages."${target}"')
              if [ -d "$result/bin" ]; then
                for f in "$result/bin/"*; do
                  ln -sf "$f" "$out/bin/"
                done
              fi
              if [ -d "$result/lib" ]; then
                for f in "$result/lib/"*; do
                  ln -sf "$f" "$out/lib/"
                done
              fi
            '') targets}
          '';
    in
    pkgs.stdenv.mkDerivation {
      name = "cargo-workspace-dynamic";

      # Use __structuredAttrs for better handling of complex attributes
      __structuredAttrs = true;

      # Content-addressed output (required for dynamic derivations to work well)
      __contentAddressed = contentAddressed;
      outputHashMode = "recursive";
      outputHashAlgo = "sha256";

      src = filteredSrc;

      nativeBuildInputs = [
        rustToolchain
        hostRustToolchain
        nix-cargo-unit
        pkgs.cacert
        pkgs.nix # For recursive nix-build/nix-instantiate
        pkgs.jq # For parsing JSON output
      ];

      # Pass toolchain paths as strings for use in recursive nix
      RUST_TOOLCHAIN_PATH = "${rustToolchain}";
      HOST_RUST_TOOLCHAIN_PATH = "${hostRustToolchain}";
      NIX_CARGO_UNIT_PATH = "${nix-cargo-unit}";

      # Environment for cargo and nix
      SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      NIX_PATH = "nixpkgs=${pkgs.path}";

      # Enable recursive Nix
      requiredSystemFeatures = [ "recursive-nix" ];

      buildPhase = ''
        runHook preBuild

        # Set up cargo home
        export CARGO_HOME="$TMPDIR/cargo-home"
        mkdir -p "$CARGO_HOME"

        # Store toolchain paths as Nix store paths for recursive nix
        # We need to pass these as derivation references
        RUST_TOOLCHAIN_DRV="${rustToolchain}"
        HOST_RUST_TOOLCHAIN_DRV="${hostRustToolchain}"

        echo "=== Generating unit graph ==="
        cargo build \
          --unit-graph \
          -Z unstable-options \
          ${lib.optionalString (profile == "release") "--release"} \
          ${cargoArgs} \
          2>/dev/null > unit-graph.json

        echo "=== Converting to Nix derivations ==="
        nix-cargo-unit ${nixCargoUnitFlags} < unit-graph.json > units.nix

        echo "=== Building with recursive Nix ==="
        # Use recursive nix to build the generated derivations
        # This is the key step that makes dynamic derivations work

        ${buildTargetsScript}

        runHook postBuild
      '';

      # No install phase needed - build phase creates $out
      dontInstall = true;
    };

  # Build a single package from a workspace using dynamic derivations
  buildPackageDynamic =
    {
      src,
      rustToolchain,
      package,
      cargoArgs ? "",
      ...
    }@args:
    buildWorkspaceDynamic (
      args
      // {
        cargoArgs = "-p ${package} ${cargoArgs}";
        targets = [ package ];
      }
    );

  # Build all binaries using dynamic derivations
  buildBinariesDynamic =
    {
      src,
      rustToolchain,
      cargoArgs ? "",
      ...
    }@args:
    buildWorkspaceDynamic (
      args
      // {
        cargoArgs = "--bins ${cargoArgs}";
        targets = [ "all" ];
      }
    );

in
{
  inherit
    buildWorkspaceDynamic
    buildPackageDynamic
    buildBinariesDynamic
    ;

  # Re-export base library functions
  inherit (baseLib)
    generateUnitGraph
    generateNixFromUnitGraph
    filterRustSource
    filterCrateSource
    ;

  # Make the library callable directly
  __functor = self: self.buildWorkspaceDynamic;
}
