//! Nix derivation code generation.
//!
//! This module provides structured builders for generating Nix expressions
//! from cargo unit graph data. It focuses on proper escaping, composability,
//! and producing readable output.

use crate::build_script::{BuildScriptInfo, BuildScriptOutput};
use crate::rustc_flags::RustcFlags;
use crate::unit_graph::{Unit, UnitGraph};

/// A Nix string with proper escaping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixString(String);

impl NixString {
    /// Creates a new escaped Nix string.
    pub fn new(s: &str) -> Self {
        Self(escape_nix_string(s))
    }

    /// Creates a raw Nix expression (not escaped, used for variable references).
    pub fn raw(s: &str) -> Self {
        Self(s.to_string())
    }

    /// Returns the escaped string content.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NixString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Escapes a string for use in Nix.
///
/// Nix strings use `"..."` syntax with the following escape sequences:
/// - `\\` -> `\`
/// - `\"` -> `"`
/// - `\n` -> newline
/// - `\r` -> carriage return
/// - `\t` -> tab
/// - `${` -> literal `${` (interpolation escape)
fn escape_nix_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '$' => {
                // Check if next char is '{' - but we only have current char
                // So we escape all $ to be safe
                result.push_str("\\$");
            }
            c => result.push(c),
        }
    }
    result
}

/// Escapes a string for use in Nix multiline strings (''...'').
///
/// Multiline strings have different escape rules:
/// - `''$` -> literal `$`
/// - `'''` -> literal `''`
pub fn escape_nix_multiline(s: &str) -> String {
    s.replace("''", "'''").replace("${", "''${")
}

/// A builder for Nix attribute sets.
#[derive(Debug, Default)]
pub struct NixAttrSet {
    attrs: Vec<(String, String)>,
}

impl NixAttrSet {
    /// Creates a new empty attribute set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a string attribute.
    pub fn string(&mut self, key: &str, value: &str) -> &mut Self {
        self.attrs
            .push((key.to_string(), format!("\"{}\"", escape_nix_string(value))));
        self
    }

    /// Adds a raw Nix expression (not quoted).
    pub fn expr(&mut self, key: &str, value: &str) -> &mut Self {
        self.attrs.push((key.to_string(), value.to_string()));
        self
    }

    /// Adds a boolean attribute.
    pub fn bool(&mut self, key: &str, value: bool) -> &mut Self {
        self.attrs.push((
            key.to_string(),
            if value { "true" } else { "false" }.to_string(),
        ));
        self
    }

    /// Adds an integer attribute.
    pub fn int(&mut self, key: &str, value: i64) -> &mut Self {
        self.attrs.push((key.to_string(), value.to_string()));
        self
    }

    /// Adds a list of strings.
    pub fn string_list(&mut self, key: &str, values: &[String]) -> &mut Self {
        let items: Vec<String> = values
            .iter()
            .map(|v| format!("\"{}\"", escape_nix_string(v)))
            .collect();
        self.attrs
            .push((key.to_string(), format!("[ {} ]", items.join(" "))));
        self
    }

    /// Adds a list of raw expressions.
    pub fn expr_list(&mut self, key: &str, values: &[String]) -> &mut Self {
        self.attrs
            .push((key.to_string(), format!("[ {} ]", values.join(" "))));
        self
    }

    /// Adds a multiline string (using ''...'').
    pub fn multiline(&mut self, key: &str, value: &str) -> &mut Self {
        self.attrs.push((
            key.to_string(),
            format!("''\n{}\n''", escape_nix_multiline(value)),
        ));
        self
    }

    /// Renders the attribute set with the given indentation.
    pub fn render(&self, indent: usize) -> String {
        let base_indent = "  ".repeat(indent);
        let inner_indent = "  ".repeat(indent + 1);

        let mut out = String::new();
        out.push_str("{\n");

        for (key, value) in &self.attrs {
            // Handle multiline values specially
            if value.starts_with("''") && value.contains('\n') {
                let lines: Vec<&str> = value.lines().collect();
                out.push_str(&inner_indent);
                out.push_str(key);
                out.push_str(" = ");
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 {
                        out.push('\n');
                        out.push_str(&inner_indent);
                        out.push_str("  ");
                    }
                    out.push_str(line);
                }
                out.push_str(";\n");
            } else {
                out.push_str(&inner_indent);
                out.push_str(key);
                out.push_str(" = ");
                out.push_str(value);
                out.push_str(";\n");
            }
        }

        out.push_str(&base_indent);
        out.push('}');
        out
    }
}

/// A dependency reference for a unit derivation.
#[derive(Debug, Clone)]
pub struct DepRef {
    /// Nix variable name for the dependency derivation.
    pub nix_var: String,

    /// Extern crate name (used in --extern flag).
    pub extern_crate_name: String,

    /// Derivation name (for reference).
    pub derivation_name: String,

    /// Whether this is a proc-macro dependency.
    pub is_proc_macro: bool,
}

/// A build script output reference for a unit.
#[derive(Debug, Clone)]
pub struct BuildScriptRef {
    /// Nix variable name for the build script run derivation.
    pub run_drv_var: String,

    /// Derivation name for the compile derivation.
    pub compile_drv_name: String,

    /// Derivation name for the run derivation.
    pub run_drv_name: String,
}

/// A builder for a single unit derivation.
#[derive(Debug)]
pub struct UnitDerivation {
    /// Derivation name (unique identifier).
    pub name: String,

    /// Package name.
    pub pname: String,

    /// Package version.
    pub version: String,

    /// Rust edition.
    pub edition: String,

    /// Crate types being built.
    pub crate_types: Vec<String>,

    /// Entry point source path (Nix expression).
    pub src_path: String,

    /// Features enabled.
    pub features: Vec<String>,

    /// Optimization level.
    pub opt_level: String,

    /// Whether this is a test build.
    pub is_test: bool,

    /// Whether this is a proc-macro.
    pub is_proc_macro: bool,

    /// Dependencies with extern crate info.
    pub deps: Vec<DepRef>,

    /// Build script outputs this unit depends on (if any).
    pub build_script_ref: Option<BuildScriptRef>,

    /// The rustc flags (precomputed).
    pub rustc_flags: RustcFlags,

    /// Whether to use content-addressed derivations.
    pub content_addressed: bool,

    /// The Nix variable for the toolchain to use.
    /// Either "rustToolchain" or "hostRustToolchain" for cross-compilation.
    pub toolchain_var: String,
}

impl UnitDerivation {
    /// Creates a derivation builder from a unit.
    ///
    /// The `workspace_root` is used to remap absolute paths to Nix source paths.
    /// The `content_addressed` flag enables CA-derivation attributes.
    /// The `toolchain_var` specifies which toolchain to use (for cross-compilation).
    pub fn from_unit(
        unit: &Unit,
        workspace_root: &str,
        content_addressed: bool,
        toolchain_var: &str,
    ) -> Self {
        let name = unit.derivation_name();
        let pname = unit.target.name.clone();
        let version = unit.package_version().unwrap_or("0.0.0").to_string();

        // Remap source path
        let src_path =
            crate::source_filter::remap_source_path(&unit.target.src_path, workspace_root, "src");

        let rustc_flags = RustcFlags::from_unit(unit);

        Self {
            name,
            pname,
            version,
            edition: unit.target.edition.clone(),
            crate_types: unit.target.crate_types.clone(),
            src_path,
            features: unit.features.clone(),
            opt_level: unit.profile.opt_level.clone(),
            is_test: unit.is_test(),
            is_proc_macro: unit.is_proc_macro(),
            deps: Vec::new(),
            build_script_ref: None,
            rustc_flags,
            content_addressed,
            toolchain_var: toolchain_var.to_string(),
        }
    }

    /// Sets the build script reference for this unit.
    pub fn set_build_script_ref(&mut self, build_script_ref: BuildScriptRef) {
        self.build_script_ref = Some(build_script_ref);
    }

    /// Adds a dependency reference with extern crate info.
    pub fn add_dep(&mut self, dep_ref: DepRef) {
        self.deps.push(dep_ref);
    }

    /// Generates the Nix derivation expression.
    pub fn to_nix(&self) -> String {
        let mut attrs = NixAttrSet::new();

        attrs.string("pname", &self.pname);
        attrs.string("version", &self.version);

        // Build inputs (dependencies) - use the nix_var for each dep
        // Also include build script run derivation if present
        let mut dep_vars: Vec<String> = self.deps.iter().map(|d| d.nix_var.clone()).collect();
        if let Some(ref bs_ref) = self.build_script_ref {
            dep_vars.push(bs_ref.run_drv_var.clone());
        }

        if !dep_vars.is_empty() {
            attrs.expr_list("buildInputs", &dep_vars);
        } else {
            attrs.expr("buildInputs", "[]");
        }

        // Native build inputs (rust toolchain)
        // Use hostRustToolchain for proc-macros when cross-compiling
        attrs.expr("nativeBuildInputs", &format!("[ {} ]", self.toolchain_var));

        // Content-addressed derivation attributes
        if self.content_addressed {
            attrs.bool("__contentAddressed", true);
            attrs.string("outputHashMode", "recursive");
            attrs.string("outputHashAlgo", "sha256");
        }

        // Build phase with rustc invocation
        let build_phase = self.generate_build_phase();
        attrs.multiline("buildPhase", &build_phase);

        // Install phase
        attrs.multiline("installPhase", "mkdir -p $out");

        attrs.render(2)
    }

    /// Generates the build phase script.
    fn generate_build_phase(&self) -> String {
        let mut script = String::new();

        // Create output directories
        if self.crate_types.contains(&"bin".to_string()) {
            script.push_str("mkdir -p $out/bin\n");
        } else {
            script.push_str("mkdir -p $out/lib\n");
        }

        // Initialize build script flags variable
        script.push_str("BUILD_SCRIPT_FLAGS=\"\"\n");

        // Read build script outputs if this unit depends on a build script
        if let Some(ref bs_ref) = self.build_script_ref {
            script.push_str("\n");
            script.push_str(&BuildScriptOutput::generate_nix_flag_reader(
                &bs_ref.run_drv_var,
            ));
            script.push_str("\n");
        }

        script.push_str("rustc \\\n");

        // Add each flag on its own line for readability
        for arg in self.rustc_flags.args() {
            script.push_str("  ");
            // Escape for shell if needed
            if arg.contains(' ') || arg.contains('"') || arg.contains('$') {
                script.push('\'');
                script.push_str(&arg.replace('\'', "'\\''"));
                script.push('\'');
            } else {
                script.push_str(arg);
            }
            script.push_str(" \\\n");
        }

        // Add -L library search paths for all dependencies
        // This allows rustc to find transitive .rlib files
        for dep in &self.deps {
            script.push_str("  -L ");
            script.push_str(&format!("dependency={}/lib", dep.nix_var));
            script.push_str(" \\\n");
        }

        // Add --extern flags for each dependency
        for dep in &self.deps {
            script.push_str("  --extern ");
            // Determine the library file name based on whether it's a proc-macro
            let lib_file = if dep.is_proc_macro {
                // Proc-macros are shared libraries (.so on Linux, .dylib on macOS)
                // Use a glob pattern since extension varies by platform
                format!(
                    "{}=\"$(find {}/lib -name 'lib{}.*' -type f | head -1)\"",
                    dep.extern_crate_name,
                    dep.nix_var,
                    dep.extern_crate_name.replace('-', "_")
                )
            } else {
                // Regular dependencies use .rlib
                format!(
                    "{}={}/lib/lib{}.rlib",
                    dep.extern_crate_name,
                    dep.nix_var,
                    dep.extern_crate_name.replace('-', "_")
                )
            };
            script.push_str(&lib_file);
            script.push_str(" \\\n");
        }

        // Add source file
        script.push_str("  ");
        script.push_str(&self.src_path);
        script.push_str(" \\\n");

        // Add output
        let output_name = if self.crate_types.contains(&"bin".to_string()) {
            format!("$out/bin/{}", self.pname)
        } else if self.crate_types.contains(&"proc-macro".to_string()) {
            // Proc-macros are shared libraries
            format!("$out/lib/lib{}.so", self.pname.replace('-', "_"))
        } else {
            format!("$out/lib/lib{}.rlib", self.pname.replace('-', "_"))
        };
        script.push_str("  -o ");
        script.push_str(&output_name);
        script.push_str(" \\\n");

        // Add build script flags (expands to flags read from build script output)
        script.push_str("  $BUILD_SCRIPT_FLAGS");

        script
    }
}

/// Configuration for the Nix code generator.
#[derive(Debug, Clone)]
pub struct NixGenConfig {
    /// The workspace root path (for source remapping).
    pub workspace_root: String,

    /// Whether to include content-addressed derivation attributes.
    pub content_addressed: bool,

    /// Whether cross-compilation is enabled.
    /// When true, proc-macros and build scripts use `hostRustToolchain`.
    pub cross_compiling: bool,

    /// The target platform triple (for regular crates).
    pub target_platform: Option<String>,

    /// The host platform triple (for proc-macros and build scripts).
    pub host_platform: Option<String>,
}

impl Default for NixGenConfig {
    fn default() -> Self {
        Self {
            workspace_root: String::new(),
            content_addressed: false,
            cross_compiling: false,
            target_platform: None,
            host_platform: None,
        }
    }
}

impl NixGenConfig {
    /// Creates a config for cross-compilation.
    pub fn with_cross_compilation(mut self, host: &str, target: &str) -> Self {
        self.cross_compiling = true;
        self.host_platform = Some(host.to_string());
        self.target_platform = Some(target.to_string());
        self
    }

    /// Returns the toolchain variable name for a given unit.
    ///
    /// - `"hostRustToolchain"` for proc-macros and build scripts when cross-compiling
    /// - `"rustToolchain"` otherwise
    pub fn toolchain_var_for_unit(&self, unit: &Unit) -> &'static str {
        if self.cross_compiling && crate::proc_macro::requires_host_toolchain(unit) {
            "hostRustToolchain"
        } else {
            "rustToolchain"
        }
    }
}

/// Generates Nix code from a unit graph.
pub struct NixGenerator {
    config: NixGenConfig,
}

impl NixGenerator {
    /// Creates a new generator with the given configuration.
    pub fn new(config: NixGenConfig) -> Self {
        Self { config }
    }

    /// Generates a complete Nix expression for the unit graph.
    pub fn generate(&self, graph: &UnitGraph) -> String {
        let mut out = String::new();

        // Header
        out.push_str("# Generated by nix-cargo-unit\n");
        out.push_str("# Do not edit manually\n\n");

        // Function signature
        // Include hostRustToolchain parameter for cross-compilation support
        if self.config.cross_compiling {
            out.push_str("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src }:\n\n");
        } else {
            out.push_str("{ pkgs, rustToolchain, src }:\n\n");
        }

        // Let block
        out.push_str("let\n");

        // Helper function for creating unit derivations
        out.push_str("  mkUnit = attrs: pkgs.stdenv.mkDerivation (attrs // {\n");
        out.push_str("    dontUnpack = true;\n");
        out.push_str("    dontConfigure = true;\n");
        out.push_str("  });\n\n");

        // Pre-compute derivation names for all units (needed for dependency resolution)
        let drv_names: Vec<String> = graph.units.iter().map(|u| u.derivation_name()).collect();

        // First pass: identify build script units and generate their compile/run derivations
        // Build a map from unit index -> BuildScriptRef for units that depend on build scripts
        let mut build_script_derivations: Vec<String> = Vec::new();
        let mut build_script_refs: std::collections::HashMap<usize, BuildScriptRef> =
            std::collections::HashMap::new();

        for (i, unit) in graph.units.iter().enumerate() {
            if unit.mode == "run-custom-build" {
                // This is a build script execution unit
                let info = BuildScriptInfo::from_unit(
                    unit,
                    &self.config.workspace_root,
                    self.config.content_addressed,
                );
                if let Some(info) = info {
                    // Generate compile derivation
                    build_script_derivations.push(format!(
                        "    \"{}\" = mkUnit {};\n",
                        info.compile_drv_name,
                        info.compile_derivation()
                    ));

                    // Generate run derivation (depends on compile derivation)
                    let compile_var = format!("units.\"{}\"", info.compile_drv_name);
                    build_script_derivations.push(format!(
                        "    \"{}\" = mkUnit {};\n",
                        info.run_drv_name,
                        info.run_derivation(&compile_var)
                    ));

                    // Store the reference for units that depend on this build script
                    build_script_refs.insert(
                        i,
                        BuildScriptRef {
                            run_drv_var: format!("units.\"{}\"", info.run_drv_name),
                            compile_drv_name: info.compile_drv_name,
                            run_drv_name: info.run_drv_name,
                        },
                    );
                }
            }
        }

        // Generate derivations for each unit
        out.push_str("  units = {\n");

        // First, output all build script derivations
        for drv_str in &build_script_derivations {
            out.push_str(drv_str);
            out.push('\n');
        }

        for (i, unit) in graph.units.iter().enumerate() {
            // Skip build script run units - they're already generated above
            if unit.mode == "run-custom-build" {
                continue;
            }

            let toolchain_var = self.config.toolchain_var_for_unit(unit);
            let mut drv = UnitDerivation::from_unit(
                unit,
                &self.config.workspace_root,
                self.config.content_addressed,
                toolchain_var,
            );

            // Wire up dependencies, and detect if any dependency is a build script
            for dep in &unit.dependencies {
                if let Some(dep_unit) = graph.units.get(dep.index) {
                    // Check if this dependency is a build script execution unit
                    if dep_unit.mode == "run-custom-build" {
                        // This unit depends on a build script - wire up the build script outputs
                        if let Some(bs_ref) = build_script_refs.get(&dep.index) {
                            drv.set_build_script_ref(bs_ref.clone());
                        }
                        // Don't add build script as a regular extern dependency
                        continue;
                    }

                    let dep_drv_name = &drv_names[dep.index];
                    drv.add_dep(DepRef {
                        nix_var: format!("units.\"{}\"", dep_drv_name),
                        extern_crate_name: dep.extern_crate_name.clone(),
                        derivation_name: dep_drv_name.clone(),
                        is_proc_macro: dep_unit.is_proc_macro(),
                    });
                }
            }

            let drv_name = &drv.name;

            out.push_str(&format!("    \"{}\" = mkUnit ", drv_name));
            out.push_str(&drv.to_nix());
            out.push_str(";\n\n");

            // Also add an alias by index for dependency resolution
            out.push_str(&format!(
                "    \"_idx_{}\" = units.\"{}\"; # index alias\n\n",
                i, drv_name
            ));
        }

        out.push_str("  };\n\n");

        // Root outputs
        out.push_str("in {\n");
        out.push_str("  inherit units;\n");

        // Root units
        let root_refs: Vec<String> = graph
            .roots
            .iter()
            .filter_map(|&i| graph.units.get(i))
            .map(|u| format!("units.\"{}\"", u.derivation_name()))
            .collect();

        out.push_str(&format!("  roots = [ {} ];\n", root_refs.join(" ")));

        // Convenience: default is the first root
        if let Some(&first_root) = graph.roots.first() {
            if let Some(unit) = graph.units.get(first_root) {
                out.push_str(&format!(
                    "  default = units.\"{}\";\n",
                    unit.derivation_name()
                ));
            }
        }

        out.push_str("}\n");

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_graph::UnitGraph;

    fn parse_unit_graph(json: &str) -> UnitGraph {
        serde_json::from_str(json).expect("failed to parse unit graph")
    }

    #[test]
    fn test_escape_nix_string() {
        assert_eq!(escape_nix_string("hello"), "hello");
        assert_eq!(escape_nix_string("hello\"world"), "hello\\\"world");
        assert_eq!(escape_nix_string("path\\to"), "path\\\\to");
        assert_eq!(escape_nix_string("${var}"), "\\${var}");
        assert_eq!(escape_nix_string("line\nbreak"), "line\\nbreak");
    }

    #[test]
    fn test_escape_nix_multiline() {
        assert_eq!(escape_nix_multiline("hello"), "hello");
        assert_eq!(escape_nix_multiline("end ''"), "end '''");
        assert_eq!(escape_nix_multiline("${var}"), "''${var}");
    }

    #[test]
    fn test_nix_string_escaping() {
        let s = NixString::new("hello \"world\"");
        assert_eq!(s.as_str(), "hello \\\"world\\\"");

        let raw = NixString::raw("pkgs.hello");
        assert_eq!(raw.as_str(), "pkgs.hello");
    }

    #[test]
    fn test_nix_attr_set() {
        let mut attrs = NixAttrSet::new();
        attrs.string("pname", "my-crate");
        attrs.string("version", "0.1.0");
        attrs.bool("dontUnpack", true);
        attrs.int("priority", 10);
        attrs.string_list("features", &["std".to_string(), "alloc".to_string()]);

        let rendered = attrs.render(0);

        assert!(rendered.contains("pname = \"my-crate\""));
        assert!(rendered.contains("version = \"0.1.0\""));
        assert!(rendered.contains("dontUnpack = true"));
        assert!(rendered.contains("priority = 10"));
        assert!(rendered.contains("features = [ \"std\" \"alloc\" ]"));
    }

    #[test]
    fn test_unit_derivation_from_unit() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "my-crate 0.1.0 (path+file:///workspace/crates/my-crate)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "my_crate",
                    "src_path": "/workspace/crates/my-crate/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["default", "std"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        let drv = UnitDerivation::from_unit(unit, "/workspace", false, "rustToolchain");

        assert_eq!(drv.pname, "my_crate");
        assert_eq!(drv.version, "0.1.0");
        assert_eq!(drv.edition, "2021");
        assert_eq!(drv.features, vec!["default", "std"]);
        assert!(drv.src_path.contains("${src}"));
        assert_eq!(drv.toolchain_var, "rustToolchain");
    }

    #[test]
    fn test_nix_generator_simple() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///workspace)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "test",
                    "src_path": "/workspace/src/lib.rs",
                    "edition": "2024"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Check structure
        assert!(nix.contains("{ pkgs, rustToolchain, src }:"));
        assert!(nix.contains("mkUnit = attrs:"));
        assert!(nix.contains("units = {"));
        assert!(nix.contains("roots = ["));
        assert!(nix.contains("default ="));

        // Check derivation content
        assert!(nix.contains("pname = \"test\""));
        assert!(nix.contains("version = \"0.1.0\""));
        assert!(nix.contains("--edition"));
        assert!(nix.contains("2024"));
    }

    #[test]
    fn test_nix_generator_with_deps() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "dep 0.1.0 (path+file:///workspace/dep)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "dep",
                        "src_path": "/workspace/dep/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": []
                },
                {
                    "pkg_id": "app 0.1.0 (path+file:///workspace/app)",
                    "target": {
                        "kind": ["bin"],
                        "crate_types": ["bin"],
                        "name": "app",
                        "src_path": "/workspace/app/src/main.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "dep", "public": false}
                    ]
                }
            ],
            "roots": [1]
        }"#;

        let graph = parse_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Should have both units
        assert!(nix.contains("pname = \"dep\""));
        assert!(nix.contains("pname = \"app\""));

        // Should have bin output path
        assert!(nix.contains("$out/bin/app"));

        // Should have --extern flag for dependency
        assert!(nix.contains("--extern"));
        assert!(nix.contains("dep="));
        assert!(nix.contains("/lib/libdep.rlib"));

        // Should have -L flag for library search path
        assert!(nix.contains("-L"));
        assert!(nix.contains("dependency="));
    }

    #[test]
    fn test_extern_crate_wiring() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "serde 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "serde",
                        "src_path": "/registry/serde/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": ["default", "std"],
                    "mode": "build",
                    "dependencies": []
                },
                {
                    "pkg_id": "serde_derive 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)",
                    "target": {
                        "kind": ["proc-macro"],
                        "crate_types": ["proc-macro"],
                        "name": "serde_derive",
                        "src_path": "/registry/serde_derive/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [],
                    "platform": "aarch64-apple-darwin"
                },
                {
                    "pkg_id": "my_app 0.1.0 (path+file:///workspace)",
                    "target": {
                        "kind": ["bin"],
                        "crate_types": ["bin"],
                        "name": "my_app",
                        "src_path": "/workspace/src/main.rs",
                        "edition": "2024"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "serde", "public": false},
                        {"index": 1, "extern_crate_name": "serde_derive", "public": false}
                    ]
                }
            ],
            "roots": [2]
        }"#;

        let graph = parse_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Should have all three units
        assert!(nix.contains("pname = \"serde\""));
        assert!(nix.contains("pname = \"serde_derive\""));
        assert!(nix.contains("pname = \"my_app\""));

        // my_app should have buildInputs with both dependencies
        assert!(nix.contains("buildInputs = ["));

        // Should have --extern flags for both dependencies
        assert!(nix.contains("serde="));
        assert!(nix.contains("serde_derive="));

        // Regular lib dep should use .rlib
        assert!(nix.contains("libserde.rlib"));

        // Proc-macro dep should use find for dynamic lib
        assert!(nix.contains("find") && nix.contains("serde_derive"));
    }

    #[test]
    fn test_dep_ref_in_build_inputs() {
        let mut drv = UnitDerivation {
            name: "test-0.1.0-abc123".to_string(),
            pname: "test".to_string(),
            version: "0.1.0".to_string(),
            edition: "2024".to_string(),
            crate_types: vec!["lib".to_string()],
            src_path: "${src}/src/lib.rs".to_string(),
            features: vec![],
            opt_level: "0".to_string(),
            is_test: false,
            is_proc_macro: false,
            deps: vec![],
            build_script_ref: None,
            rustc_flags: RustcFlags::new(),
            content_addressed: false,
            toolchain_var: "rustToolchain".to_string(),
        };

        // Add a dependency
        drv.add_dep(DepRef {
            nix_var: "units.\"dep-0.1.0-xyz789\"".to_string(),
            extern_crate_name: "dep".to_string(),
            derivation_name: "dep-0.1.0-xyz789".to_string(),
            is_proc_macro: false,
        });

        let nix = drv.to_nix();

        // Should have the dependency in buildInputs
        assert!(nix.contains("buildInputs = [ units.\"dep-0.1.0-xyz789\" ]"));
    }

    #[test]
    fn test_multiline_build_phase() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///workspace)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "test",
                    "src_path": "/workspace/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {"name": "release", "opt_level": "3", "lto": "thin"},
                "features": ["std", "derive"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        let drv = UnitDerivation::from_unit(unit, "/workspace", false, "rustToolchain");
        let build_phase = drv.generate_build_phase();

        // Check for proper flag formatting
        assert!(build_phase.contains("--crate-name"));
        assert!(build_phase.contains("test"));
        assert!(build_phase.contains("--edition"));
        assert!(build_phase.contains("2021"));
        assert!(build_phase.contains("opt-level=3"));
        assert!(build_phase.contains("lto=thin"));
        assert!(
            build_phase.contains("feature=\\\"std\\\"") || build_phase.contains("feature=\"std\"")
        );
    }

    #[test]
    fn test_content_addressed_derivation() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///workspace)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "test",
                    "src_path": "/workspace/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        // Without content-addressed
        let drv = UnitDerivation::from_unit(unit, "/workspace", false, "rustToolchain");
        let nix = drv.to_nix();
        assert!(!nix.contains("__contentAddressed"));
        assert!(!nix.contains("outputHashMode"));
        assert!(!nix.contains("outputHashAlgo"));

        // With content-addressed
        let drv_ca = UnitDerivation::from_unit(unit, "/workspace", true, "rustToolchain");
        let nix_ca = drv_ca.to_nix();
        assert!(nix_ca.contains("__contentAddressed = true"));
        assert!(nix_ca.contains("outputHashMode = \"recursive\""));
        assert!(nix_ca.contains("outputHashAlgo = \"sha256\""));
    }

    #[test]
    fn test_nix_generator_content_addressed() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///workspace)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "test",
                    "src_path": "/workspace/src/lib.rs",
                    "edition": "2024"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);

        // Without CA
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };
        let nix = NixGenerator::new(config).generate(&graph);
        assert!(!nix.contains("__contentAddressed"));

        // With CA
        let config_ca = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: true,
            ..Default::default()
        };
        let nix_ca = NixGenerator::new(config_ca).generate(&graph);
        assert!(nix_ca.contains("__contentAddressed = true"));
        assert!(nix_ca.contains("outputHashMode = \"recursive\""));
        assert!(nix_ca.contains("outputHashAlgo = \"sha256\""));
    }

    #[test]
    fn test_build_script_output_wiring() {
        // Test a unit graph where a library depends on a build script
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-crate 0.1.0 (path+file:///workspace)",
                    "target": {
                        "kind": ["custom-build"],
                        "crate_types": ["bin"],
                        "name": "build-script-build",
                        "src_path": "/workspace/build.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": ["feature-x"],
                    "mode": "run-custom-build",
                    "dependencies": []
                },
                {
                    "pkg_id": "my-crate 0.1.0 (path+file:///workspace)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "my_crate",
                        "src_path": "/workspace/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": ["feature-x"],
                    "mode": "build",
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "build_script_build", "public": false}
                    ]
                }
            ],
            "roots": [1]
        }"#;

        let graph = parse_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Should have build script compile derivation
        assert!(nix.contains("my-crate-build-script-"));
        assert!(nix.contains("pname = \"my-crate-build-script\""));

        // Should have build script run derivation
        assert!(nix.contains("my-crate-build-script-run-"));
        assert!(nix.contains("pname = \"my-crate-build-script-output\""));

        // The library should read build script outputs
        assert!(nix.contains("BUILD_SCRIPT_FLAGS"));
        assert!(nix.contains("# Read build script outputs"));
        assert!(nix.contains("rustc-cfg"));

        // Library build phase should include $BUILD_SCRIPT_FLAGS
        assert!(nix.contains("$BUILD_SCRIPT_FLAGS"));

        // Library should have build script run derivation in buildInputs
        assert!(nix.contains("my-crate-build-script-run-"));
    }

    #[test]
    fn test_build_script_ref_in_build_inputs() {
        let mut drv = UnitDerivation {
            name: "test-0.1.0-abc123".to_string(),
            pname: "test".to_string(),
            version: "0.1.0".to_string(),
            edition: "2024".to_string(),
            crate_types: vec!["lib".to_string()],
            src_path: "${src}/src/lib.rs".to_string(),
            features: vec![],
            opt_level: "0".to_string(),
            is_test: false,
            is_proc_macro: false,
            deps: vec![],
            build_script_ref: Some(BuildScriptRef {
                run_drv_var: "units.\"my-build-script-run\"".to_string(),
                compile_drv_name: "my-build-script".to_string(),
                run_drv_name: "my-build-script-run".to_string(),
            }),
            rustc_flags: RustcFlags::new(),
            content_addressed: false,
            toolchain_var: "rustToolchain".to_string(),
        };

        // Add a regular dependency too
        drv.add_dep(DepRef {
            nix_var: "units.\"dep-0.1.0-xyz789\"".to_string(),
            extern_crate_name: "dep".to_string(),
            derivation_name: "dep-0.1.0-xyz789".to_string(),
            is_proc_macro: false,
        });

        let nix = drv.to_nix();

        // Should have both regular dep and build script in buildInputs
        assert!(nix.contains("buildInputs = ["));
        assert!(nix.contains("units.\"dep-0.1.0-xyz789\""));
        assert!(nix.contains("units.\"my-build-script-run\""));

        // Build phase should read build script outputs
        let build_phase = drv.generate_build_phase();
        assert!(build_phase.contains("BUILD_SCRIPT_FLAGS"));
        assert!(build_phase.contains("units.\"my-build-script-run\""));
        assert!(build_phase.contains("rustc-cfg"));
    }

    #[test]
    fn test_proc_macro_host_toolchain() {
        // Test that proc-macros use hostRustToolchain in cross-compilation
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "serde_derive 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)",
                    "target": {
                        "kind": ["proc-macro"],
                        "crate_types": ["proc-macro"],
                        "name": "serde_derive",
                        "src_path": "/registry/serde_derive/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [],
                    "platform": "aarch64-apple-darwin"
                },
                {
                    "pkg_id": "my_app 0.1.0 (path+file:///workspace)",
                    "target": {
                        "kind": ["bin"],
                        "crate_types": ["bin"],
                        "name": "my_app",
                        "src_path": "/workspace/src/main.rs",
                        "edition": "2024"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "serde_derive", "public": false}
                    ]
                }
            ],
            "roots": [1]
        }"#;

        let graph = parse_unit_graph(json);

        // Without cross-compilation: both use rustToolchain
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            cross_compiling: false,
            ..Default::default()
        };
        let nix = NixGenerator::new(config).generate(&graph);

        // Should use rustToolchain for both
        assert!(nix.contains("{ pkgs, rustToolchain, src }:"));
        assert!(!nix.contains("hostRustToolchain"));

        // With cross-compilation: proc-macro uses hostRustToolchain
        let config_cross = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            cross_compiling: true,
            host_platform: Some("aarch64-apple-darwin".to_string()),
            target_platform: Some("x86_64-unknown-linux-gnu".to_string()),
        };
        let nix_cross = NixGenerator::new(config_cross).generate(&graph);

        // Should have hostRustToolchain in function signature
        assert!(nix_cross.contains("hostRustToolchain"));
        assert!(
            nix_cross.contains("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src }:")
        );

        // Proc-macro should use hostRustToolchain
        // Regular bin should use rustToolchain
        // Check that both toolchains appear in nativeBuildInputs
        assert!(nix_cross.contains("nativeBuildInputs = [ hostRustToolchain ]"));
        assert!(nix_cross.contains("nativeBuildInputs = [ rustToolchain ]"));
    }

    #[test]
    fn test_proc_macro_output_path() {
        // Test that proc-macros output to shared library path
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my_macro 0.1.0 (path+file:///workspace)",
                    "target": {
                        "kind": ["proc-macro"],
                        "crate_types": ["proc-macro"],
                        "name": "my_macro",
                        "src_path": "/workspace/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [],
                    "platform": "x86_64-unknown-linux-gnu"
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        let drv = UnitDerivation::from_unit(unit, "/workspace", false, "rustToolchain");
        let build_phase = drv.generate_build_phase();

        // Should output to shared library path (.so)
        assert!(build_phase.contains("$out/lib/libmy_macro.so"));
        assert!(drv.is_proc_macro);
    }
}
