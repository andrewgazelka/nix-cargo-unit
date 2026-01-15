# nix-cargo-unit library for IFD-based per-unit Rust builds
#
# This library provides functions to build Rust projects with fine-grained
# caching at the compilation unit level using Import From Derivation.
#
# Usage:
#   let
#     cargoUnit = import ./nix/lib.nix { inherit pkgs; };
#   in
#     cargoUnit.buildWorkspace {
#       src = ./.;
#       # Optional: override the rust toolchain
#       rustToolchain = pkgs.rust-bin.stable.latest.default;
#     }
{
  pkgs,
  nix-cargo-unit ? pkgs.nix-cargo-unit or (throw "nix-cargo-unit not found in pkgs, pass it explicitly"),
}:
let
  inherit (pkgs) lib;

  # Generate the unit graph JSON from a cargo workspace
  #
  # This runs `cargo +nightly --unit-graph` and outputs the JSON to $out.
  # Requires nightly cargo for the unstable --unit-graph flag.
  generateUnitGraph =
    {
      src,
      rustToolchain,
      cargoArgs ? "",
      profile ? "release",
    }:
    pkgs.runCommand "unit-graph.json"
      {
        nativeBuildInputs = [
          rustToolchain
          pkgs.cacert # For fetching crates
        ];

        # Environment for cargo
        CARGO_HOME = "$TMPDIR/cargo-home";
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      }
      ''
        mkdir -p $CARGO_HOME
        cd ${src}

        # Generate unit graph
        # --unit-graph requires -Z unstable-options (nightly)
        # We use --release by default for optimized builds
        cargo build \
          --unit-graph \
          -Z unstable-options \
          ${lib.optionalString (profile == "release") "--release"} \
          ${cargoArgs} \
          2>/dev/null > $out
      '';

  # Convert unit graph JSON to Nix derivations using nix-cargo-unit
  #
  # This takes the unit graph JSON and runs it through nix-cargo-unit
  # to generate Nix expressions that can be imported.
  generateNixFromUnitGraph =
    {
      unitGraphJson,
      workspaceRoot,
      contentAddressed ? true,
      crossCompile ? false,
      hostPlatform ? null,
      targetPlatform ? null,
    }:
    let
      flags = lib.concatStringsSep " " (
        [
          "-w ${workspaceRoot}"
        ]
        ++ lib.optional contentAddressed "--content-addressed"
        ++ lib.optional crossCompile "--cross-compile"
        ++ lib.optional (hostPlatform != null) "--host-platform ${hostPlatform}"
        ++ lib.optional (targetPlatform != null) "--target-platform ${targetPlatform}"
      );
    in
    pkgs.runCommand "units.nix"
      {
        nativeBuildInputs = [ nix-cargo-unit ];
      }
      ''
        nix-cargo-unit ${flags} < ${unitGraphJson} > $out
      '';

  # Build a Rust workspace using IFD
  #
  # This is the main entry point. It:
  # 1. Generates the unit graph using cargo
  # 2. Converts it to Nix derivations using nix-cargo-unit
  # 3. Imports the generated Nix (IFD) and builds the workspace
  #
  # Arguments:
  #   src: Path to the cargo workspace
  #   rustToolchain: Rust toolchain to use (must be nightly for unit-graph)
  #   hostRustToolchain: Host toolchain for proc-macros in cross-compilation
  #   profile: Build profile ("release" or "dev")
  #   cargoArgs: Additional args to pass to cargo
  #   contentAddressed: Enable CA-derivations for deduplication
  #   crossCompile: Enable cross-compilation mode
  #   hostPlatform: Host platform triple
  #   targetPlatform: Target platform triple
  buildWorkspace =
    {
      src,
      rustToolchain,
      hostRustToolchain ? rustToolchain,
      profile ? "release",
      cargoArgs ? "",
      contentAddressed ? true,
      crossCompile ? false,
      hostPlatform ? null,
      targetPlatform ? null,
    }:
    let
      # Step 1: Generate unit graph JSON (IFD step 1)
      unitGraphJson = generateUnitGraph {
        inherit
          src
          rustToolchain
          cargoArgs
          profile
          ;
      };

      # Get the absolute workspace root path as a string for source remapping
      # In Nix, we use the src derivation path
      workspaceRoot = toString src;

      # Step 2: Generate Nix expressions from unit graph (IFD step 2)
      unitsNix = generateNixFromUnitGraph {
        inherit
          unitGraphJson
          workspaceRoot
          contentAddressed
          crossCompile
          hostPlatform
          targetPlatform
          ;
      };

      # Step 3: Import the generated Nix (IFD - Import From Derivation)
      # This is where the magic happens - Nix evaluates a derivation output
      units = import unitsNix {
        inherit pkgs rustToolchain src;
        inherit hostRustToolchain;
      };
    in
    {
      # The imported units attrset (all derivations indexed by name)
      inherit (units) units;

      # Root derivations (the final build targets)
      inherit (units) roots;

      # Default output (typically the main binary/library)
      inherit (units) default;

      # Expose the intermediate derivations for debugging
      inherit unitGraphJson unitsNix;
    };

  # Build a single crate (non-workspace) using IFD
  #
  # Convenience wrapper for buildWorkspace that works with single crates.
  buildCrate =
    {
      src,
      rustToolchain,
      ...
    }@args:
    buildWorkspace args;

in
{
  inherit
    generateUnitGraph
    generateNixFromUnitGraph
    buildWorkspace
    buildCrate
    ;

  # Make the library callable directly
  __functor = self: self.buildWorkspace;
}
