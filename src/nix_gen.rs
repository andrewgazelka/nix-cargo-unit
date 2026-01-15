//! Nix derivation code generation.
//!
//! This module provides structured builders for generating Nix expressions
//! from cargo unit graph data. It focuses on proper escaping, composability,
//! and producing readable output.

use std::fmt::Write as _;
use std::rc::Rc;

use crate::build_script::{BuildScriptInfo, BuildScriptOutput};

/// Parsed version components from a semver string.
#[derive(Debug, Clone)]
pub struct VersionParts<'a> {
    pub major: &'a str,
    pub minor: &'a str,
    pub patch: &'a str,
}

impl<'a> VersionParts<'a> {
    /// Parses version components from a version string like "1.2.3" or "1.2.3-alpha".
    pub fn parse(version: &'a str) -> Self {
        let parts: Vec<&str> = version.split('.').collect();
        let major = parts.first().copied().unwrap_or("0");
        let minor = parts.get(1).copied().unwrap_or("0");
        let patch_full = parts.get(2).copied().unwrap_or("0");
        // Strip any pre-release suffix from patch (e.g., "0-alpha" -> "0")
        let patch = patch_full.split('-').next().unwrap_or("0");
        Self {
            major,
            minor,
            patch,
        }
    }
}

/// Generates shell script exports for CARGO_PKG_* environment variables.
///
/// These are needed by crates that use `env!()` macros at compile time.
pub fn generate_cargo_pkg_exports(
    package_name: &str,
    version: &str,
    features: &[String],
) -> String {
    // Pre-allocate: ~500 bytes base + ~40 bytes per feature
    let mut script = String::with_capacity(500 + features.len() * 40);
    let vp = VersionParts::parse(version);

    script.push_str("# Cargo package environment variables for env!() macros\n");
    let _ = writeln!(script, "export CARGO_PKG_NAME=\"{package_name}\"");
    let _ = writeln!(script, "export CARGO_PKG_VERSION=\"{version}\"");
    let _ = writeln!(script, "export CARGO_PKG_VERSION_MAJOR=\"{}\"", vp.major);
    let _ = writeln!(script, "export CARGO_PKG_VERSION_MINOR=\"{}\"", vp.minor);
    let _ = writeln!(script, "export CARGO_PKG_VERSION_PATCH=\"{}\"", vp.patch);
    script.push_str("export CARGO_PKG_VERSION_PRE=\"\"\n");
    script.push_str("export CARGO_PKG_AUTHORS=\"\"\n");
    script.push_str("export CARGO_PKG_DESCRIPTION=\"\"\n");
    script.push_str("export CARGO_PKG_HOMEPAGE=\"\"\n");
    script.push_str("export CARGO_PKG_REPOSITORY=\"\"\n");
    script.push_str("export CARGO_PKG_LICENSE=\"\"\n");
    script.push_str("export CARGO_PKG_LICENSE_FILE=\"\"\n");
    script.push_str("export CARGO_PKG_RUST_VERSION=\"\"\n");
    script.push_str("export CARGO_PKG_README=\"\"\n");

    // Set feature flags as environment variables
    for feature in features {
        script.push_str("export CARGO_FEATURE_");
        for c in feature.chars() {
            if c == '-' {
                script.push('_');
            } else {
                script.push(c.to_ascii_uppercase());
            }
        }
        script.push_str("=1\n");
    }

    script
}
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

    /// Adds content-addressed derivation attributes.
    pub fn add_ca_attrs(&mut self) -> &mut Self {
        self.bool("__contentAddressed", true);
        self.string("outputHashMode", "recursive");
        self.string("outputHashAlgo", "sha256");
        // Skip fixup phase entirely for CA derivations:
        // 1. Rust crates don't need stripping/patching that fixup provides
        // 2. fixupPhase runs chmod which fails on read-only CA store paths
        self.bool("dontFixup", true);
        self
    }

    /// Adds an integer attribute.
    pub fn int(&mut self, key: &str, value: i64) -> &mut Self {
        self.attrs.push((key.to_string(), value.to_string()));
        self
    }

    /// Adds a list of strings.
    pub fn string_list(&mut self, key: &str, values: &[String]) -> &mut Self {
        // Build directly without intermediate Vec
        let mut result = String::with_capacity(values.len() * 20 + 4);
        result.push_str("[ ");
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                result.push(' ');
            }
            result.push('"');
            result.push_str(&escape_nix_string(v));
            result.push('"');
        }
        result.push_str(" ]");
        self.attrs.push((key.to_owned(), result));
        self
    }

    /// Adds a list of raw expressions.
    pub fn expr_list(&mut self, key: &str, values: &[String]) -> &mut Self {
        let mut result =
            String::with_capacity(values.iter().map(|s| s.len() + 1).sum::<usize>() + 4);
        result.push_str("[ ");
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                result.push(' ');
            }
            result.push_str(v);
        }
        result.push_str(" ]");
        self.attrs.push((key.to_owned(), result));
        self
    }

    /// Adds a multiline string (using ''...'').
    /// This escapes ${} to prevent accidental Nix interpolation.
    pub fn multiline(&mut self, key: &str, value: &str) -> &mut Self {
        self.attrs.push((
            key.to_string(),
            format!("''\n{}\n''", escape_nix_multiline(value)),
        ));
        self
    }

    /// Adds a raw multiline string - no escaping is done.
    /// Caller is responsible for proper Nix syntax:
    /// - Use ${...} for Nix interpolation
    /// - Use ''${...} for literal ${...} (bash variable expansion)
    /// - Use ''' for literal ''
    pub fn multiline_interpolated(&mut self, key: &str, value: &str) -> &mut Self {
        // No escaping - caller handles Nix syntax
        self.attrs
            .push((key.to_string(), format!("''\n{}\n''", value)));
        self
    }

    /// Renders the attribute set with the given indentation.
    pub fn render(&self, indent: usize) -> String {
        let base_indent = "  ".repeat(indent);
        let inner_indent = "  ".repeat(indent + 1);

        // Pre-allocate based on content size
        let estimated_size: usize = self.attrs.iter().map(|(k, v)| k.len() + v.len() + 10).sum();
        let mut out = String::with_capacity(estimated_size + 64);
        out.push_str("{\n");

        for (key, value) in &self.attrs {
            // Handle multiline values specially
            if value.starts_with("''") && value.contains('\n') {
                out.push_str(&inner_indent);
                out.push_str(key);
                out.push_str(" = ");
                // Iterate lines directly without collecting
                for (i, line) in value.lines().enumerate() {
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
    /// This is the alias name (e.g., `libc_errno` when `errno` is renamed).
    pub extern_crate_name: String,

    /// Library name (used for constructing path to .rlib file).
    /// This is the actual crate library name as it appears on disk (e.g., `errno`).
    pub lib_name: String,

    /// Identity hash of the dependency (used in -C extra-filename suffix).
    /// The library file is named `lib{lib_name}-{identity_hash}.rlib`.
    pub identity_hash: String,

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

    /// Dependencies with extern crate info (direct deps only - used for --extern).
    pub deps: Vec<DepRef>,

    /// Library search paths (transitive deps - used for -L dependency=).
    /// Tuple of (nix_var, lib_name) - e.g., ("units.\"foo-1.0.0-hash\"", "foo")
    pub lib_search_deps: Vec<(String, String)>,

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
    /// The `drv_name` and `identity_hash` should be pre-computed for efficiency.
    /// The `is_external_dep` flag indicates if this is a dependency (registry/git)
    /// vs a local workspace crate; external deps get `--cap-lints warn`.
    pub fn from_unit(
        unit: &Unit,
        workspace_root: &str,
        content_addressed: bool,
        toolchain_var: &str,
        drv_name: &str,
        identity_hash: &str,
        is_external_dep: bool,
    ) -> Self {
        let pname = unit.target.name.clone();
        let version = unit.package_version().unwrap_or("0.0.0").to_string();

        // Remap source path
        let src_path =
            crate::source_filter::remap_source_path(&unit.target.src_path, workspace_root, "src");

        let mut rustc_flags = RustcFlags::from_unit(unit);
        // Add metadata hash for stable crate identity across compilations
        rustc_flags.add_metadata(identity_hash);

        // Cap lints to warn for external dependencies (same as cargo does)
        // This prevents #[deny(dead_code)] etc from breaking dependency builds
        if is_external_dep {
            rustc_flags.cap_lints_for_dependency();
        }

        Self {
            name: drv_name.to_owned(),
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
            lib_search_deps: Vec::new(),
            build_script_ref: None,
            rustc_flags,
            content_addressed,
            toolchain_var: toolchain_var.to_owned(),
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

    /// Sets the library search dependencies (transitive deps for -L flags).
    pub fn set_lib_search_deps(&mut self, deps: Vec<(String, String)>) {
        self.lib_search_deps = deps;
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

        // Don't strip Rust libraries - it removes metadata required for compilation
        attrs.bool("dontStrip", true);

        // Content-addressed derivation attributes
        if self.content_addressed {
            attrs.add_ca_attrs();
        }

        // Build phase with rustc invocation
        // Use multiline_interpolated so ${...} gets interpolated by Nix
        let build_phase = self.generate_build_phase();
        attrs.multiline_interpolated("buildPhase", &build_phase);

        // Install phase - copy outputs from build directory to $out
        let install_phase = self.generate_install_phase();
        attrs.multiline("installPhase", &install_phase);

        attrs.render(2)
    }

    /// Generates the build phase script.
    fn generate_build_phase(&self) -> String {
        // Pre-allocate: ~1KB base + ~100 bytes per dep
        let mut script =
            String::with_capacity(1024 + (self.deps.len() + self.lib_search_deps.len()) * 100);

        // Create build directory (NOT $out - $out is read-only during buildPhase in Nix sandbox)
        // We'll copy outputs to $out in installPhase
        script.push_str("mkdir -p build\n");

        // Initialize build script flags variable
        script.push_str("BUILD_SCRIPT_FLAGS=\"\"\n\n");

        // Set CARGO_PKG_* environment variables that crates may use via env!() at compile time
        script.push_str(&generate_cargo_pkg_exports(
            &self.pname,
            &self.version,
            &self.features,
        ));
        script.push('\n');

        // Read build script outputs if this unit depends on a build script
        if let Some(ref bs_ref) = self.build_script_ref {
            script.push('\n');
            // Build shell_var directly without format!: "${units.\"name\"}"
            let mut shell_var = String::with_capacity(bs_ref.run_drv_var.len() + 3);
            shell_var.push_str("${");
            shell_var.push_str(&bs_ref.run_drv_var);
            shell_var.push('}');
            script.push_str(&BuildScriptOutput::generate_nix_flag_reader(&shell_var));
            script.push('\n');
        }

        // Set up proc-macro path variables with platform fallback (before rustc command)
        for dep in &self.deps {
            if dep.is_proc_macro {
                let var_name = format!(
                    "PROCMACRO_{}",
                    dep.lib_name.to_uppercase().replace('-', "_")
                );
                script.push_str(&var_name);
                script.push_str("=\"${");
                script.push_str(&dep.nix_var);
                script.push_str("}/lib/lib");
                script.push_str(&dep.lib_name);
                script.push('-');
                script.push_str(&dep.identity_hash);
                script.push_str(".dylib\"\n");
                script.push_str("[ -f \"$");
                script.push_str(&var_name);
                script.push_str("\" ] || ");
                script.push_str(&var_name);
                script.push_str("=\"${");
                script.push_str(&dep.nix_var);
                script.push_str("}/lib/lib");
                script.push_str(&dep.lib_name);
                script.push('-');
                script.push_str(&dep.identity_hash);
                script.push_str(".so\"\n");
                // Debug: print the variable value
                script.push_str("echo \"DEBUG: ");
                script.push_str(&var_name);
                script.push_str(" = $");
                script.push_str(&var_name);
                script.push_str("\" && ls -la \"$");
                script.push_str(&var_name);
                script.push_str("\" || echo \"File not found: $");
                script.push_str(&var_name);
                script.push_str("\"\n");
            }
        }

        // Debug: enable command tracing to see the actual rustc command
        script.push_str("set -x\n");
        script.push_str("rustc \\\n");

        // Add each flag on its own line for readability
        for arg in self.rustc_flags.args() {
            script.push_str("  ");
            script.push_str(&crate::shell::quote_arg(arg));
            script.push_str(" \\\n");
        }

        // Add -L library search paths for ALL dependencies (direct and transitive).
        // This is required because when rustc loads a dependency's rlib (e.g., http),
        // it needs to resolve THAT crate's dependencies (e.g., bytes) via -L search paths.
        //
        // Add -L for direct deps first (avoid format! - write directly)
        for dep in &self.deps {
            script.push_str("  -L dependency=${");
            script.push_str(&dep.nix_var);
            script.push_str("}/lib \\\n");
        }
        // Add -L for transitive deps (lib_search_deps)
        for (lib_dep, _lib_name) in &self.lib_search_deps {
            script.push_str("  -L dependency=${");
            script.push_str(lib_dep);
            script.push_str("}/lib \\\n");
        }

        // Proc-macro crates need --extern proc_macro (compiler-provided crate)
        if self.is_proc_macro {
            script.push_str("  --extern proc_macro \\\n");
        }

        // Add --extern flags for each dependency
        // Note: extern_crate_name is the alias (used in --extern name=), while
        // lib_name is the actual library filename on disk (used in path to .rlib)
        for dep in &self.deps {
            script.push_str("  --extern ");
            if dep.is_proc_macro {
                // Proc-macros use the variable set above
                script.push_str(&dep.extern_crate_name);
                script.push_str("=\"$PROCMACRO_");
                script.push_str(&dep.lib_name.to_uppercase().replace('-', "_"));
                script.push('"');
            } else {
                // Regular dependencies use .rlib
                script.push_str(&dep.extern_crate_name);
                script.push_str("=${");
                script.push_str(&dep.nix_var);
                script.push_str("}/lib/lib");
                script.push_str(&dep.lib_name);
                script.push('-');
                script.push_str(&dep.identity_hash);
                script.push_str(".rlib");
            }
            script.push_str(" \\\n");
        }

        // Add source file
        script.push_str("  ");
        script.push_str(&self.src_path);
        script.push_str(" \\\n");

        // Add output options
        if self.crate_types.iter().any(|t| t == "bin") {
            // Binaries use -o for direct output
            script.push_str("  -o build/");
            script.push_str(&self.pname);
            script.push_str(" \\\n");
        } else {
            // Libraries use --out-dir to produce output files
            script.push_str("  --out-dir build \\\n");
            script.push_str("  --emit=dep-info,link \\\n");
        }

        // Add build script flags (expands to flags read from build script output)
        script.push_str("  $BUILD_SCRIPT_FLAGS");

        script
    }

    /// Generates the install phase script.
    fn generate_install_phase(&self) -> String {
        let mut script = String::with_capacity(200);

        if self.crate_types.iter().any(|t| t == "bin") {
            // Skip entirely if binary exists (CA-derivation reuse)
            script.push_str("[ -f \"$out/bin/");
            script.push_str(&self.pname);
            script.push_str("\" ] || {\n  mkdir -p $out/bin\n  cp build/");
            script.push_str(&self.pname);
            script.push_str(" $out/bin/\n  chmod 755 $out/bin/");
            script.push_str(&self.pname);
            script.push_str("\n}");
        } else {
            // For libraries and proc-macros, copy all outputs from --out-dir
            // This includes .rlib, .rmeta, .d files, and .dylib/.so for proc-macros
            // Skip entirely if $out/lib exists (CA-derivation reuse)
            // For proc-macro dylibs on macOS, fix the install name so rustc can load them
            // Dylibs need execute permission (755) to be dlopen'd
            script.push_str(
                r#"[ -d "$out/lib" ] || {
  mkdir -p $out/lib
  cp build/* $out/lib/
  # Set permissions: 755 for shared libs (dylib/so), 644 for others
  for f in $out/lib/*; do
    case "$f" in
      *.dylib|*.so) chmod 755 "$f" ;;
      *) chmod 644 "$f" ;;
    esac
  done
  # Fix install_name for macOS dylibs (proc-macros) so they can be loaded from $out/lib
  for dylib in $out/lib/*.dylib; do
    [ -f "$dylib" ] && install_name_tool -id "$dylib" "$dylib" 2>/dev/null || true
  done
}"#,
            );
        }

        script
    }
}

/// Configuration for the Nix code generator.
#[derive(Debug, Clone, Default)]
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
        // Always include hostRustToolchain with default for compatibility with lib.nix
        // extraNativeBuildInputs allows passing protobuf, cmake, etc. for build scripts
        // vendorDir allows passing pre-vendored crate sources for registry deps
        out.push_str("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src, extraNativeBuildInputs ? [], vendorDir ? null }:\n\n");

        // Let block
        out.push_str("let\n");

        // Helper function for creating unit derivations
        out.push_str("  mkUnit = attrs: pkgs.stdenv.mkDerivation (attrs // {\n");
        out.push_str("    dontUnpack = true;\n");
        out.push_str("    dontConfigure = true;\n");
        out.push_str("  });\n\n");

        // Pre-compute identity hashes and derivation names for all units (needed for dependency resolution)
        // Computing these upfront avoids redundant SHA-256 computations
        let identity_hashes: Vec<String> = graph.units.iter().map(|u| u.identity_hash()).collect();
        let drv_names: Vec<String> = graph
            .units
            .iter()
            .zip(&identity_hashes)
            .map(|(u, hash)| {
                let name = &u.target.name;
                let version = u.package_version().unwrap_or("0.0.0");
                format!("{name}-{version}-{hash}")
            })
            .collect();

        // Compute transitive dependencies for each unit
        // This is needed for -L library search paths (rustc needs to find all transitive rlibs)
        // Uses Rc<FxHashSet> to avoid O(n²) cloning - computed sets are shared via Rc
        let transitive_deps: Vec<Rc<rustc_hash::FxHashSet<usize>>> = {
            type FxSet = rustc_hash::FxHashSet<usize>;

            // Build direct dependency map (unit index -> Vec of dep indices)
            let direct_deps: Vec<Vec<usize>> = graph
                .units
                .iter()
                .map(|unit| {
                    unit.dependencies
                        .iter()
                        .filter_map(|d| {
                            // Skip build script run units for transitive deps
                            graph
                                .units
                                .get(d.index)
                                .filter(|dep_unit| dep_unit.mode != "run-custom-build")
                                .map(|_| d.index)
                        })
                        .collect()
                })
                .collect();

            // Compute transitive closure for each unit using DFS with Rc sharing
            fn transitive_closure(
                unit_idx: usize,
                direct_deps: &[Vec<usize>],
                cache: &mut [Option<Rc<FxSet>>],
            ) -> Rc<FxSet> {
                if let Some(cached) = &cache[unit_idx] {
                    return Rc::clone(cached); // Cheap Rc clone, not set clone
                }

                // Pre-size based on direct deps (heuristic)
                let mut result = FxSet::with_capacity_and_hasher(
                    direct_deps[unit_idx].len() * 4,
                    Default::default(),
                );
                for &dep_idx in &direct_deps[unit_idx] {
                    result.insert(dep_idx);
                    // Recursively add transitive deps
                    let trans = transitive_closure(dep_idx, direct_deps, cache);
                    result.extend(trans.iter().copied());
                }
                let rc = Rc::new(result);
                cache[unit_idx] = Some(Rc::clone(&rc));
                rc
            }

            let mut cache: Vec<Option<Rc<FxSet>>> = vec![None; graph.units.len()];
            (0..graph.units.len())
                .map(|i| transitive_closure(i, &direct_deps, &mut cache))
                .collect()
        };

        // First pass: identify build script RUN units and their corresponding COMPILE units
        // Build a map from run unit index -> BuildScriptRef for units that depend on build scripts
        //
        // Build scripts appear as two units in the graph:
        // 1. COMPILE unit: mode="build", kind=["custom-build"] - compiles build.rs with its deps
        // 2. RUN unit: mode="run-custom-build" - executes the compiled binary
        //
        // The RUN unit depends on the COMPILE unit. We process COMPILE units as normal
        // derivations (to get their dependencies like tonic-build), and generate special
        // RUN derivations that execute the binary and capture cargo: directives.
        let mut build_script_run_derivations: Vec<String> = Vec::new();
        let mut build_script_refs: rustc_hash::FxHashMap<usize, BuildScriptRef> =
            rustc_hash::FxHashMap::default();

        // First pass: identify all build script RUN units and their info
        // We need this map to wire up DEP_* variables between build scripts
        struct BuildScriptRunInfo {
            unit_index: usize,
            package_name: String,
            compile_dep_index: usize,
            info: BuildScriptInfo,
        }
        let mut build_script_runs: Vec<BuildScriptRunInfo> = Vec::new();
        let mut package_to_bs_run: rustc_hash::FxHashMap<String, usize> =
            rustc_hash::FxHashMap::default();

        for (i, unit) in graph.units.iter().enumerate() {
            if unit.mode == "run-custom-build" {
                // This is a build script RUN unit - find its compile unit dependency
                let compile_dep = unit.dependencies.iter().find(|dep| {
                    graph.units.get(dep.index).is_some_and(|u| {
                        u.mode == "build" && u.target.kind.contains(&"custom-build".to_string())
                    })
                });

                if let Some(compile_dep) = compile_dep {
                    let info = BuildScriptInfo::from_unit(
                        unit,
                        &self.config.workspace_root,
                        self.config.content_addressed,
                    );
                    if let Some(info) = info {
                        let package_name = unit.package_name().to_string();
                        package_to_bs_run.insert(package_name.clone(), build_script_runs.len());
                        build_script_runs.push(BuildScriptRunInfo {
                            unit_index: i,
                            package_name,
                            compile_dep_index: compile_dep.index,
                            info,
                        });
                    }
                }
            }
        }

        // Second pass: for each build script RUN, find which other build scripts' outputs
        // it should receive DEP_* variables from (based on library dependencies)
        for bs_run in &build_script_runs {
            let compile_drv_name = drv_names[bs_run.compile_dep_index].clone();
            let compile_var = format!("units.\"{}\"", compile_drv_name);

            // Find dependency build script outputs:
            // Look at the library unit for this package and collect build script outputs
            // from its dependencies
            let mut dep_bs_outputs: Vec<String> = Vec::new();

            // Find the library unit for this package (same pkg_id, mode="build", kind contains "lib")
            let unit = &graph.units[bs_run.unit_index];
            let lib_unit_idx = graph.units.iter().enumerate().find(|(_, u)| {
                u.pkg_id == unit.pkg_id
                    && u.mode == "build"
                    && (u.target.kind.contains(&"lib".to_string())
                        || u.target.kind.contains(&"rlib".to_string()))
            });

            if let Some((_, lib_unit)) = lib_unit_idx {
                // For each dependency of the library unit, check if it has a build script
                for dep in &lib_unit.dependencies {
                    if let Some(dep_unit) = graph.units.get(dep.index) {
                        // If this dependency is a build script RUN, add it
                        // Skip the current package's own build script to avoid self-reference
                        if dep_unit.mode == "run-custom-build"
                            && dep_unit.package_name() != bs_run.package_name
                        {
                            if let Some(other_bs_run_idx) =
                                package_to_bs_run.get(dep_unit.package_name())
                            {
                                let other_bs = &build_script_runs[*other_bs_run_idx];
                                dep_bs_outputs
                                    .push(format!("units.\"{}\"", other_bs.info.run_drv_name));
                            }
                        }
                        // Also check if the dependency's package has a build script
                        // (in case it's a lib unit that depends on another lib)
                        // Skip the current package's own build script to avoid self-reference
                        let dep_pkg_name = dep_unit.package_name();
                        if dep_pkg_name != bs_run.package_name {
                            if let Some(other_bs_run_idx) = package_to_bs_run.get(dep_pkg_name) {
                                let other_bs = &build_script_runs[*other_bs_run_idx];
                                let run_var = format!("units.\"{}\"", other_bs.info.run_drv_name);
                                if !dep_bs_outputs.contains(&run_var) {
                                    dep_bs_outputs.push(run_var);
                                }
                            }
                        }
                    }
                }
            }

            // Generate run derivation with dependency build script outputs
            build_script_run_derivations.push(format!(
                "    \"{}\" = mkUnit {};\n",
                bs_run.info.run_drv_name,
                bs_run.info.run_derivation(&compile_var, &dep_bs_outputs)
            ));

            // Store the reference for units that depend on this build script
            build_script_refs.insert(
                bs_run.unit_index,
                BuildScriptRef {
                    run_drv_var: format!("units.\"{}\"", bs_run.info.run_drv_name),
                    compile_drv_name,
                    run_drv_name: bs_run.info.run_drv_name.clone(),
                },
            );
        }

        // Generate derivations for each unit
        out.push_str("  units = {\n");

        // First, output all build script RUN derivations
        // (COMPILE derivations are generated as normal units in the main loop)
        for drv_str in &build_script_run_derivations {
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
                &drv_names[i],
                &identity_hashes[i],
                unit.is_external_dependency(),
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
                    // Get the actual library name from the dependency unit's target
                    // This is the filename used for the .rlib (may differ from extern_crate_name if renamed)
                    let lib_name = dep_unit.target.name.replace('-', "_");
                    drv.add_dep(DepRef {
                        nix_var: format!("units.\"{}\"", dep_drv_name),
                        extern_crate_name: dep.extern_crate_name.clone(),
                        lib_name,
                        identity_hash: identity_hashes[dep.index].clone(),
                        derivation_name: dep_drv_name.clone(),
                        is_proc_macro: dep_unit.is_proc_macro(),
                    });
                }
            }

            // Set lib search deps (transitive closure for -L flags)
            // Include (nix_var, lib_name) so we can filter out direct deps by name
            let lib_deps: Vec<(String, String)> = transitive_deps[i]
                .iter()
                .filter_map(|&idx| {
                    let dep_unit = graph.units.get(idx)?;
                    let nix_var = format!("units.\"{}\"", drv_names[idx]);
                    let lib_name = dep_unit.target.name.replace('-', "_");
                    Some((nix_var, lib_name))
                })
                .collect();
            drv.set_lib_search_deps(lib_deps);

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

        // Packages attrset - maps package target name to derivation for workspace support
        // This allows accessing individual workspace members by name
        out.push_str("\n  # Workspace packages by target name\n");
        out.push_str("  packages = {\n");
        for &root_idx in &graph.roots {
            if let Some(unit) = graph.units.get(root_idx) {
                let target_name = &unit.target.name;
                let drv_name = unit.derivation_name();
                out.push_str(&format!(
                    "    \"{}\" = units.\"{}\";\n",
                    escape_nix_string(target_name),
                    drv_name
                ));
            }
        }
        out.push_str("  };\n");

        // Binaries attrset - only binary targets for convenient access
        out.push_str("\n  # Binary targets only\n");
        out.push_str("  binaries = {\n");
        for &root_idx in &graph.roots {
            if let Some(unit) = graph.units.get(root_idx)
                && unit.is_bin()
            {
                let target_name = &unit.target.name;
                let drv_name = unit.derivation_name();
                out.push_str(&format!(
                    "    \"{}\" = units.\"{}\";\n",
                    escape_nix_string(target_name),
                    drv_name
                ));
            }
        }
        out.push_str("  };\n");

        // Libraries attrset - only library targets
        out.push_str("\n  # Library targets only\n");
        out.push_str("  libraries = {\n");
        for &root_idx in &graph.roots {
            if let Some(unit) = graph.units.get(root_idx)
                && (unit.is_lib() || unit.is_proc_macro())
            {
                let target_name = &unit.target.name;
                let drv_name = unit.derivation_name();
                out.push_str(&format!(
                    "    \"{}\" = units.\"{}\";\n",
                    escape_nix_string(target_name),
                    drv_name
                ));
            }
        }
        out.push_str("  };\n");

        // Convenience: default is the first root
        if let Some(&first_root) = graph.roots.first()
            && let Some(unit) = graph.units.get(first_root)
        {
            out.push_str(&format!(
                "\n  default = units.\"{}\";\n",
                unit.derivation_name()
            ));
        }

        out.push_str("}\n");

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_graph::parse_test_unit_graph;

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

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let identity_hash = unit.identity_hash();
        let drv_name = unit.derivation_name();

        let drv = UnitDerivation::from_unit(
            unit,
            "/workspace",
            false,
            "rustToolchain",
            &drv_name,
            &identity_hash,
            false, // not an external dep (path source)
        );

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

        let graph = parse_test_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Check structure
        assert!(nix.contains("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src, extraNativeBuildInputs ? [], vendorDir ? null }:"));
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

        let graph = parse_test_unit_graph(json);
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

        // Should have bin output in installPhase
        assert!(nix.contains("cp build/app $out/bin/"));

        // Should have --extern flag for dependency (with identity hash in filename)
        assert!(nix.contains("--extern"));
        assert!(nix.contains("dep="));
        // Library files include identity hash: libdep-{hash}.rlib
        assert!(nix.contains("/lib/libdep-") && nix.contains(".rlib"));

        // -L flags are NOT added for direct deps (they're covered by --extern with explicit path)
        // This test only has one direct dep, so no -L flags are generated
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

        let graph = parse_test_unit_graph(json);
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

        // Regular lib dep should use .rlib (with identity hash in filename)
        assert!(nix.contains("libserde-") && nix.contains(".rlib"));

        // Proc-macro dep should use variable with platform fallback
        // Should have variable setup: PROCMACRO_SERDE_DERIVE="...dylib"
        assert!(nix.contains("PROCMACRO_SERDE_DERIVE="));
        // Should have .dylib and .so fallback
        assert!(nix.contains("libserde_derive-") && nix.contains(".dylib"));
        assert!(nix.contains("libserde_derive-") && nix.contains(".so"));
        // Should use the variable in --extern: serde_derive="$PROCMACRO_SERDE_DERIVE"
        assert!(nix.contains("serde_derive=\"$PROCMACRO_SERDE_DERIVE\""));
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
            lib_search_deps: vec![],
            build_script_ref: None,
            rustc_flags: RustcFlags::new(),
            content_addressed: false,
            toolchain_var: "rustToolchain".to_string(),
        };

        // Add a dependency
        drv.add_dep(DepRef {
            nix_var: "units.\"dep-0.1.0-xyz789\"".to_string(),
            extern_crate_name: "dep".to_string(),
            lib_name: "dep".to_string(),
            identity_hash: "xyz789".to_string(),
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

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let identity_hash = unit.identity_hash();
        let drv_name = unit.derivation_name();

        let drv = UnitDerivation::from_unit(
            unit,
            "/workspace",
            false,
            "rustToolchain",
            &drv_name,
            &identity_hash,
            false, // not an external dep
        );
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

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let identity_hash = unit.identity_hash();
        let drv_name = unit.derivation_name();

        // Without content-addressed
        let drv = UnitDerivation::from_unit(
            unit,
            "/workspace",
            false,
            "rustToolchain",
            &drv_name,
            &identity_hash,
            false, // not an external dep
        );
        let nix = drv.to_nix();
        assert!(!nix.contains("__contentAddressed"));
        assert!(!nix.contains("outputHashMode"));
        assert!(!nix.contains("outputHashAlgo"));

        // With content-addressed
        let drv_ca = UnitDerivation::from_unit(
            unit,
            "/workspace",
            true,
            "rustToolchain",
            &drv_name,
            &identity_hash,
            false, // not an external dep
        );
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

        let graph = parse_test_unit_graph(json);

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
        // Real cargo output has THREE units for build scripts:
        // 1. COMPILE unit: mode="build", kind=["custom-build"] - compiles build.rs
        // 2. RUN unit: mode="run-custom-build" - executes the compiled binary
        // 3. LIB unit: depends on RUN unit for build script outputs
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
                    "mode": "build",
                    "dependencies": []
                },
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
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "build_script_build", "public": false}
                    ]
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
                        {"index": 1, "extern_crate_name": "build_script_build", "public": false}
                    ]
                }
            ],
            "roots": [2]
        }"#;

        let graph = parse_test_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Should have build script compile derivation (now uses target name "build-script-build")
        assert!(
            nix.contains("pname = \"build-script-build\""),
            "missing build script compile derivation"
        );

        // Should have build script run derivation
        assert!(
            nix.contains("my-crate-build-script-run-"),
            "missing build script run derivation name"
        );
        assert!(
            nix.contains("pname = \"my-crate-build-script-output\""),
            "missing build script output pname"
        );

        // The library should read build script outputs
        assert!(
            nix.contains("BUILD_SCRIPT_FLAGS"),
            "missing BUILD_SCRIPT_FLAGS"
        );
        assert!(
            nix.contains("# Read build script outputs"),
            "missing build script outputs comment"
        );
        assert!(nix.contains("rustc-cfg"), "missing rustc-cfg handling");

        // Library build phase should include $BUILD_SCRIPT_FLAGS
        assert!(
            nix.contains("$BUILD_SCRIPT_FLAGS"),
            "missing $BUILD_SCRIPT_FLAGS in build phase"
        );

        // Library should have build script run derivation in buildInputs
        assert!(
            nix.contains("my-crate-build-script-run-"),
            "missing build script run derivation reference"
        );
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
            lib_search_deps: vec![],
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
            lib_name: "dep".to_string(),
            identity_hash: "xyz789".to_string(),
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

        let graph = parse_test_unit_graph(json);

        // Without cross-compilation: both use rustToolchain
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            cross_compiling: false,
            ..Default::default()
        };
        let nix = NixGenerator::new(config).generate(&graph);

        // Should use rustToolchain for both (hostRustToolchain is in signature but defaults to rustToolchain)
        assert!(nix.contains("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src, extraNativeBuildInputs ? [], vendorDir ? null }:"));
        // Proc-macro should use rustToolchain when not cross-compiling
        assert!(nix.contains("nativeBuildInputs = [ rustToolchain ]"));
        // Should NOT have hostRustToolchain in nativeBuildInputs when not cross-compiling
        assert!(!nix.contains("nativeBuildInputs = [ hostRustToolchain ]"));

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
            nix_cross.contains("{ pkgs, rustToolchain, hostRustToolchain ? rustToolchain, src, extraNativeBuildInputs ? [], vendorDir ? null }:")
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

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let identity_hash = unit.identity_hash();
        let drv_name = unit.derivation_name();

        let drv = UnitDerivation::from_unit(
            unit,
            "/workspace",
            false,
            "rustToolchain",
            &drv_name,
            &identity_hash,
            false, // not an external dep
        );
        let build_phase = drv.generate_build_phase();

        // Should use --out-dir for libraries (including proc-macros)
        assert!(build_phase.contains("--out-dir build"));
        assert!(build_phase.contains("--emit=dep-info,link"));
        assert!(drv.is_proc_macro);

        // Check install phase copies all outputs to $out
        let install_phase = drv.generate_install_phase();
        assert!(install_phase.contains("$out/lib"));
        assert!(install_phase.contains("cp build/*"));
    }

    #[test]
    fn test_workspace_packages_attrset() {
        // Test workspace with multiple root units
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "core-lib 0.1.0 (path+file:///workspace/crates/core)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "core_lib",
                        "src_path": "/workspace/crates/core/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": []
                },
                {
                    "pkg_id": "my-app 0.1.0 (path+file:///workspace/crates/app)",
                    "target": {
                        "kind": ["bin"],
                        "crate_types": ["bin"],
                        "name": "my_app",
                        "src_path": "/workspace/crates/app/src/main.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "core_lib", "public": false}
                    ]
                },
                {
                    "pkg_id": "cli-tool 0.1.0 (path+file:///workspace/crates/cli)",
                    "target": {
                        "kind": ["bin"],
                        "crate_types": ["bin"],
                        "name": "cli_tool",
                        "src_path": "/workspace/crates/cli/src/main.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [
                        {"index": 0, "extern_crate_name": "core_lib", "public": false}
                    ]
                }
            ],
            "roots": [0, 1, 2]
        }"#;

        let graph = parse_test_unit_graph(json);
        let config = NixGenConfig {
            workspace_root: "/workspace".to_string(),
            content_addressed: false,
            ..Default::default()
        };

        let generator = NixGenerator::new(config);
        let nix = generator.generate(&graph);

        // Should have packages attrset with all roots
        assert!(nix.contains("packages = {"));
        assert!(nix.contains("\"core_lib\" = units.\""));
        assert!(nix.contains("\"my_app\" = units.\""));
        assert!(nix.contains("\"cli_tool\" = units.\""));

        // Should have binaries attrset with only binaries
        assert!(nix.contains("binaries = {"));
        // binaries should contain my_app and cli_tool but NOT core_lib
        let binaries_section = nix
            .split("# Binary targets only")
            .nth(1)
            .unwrap()
            .split("# Library targets only")
            .next()
            .unwrap();
        assert!(binaries_section.contains("\"my_app\""));
        assert!(binaries_section.contains("\"cli_tool\""));
        assert!(!binaries_section.contains("\"core_lib\""));

        // Should have libraries attrset with only libraries
        assert!(nix.contains("libraries = {"));
        let libraries_section = nix.split("# Library targets only").nth(1).unwrap();
        assert!(libraries_section.contains("\"core_lib\""));
        // Libraries should NOT contain binaries
        assert!(
            !libraries_section
                .split("default =")
                .next()
                .unwrap()
                .contains("\"my_app\"")
        );
    }
}
