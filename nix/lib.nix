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

  # Filter source to only include Rust-relevant files using lib.fileset
  #
  # This reduces the input hash for each derivation, improving cache hits.
  # Changes to non-Rust files (docs, CI configs, etc.) won't invalidate builds.
  #
  # Arguments:
  #   src: The source tree to filter
  #   extraPaths: Additional paths to include (e.g., build.rs locations)
  filterRustSource =
    {
      src,
      extraPaths ? [ ],
    }:
    let
      # Create a fileset from a path, returning null if it doesn't exist
      # This handles optional files like Cargo.lock gracefully
      optionalPath = path: lib.fileset.maybeMissing path;

      # Standard Rust source patterns
      rustFilesets = [
        # Cargo manifests and lock file
        (optionalPath (src + "/Cargo.toml"))
        (optionalPath (src + "/Cargo.lock"))
        # Workspace Cargo.toml files in common locations
        (optionalPath (src + "/crates"))
        # Source directories
        (optionalPath (src + "/src"))
        # Build scripts at root
        (optionalPath (src + "/build.rs"))
        # Benches and tests
        (optionalPath (src + "/benches"))
        (optionalPath (src + "/tests"))
        # Examples
        (optionalPath (src + "/examples"))
      ];

      # Filter to only .rs, .toml, and build-related files
      rustFilesFilter = lib.fileset.fileFilter (
        file:
        lib.any (ext: file.hasExt ext) [
          "rs"
          "toml"
        ]
        || file.name == "Cargo.lock"
        || file.name == "build.rs"
      ) src;

      # Combine standard paths with extra paths and filter
      allFilesets =
        rustFilesets ++ (map (p: optionalPath (src + "/${p}")) extraPaths) ++ [ rustFilesFilter ];

      # Filter out null values (non-existent paths) and create union
      validFilesets = builtins.filter (x: x != null) allFilesets;
    in
    if validFilesets == [ ] then
      src
    else
      lib.fileset.toSource {
        root = src;
        fileset = lib.fileset.unions validFilesets;
      };

  # Filter source for a specific crate within a workspace
  #
  # This is more aggressive filtering - only includes the specific crate's
  # source directory plus workspace-level Cargo files.
  #
  # Arguments:
  #   src: The workspace source tree
  #   cratePath: Relative path to the crate (e.g., "crates/core")
  filterCrateSource =
    {
      src,
      cratePath,
    }:
    let
      optionalPath = path: lib.fileset.maybeMissing path;

      crateFilesets = [
        # Workspace-level Cargo files
        (optionalPath (src + "/Cargo.toml"))
        (optionalPath (src + "/Cargo.lock"))
        # The crate's entire directory
        (optionalPath (src + "/${cratePath}"))
      ];

      validFilesets = builtins.filter (x: x != null) crateFilesets;
    in
    if validFilesets == [ ] then
      src
    else
      lib.fileset.toSource {
        root = src;
        fileset = lib.fileset.unions validFilesets;
      };

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
  #   filterSource: Enable source filtering to reduce input hash (default: true)
  #   extraSourcePaths: Additional paths to include when filtering (e.g., ["proto", "assets"])
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
      filterSource ? true,
      extraSourcePaths ? [ ],
    }:
    let
      # Apply source filtering if enabled
      # This reduces the input hash by excluding non-Rust files
      filteredSrc =
        if filterSource then
          filterRustSource {
            src = src;
            extraPaths = extraSourcePaths;
          }
        else
          src;

      # Step 1: Generate unit graph JSON (IFD step 1)
      # Use filtered source for unit graph generation
      unitGraphJson = generateUnitGraph {
        src = filteredSrc;
        inherit rustToolchain cargoArgs profile;
      };

      # Get the absolute workspace root path as a string for source remapping
      # Use the filtered source path for consistent remapping
      workspaceRoot = toString filteredSrc;

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
      # Pass filtered source to the generated derivations
      units = import unitsNix {
        inherit pkgs rustToolchain;
        inherit hostRustToolchain;
        src = filteredSrc;
      };
    in
    {
      # The imported units attrset (all derivations indexed by name)
      inherit (units) units;

      # Root derivations (the final build targets)
      inherit (units) roots;

      # Workspace packages by target name (for multi-crate workspaces)
      # Usage: result.packages.my-binary or result.packages."my-lib"
      inherit (units) packages;

      # Binary targets only (convenient for deployment)
      # Usage: result.binaries.my-app
      inherit (units) binaries;

      # Library targets only
      # Usage: result.libraries.my-lib
      inherit (units) libraries;

      # Default output (typically the first root unit)
      inherit (units) default;

      # Expose the intermediate derivations for debugging
      inherit unitGraphJson unitsNix;

      # Expose the filtered source for inspection
      inherit filteredSrc;
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

  # Build a specific workspace member by package name
  #
  # This is more efficient than buildWorkspace when you only need one package,
  # as cargo can skip building unrelated workspace members.
  #
  # Arguments:
  #   src: Path to the cargo workspace
  #   rustToolchain: Rust toolchain to use
  #   package: Name of the package to build (as specified in Cargo.toml)
  #   ... other buildWorkspace args
  buildPackage =
    {
      src,
      rustToolchain,
      package,
      cargoArgs ? "",
      ...
    }@args:
    let
      # Add -p flag to build only the specified package
      packageArgs = "-p ${package} ${cargoArgs}";
      result = buildWorkspace (args // { cargoArgs = packageArgs; });
    in
    result
    // {
      # Override default to be the requested package if it exists
      default = result.packages.${package} or result.default;
    };

  # Build all binaries in a workspace
  #
  # Convenience function that builds all binary targets and returns them
  # as an attrset.
  buildBinaries =
    {
      src,
      rustToolchain,
      cargoArgs ? "",
      ...
    }@args:
    let
      # Use --bins to build all binaries
      result = buildWorkspace (args // { cargoArgs = "--bins ${cargoArgs}"; });
    in
    result.binaries;

  # Build all libraries in a workspace
  #
  # Convenience function that builds all library targets.
  buildLibraries =
    {
      src,
      rustToolchain,
      cargoArgs ? "",
      ...
    }@args:
    let
      # Use --lib to build libraries
      result = buildWorkspace (args // { cargoArgs = "--lib ${cargoArgs}"; });
    in
    result.libraries;

in
{
  inherit
    generateUnitGraph
    generateNixFromUnitGraph
    buildWorkspace
    buildCrate
    buildPackage
    buildBinaries
    buildLibraries
    # Source filtering utilities
    filterRustSource
    filterCrateSource
    ;

  # Make the library callable directly
  __functor = self: self.buildWorkspace;
}
