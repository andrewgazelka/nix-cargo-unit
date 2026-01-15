//! Integration tests for nix-cargo-unit.
//!
//! These tests verify the full pipeline:
//! 1. Parsing cargo's unit graph JSON
//! 2. Generating Nix derivations
//! 3. Verifying output structure and content

use std::process::Command;

/// Path to the example workspace relative to the crate root.
const EXAMPLE_WORKSPACE: &str = "examples/workspace";

/// Helper to run cargo --unit-graph on the example workspace.
fn get_unit_graph() -> String {
    // In Nix, the toolchain is provided directly (no rustup), so we can't use +nightly
    // Detect by checking if `cargo +nightly` works, fall back to plain cargo
    let mut cmd = Command::new("cargo");

    // Try +nightly first (rustup environment)
    let nightly_check = Command::new("cargo")
        .args(["+nightly", "--version"])
        .output();

    let use_nightly_flag = nightly_check.map(|o| o.status.success()).unwrap_or(false);

    if use_nightly_flag {
        cmd.arg("+nightly");
    }

    cmd.args([
        "build",
        "--unit-graph",
        "-Z",
        "unstable-options",
        "--release",
    ]);
    cmd.current_dir(EXAMPLE_WORKSPACE);

    let output = cmd.output().expect("failed to run cargo --unit-graph");

    if !output.status.success() {
        panic!(
            "cargo --unit-graph failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    String::from_utf8(output.stdout).expect("invalid UTF-8 from cargo")
}

/// Parse a unit graph JSON string.
fn parse_unit_graph(json: &str) -> nix_cargo_unit::unit_graph::UnitGraph {
    serde_json::from_str(json).expect("failed to parse unit graph JSON")
}

#[test]
fn test_example_workspace_unit_graph_parses() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    // Should have multiple units (at least: core lib, macros, app, build scripts)
    assert!(graph.units.len() >= 3, "expected at least 3 units");

    // Should have root units
    assert!(!graph.roots.is_empty(), "expected at least one root");
}

#[test]
fn test_example_workspace_has_build_scripts() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    // Should have build script units (mode = "run-custom-build")
    let build_scripts: Vec<_> = graph
        .units
        .iter()
        .filter(|u| u.mode == "run-custom-build")
        .collect();

    // Both core and app have build.rs
    assert!(
        build_scripts.len() >= 2,
        "expected at least 2 build script run units, got {}",
        build_scripts.len()
    );
}

#[test]
fn test_example_workspace_has_proc_macro() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    // Should have proc-macro units
    let proc_macros: Vec<_> = graph.units.iter().filter(|u| u.is_proc_macro()).collect();

    assert!(
        !proc_macros.is_empty(),
        "expected at least 1 proc-macro unit"
    );

    // The proc-macro should be example-macros
    let has_example_macros = proc_macros
        .iter()
        .any(|u| u.target.name.contains("example_macros"));
    assert!(has_example_macros, "expected example_macros proc-macro");
}

#[test]
fn test_nix_generation_produces_valid_structure() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: true,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Check Nix structure
    assert!(
        nix.contains("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src }:"),
        "missing function signature"
    );
    assert!(nix.contains("let"), "missing let block");
    assert!(nix.contains("mkUnit = attrs:"), "missing mkUnit helper");
    assert!(nix.contains("units = {"), "missing units attrset");
    assert!(nix.contains("roots = ["), "missing roots list");
    assert!(nix.contains("packages = {"), "missing packages attrset");
    assert!(nix.contains("binaries = {"), "missing binaries attrset");
    assert!(nix.contains("libraries = {"), "missing libraries attrset");
    assert!(nix.contains("default ="), "missing default");

    // Check CA attributes since we enabled them
    assert!(
        nix.contains("__contentAddressed = true"),
        "missing CA attribute"
    );
    assert!(
        nix.contains("outputHashMode = \"recursive\""),
        "missing outputHashMode"
    );
    assert!(
        nix.contains("outputHashAlgo = \"sha256\""),
        "missing outputHashAlgo"
    );
}

#[test]
fn test_nix_generation_has_example_derivations() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Check for example-app derivation
    assert!(
        nix.contains("pname = \"example-app\"") || nix.contains("\"example-app\""),
        "missing example-app derivation"
    );

    // Check for example_core derivation
    assert!(
        nix.contains("pname = \"example_core\""),
        "missing example_core derivation"
    );

    // Check for example_macros derivation
    assert!(
        nix.contains("pname = \"example_macros\""),
        "missing example_macros derivation"
    );

    // Check for build script derivations
    assert!(
        nix.contains("-build-script-"),
        "missing build script derivations"
    );

    // Check that build script outputs are wired
    assert!(
        nix.contains("BUILD_SCRIPT_FLAGS"),
        "missing BUILD_SCRIPT_FLAGS"
    );
    assert!(
        nix.contains("rustc-cfg"),
        "missing rustc-cfg output parsing"
    );
}

#[test]
fn test_nix_generation_has_dependency_wiring() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Check for --extern flags (dependency wiring)
    assert!(nix.contains("--extern"), "missing --extern flags");

    // Check for -L library search paths
    assert!(
        nix.contains("-L") || nix.contains("dependency="),
        "missing -L library search paths"
    );

    // Check for buildInputs with dependencies
    assert!(nix.contains("buildInputs = ["), "missing buildInputs");
}

#[test]
fn test_unit_identity_hashes_are_unique() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let mut seen_names = std::collections::HashSet::new();

    for unit in &graph.units {
        let name = unit.derivation_name();

        // Each unit should have a unique derivation name
        assert!(
            seen_names.insert(name.clone()),
            "duplicate derivation name: {}",
            name
        );
    }
}

#[test]
fn test_proc_macro_output_is_shared_library() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Proc-macros should use --crate-type proc-macro which produces a shared library
    // The extern references use find to locate the .so file
    assert!(
        nix.contains("--crate-type proc-macro")
            || nix.contains("find") && nix.contains("libexample_macros"),
        "proc-macro should use proc-macro crate type"
    );
}

#[test]
fn test_binary_output_is_in_bin_dir() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Binaries should output to $out/bin/ in installPhase
    assert!(
        nix.contains("cp build/example-app $out/bin/"),
        "binary should be copied to $out/bin/ in installPhase"
    );
}

#[test]
fn test_library_output_is_rlib() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Libraries should output to .rlib
    assert!(
        nix.contains("libexample_core.rlib"),
        "library should output to .rlib file"
    );
}

#[test]
fn test_rustc_flags_include_edition() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Should have --edition flag
    assert!(nix.contains("--edition"), "missing --edition flag");

    // Example workspace uses edition 2024
    assert!(
        nix.contains("2024"),
        "missing edition 2024 (example uses edition.workspace = true)"
    );
}

#[test]
fn test_source_paths_are_remapped() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // Source paths should use ${src} variable
    assert!(
        nix.contains("${src}"),
        "source paths should be remapped to use \\${{src}}"
    );

    // Should reference crate paths relative to workspace
    assert!(
        nix.contains("${src}/crates/") || nix.contains("''${src}/crates/"),
        "should have relative crate paths"
    );
}

#[test]
fn test_workspace_outputs_map_targets() {
    let json = get_unit_graph();
    let graph = parse_unit_graph(&json);

    let workspace_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EXAMPLE_WORKSPACE);

    let config = nix_cargo_unit::nix_gen::NixGenConfig {
        workspace_root: workspace_root.to_string_lossy().to_string(),
        content_addressed: false,
        ..Default::default()
    };

    let generator = nix_cargo_unit::nix_gen::NixGenerator::new(config);
    let nix = generator.generate(&graph);

    // packages should map target names to derivations
    assert!(nix.contains("packages = {"), "should have packages attrset");

    // binaries should only have binary targets
    assert!(nix.contains("binaries = {"), "should have binaries attrset");

    // libraries should only have library/proc-macro targets
    assert!(
        nix.contains("libraries = {"),
        "should have libraries attrset"
    );

    // Check that our specific targets are in the right places
    // example-app should be in binaries
    let binaries_section = nix
        .split("# Binary targets only")
        .nth(1)
        .and_then(|s| s.split("# Library targets only").next())
        .unwrap_or("");
    assert!(
        binaries_section.contains("example-app"),
        "example-app should be in binaries"
    );

    // example_core and example_macros should be in libraries
    let libraries_section = nix
        .split("# Library targets only")
        .nth(1)
        .and_then(|s| s.split("default =").next())
        .unwrap_or("");
    assert!(
        libraries_section.contains("example_core"),
        "example_core should be in libraries"
    );
    assert!(
        libraries_section.contains("example_macros"),
        "example_macros should be in libraries"
    );
}
