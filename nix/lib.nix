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
#       # Required: pinned Rust version (e.g., "1.84.0" or "nightly-2026-01-14")
#       rustVersion = "nightly-2026-01-14";
#     }
{
  pkgs,
  nix-cargo-unit ? pkgs.nix-cargo-unit or (throw "nix-cargo-unit not found in pkgs, pass it explicitly"),
}:
let
  inherit (pkgs) lib;

  # Read and validate rust version from rust-toolchain.toml
  #
  # Parses rust-toolchain.toml and extracts the channel.
  # Validates that the channel is precisely pinned (not floating).
  #
  # Arguments:
  #   src: Path to the source tree containing rust-toolchain.toml
  #
  # Returns: The validated version string (e.g., "nightly-2026-01-14" or "1.84.0")
  #
  # Throws if:
  #   - rust-toolchain.toml doesn't exist
  #   - Channel is not precisely pinned (e.g., "nightly" without date)
  readRustVersion =
    src:
    let
      toolchainFile = src + "/rust-toolchain.toml";
      hasFile = builtins.pathExists toolchainFile;
      contents = if hasFile then builtins.readFile toolchainFile else null;
      parsed = if contents != null then builtins.fromTOML contents else null;
      channel = if parsed != null then parsed.toolchain.channel or null else null;

      # Validate the channel is precisely pinned
      isStable = channel != null && builtins.match "[0-9]+\\.[0-9]+\\.[0-9]+" channel != null;
      isNightlyPinned = channel != null && builtins.match "nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}" channel != null;
      isBetaPinned = channel != null && builtins.match "beta-[0-9]{4}-[0-9]{2}-[0-9]{2}" channel != null;
      isPinned = isStable || isNightlyPinned || isBetaPinned;
    in
    if !hasFile then
      throw ''
        nix-cargo-unit requires a rust-toolchain.toml file in the source root.

        Create one with a pinned Rust version:
          [toolchain]
          channel = "nightly-2026-01-14"  # or "1.84.0" for stable
      ''
    else if channel == null then
      throw ''
        rust-toolchain.toml must contain [toolchain].channel

        Example:
          [toolchain]
          channel = "nightly-2026-01-14"
      ''
    else if !isPinned then
      throw ''
        rust-toolchain.toml channel must be precisely pinned.

        Found: "${channel}"

        Valid formats:
          - Stable: "1.84.0" (specific version number)
          - Nightly: "nightly-2026-01-14" (with specific date)
          - Beta: "beta-2026-01-14" (with specific date)

        Invalid formats:
          - "nightly" (no date - floating)
          - "stable" (no version - floating)
          - "beta" (no date - floating)

        This requirement ensures reproducible builds.
      ''
    else
      channel;

  # Create a rust toolchain from a pinned version string
  #
  # Parses version strings like:
  #   - "1.84.0" -> pkgs.rust-bin.stable."1.84.0".default
  #   - "nightly-2026-01-14" -> pkgs.rust-bin.nightly."2026-01-14".default
  #   - "beta-2026-01-14" -> pkgs.rust-bin.beta."2026-01-14".default
  #
  # This requires rust-overlay to be applied to pkgs.
  toolchainFromVersion =
    version:
    let
      nightlyMatch = builtins.match "nightly-([0-9]{4}-[0-9]{2}-[0-9]{2})" version;
      betaMatch = builtins.match "beta-([0-9]{4}-[0-9]{2}-[0-9]{2})" version;
      isStable = builtins.match "[0-9]+\\.[0-9]+\\.[0-9]+" version != null;
    in
    if nightlyMatch != null then
      pkgs.rust-bin.nightly.${builtins.elemAt nightlyMatch 0}.default
    else if betaMatch != null then
      pkgs.rust-bin.beta.${builtins.elemAt betaMatch 0}.default
    else if isStable then
      pkgs.rust-bin.stable.${version}.default
    else
      throw ''
        Invalid rustVersion format: "${version}"

        Expected formats:
          - Stable: "1.84.0"
          - Nightly: "nightly-2026-01-14"
          - Beta: "beta-2026-01-14"
      '';

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
      vendorDir ? null,
    }:
    pkgs.runCommand "unit-graph.json"
      {
        nativeBuildInputs = [
          rustToolchain
          pkgs.cacert # For fetching crates (if not vendored)
        ];

        # SSL cert for downloading crates
        SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
      }
      ''
        # Set up cargo home in the temp directory (must expand $TMPDIR in shell)
        export CARGO_HOME="$TMPDIR/cargo-home"
        mkdir -p "$CARGO_HOME"

        ${lib.optionalString (vendorDir != null) ''
        # Configure cargo to use vendored dependencies
        # The .cargo/config.toml from importCargoLock has the git source replacements
        # but uses a relative path. We need to fix it to use the absolute nix store path.
        if [ -f "${vendorDir}/.cargo/config.toml" ]; then
          # Copy and fix the directory path to be absolute
          sed 's|directory = "cargo-vendor-dir"|directory = "${vendorDir}"|' \
            "${vendorDir}/.cargo/config.toml" > "$CARGO_HOME/config.toml"
        else
          cat > "$CARGO_HOME/config.toml" << EOF
        [source.crates-io]
        replace-with = "vendored-sources"

        [source.vendored-sources]
        directory = "${vendorDir}"
        EOF
        fi
        ''}

        cd ${src}

        # Generate unit graph
        # --unit-graph requires -Z unstable-options (nightly)
        # We use --release by default for optimized builds
        cargo build \
          --unit-graph \
          -Z unstable-options \
          ${lib.optionalString (profile == "release") "--release"} \
          ${lib.optionalString (vendorDir != null) "--offline"} \
          ${cargoArgs} \
          > $out
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

  # Validate that a Rust version string is precisely pinned (not floating)
  #
  # Valid examples:
  #   - "1.84.0" (stable version)
  #   - "nightly-2026-01-14" (nightly with date)
  #   - "beta-2026-01-14" (beta with date)
  #
  # Invalid examples:
  #   - "latest" (floating)
  #   - "nightly" (floating, no date)
  #   - "stable" (floating)
  #
  assertPinnedRustVersion =
    version:
    let
      # Stable versions: X.Y.Z format
      isStableVersion = builtins.match "[0-9]+\\.[0-9]+\\.[0-9]+" version != null;
      # Nightly/beta with date: channel-YYYY-MM-DD format
      isChannelWithDate = builtins.match "(nightly|beta)-[0-9]{4}-[0-9]{2}-[0-9]{2}" version != null;
      isPinned = isStableVersion || isChannelWithDate;
    in
    if !isPinned then
      throw ''
        nix-cargo-unit requires a precisely pinned Rust version.

        You provided: "${version}"

        Valid formats:
          - Stable: "1.84.0" (specific version number)
          - Nightly: "nightly-2026-01-14" (with specific date)
          - Beta: "beta-2026-01-14" (with specific date)

        Invalid formats:
          - "latest", "stable", "nightly", "beta" (floating versions)

        This requirement ensures reproducible builds and prevents ABI
        incompatibility between cached crates compiled with different
        compiler versions.

        Example usage with rust-overlay:
          rustToolchain = pkgs.rust-bin.stable."1.84.0".default;
          # or
          rustToolchain = pkgs.rust-bin.nightly."2026-01-14".default;
      ''
    else
      true;

  # Build a Rust workspace using IFD
  #
  # This is the main entry point. It:
  # 1. Generates the unit graph using cargo
  # 2. Converts it to Nix derivations using nix-cargo-unit
  # 3. Imports the generated Nix (IFD) and builds the workspace
  #
  # Arguments:
  #   src: Path to the cargo workspace (must contain rust-toolchain.toml with pinned version)
  #   rustVersion: Optional - Override version from rust-toolchain.toml
  #                If not provided, reads from rust-toolchain.toml (MUST be pinned)
  #   rustToolchain: Optional - Custom toolchain derivation. If not provided, derived from rustVersion.
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
      # Auto-read from rust-toolchain.toml if not provided
      rustVersion ? readRustVersion src,
      rustToolchain ? toolchainFromVersion rustVersion,
      hostRustToolchain ? rustToolchain,
      profile ? "release",
      cargoArgs ? "",
      contentAddressed ? true,
      crossCompile ? false,
      hostPlatform ? null,
      targetPlatform ? null,
      filterSource ? true,
      extraSourcePaths ? [ ],
      # Extra native build inputs for build scripts (e.g., protobuf, cmake)
      nativeBuildInputs ? [ ],
      # Path to Cargo.lock for vendoring external deps
      # If not provided, will try src/Cargo.lock
      cargoLock ? null,
      # Output hashes for git dependencies (passed to importCargoLock)
      outputHashes ? { },
    }:
    let
      # Validate that the Rust version is precisely pinned
      _ = assertPinnedRustVersion rustVersion;

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

      # Vendor dependencies if Cargo.lock is available
      # This creates a directory with all crates from crates.io/git
      lockFile = if cargoLock != null then cargoLock else (src + "/Cargo.lock");
      hasLockFile = builtins.pathExists lockFile;
      vendorDir =
        if hasLockFile then
          pkgs.rustPlatform.importCargoLock {
            inherit lockFile outputHashes;
          }
        else
          null;

      # Step 1: Generate unit graph JSON (IFD step 1)
      # Use filtered source for unit graph generation
      unitGraphJson = generateUnitGraph {
        src = filteredSrc;
        inherit rustToolchain cargoArgs profile vendorDir;
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
        inherit pkgs rustToolchain vendorDir;
        inherit hostRustToolchain;
        src = filteredSrc;
        extraNativeBuildInputs = nativeBuildInputs;
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

      # Expose vendor directory for debugging
      inherit vendorDir;
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
