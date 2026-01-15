//! Build script (build.rs) detection and derivation generation.
//!
//! Build scripts are special compilation units that execute at build time to configure
//! the main compilation. They output directives like:
//! - `cargo:rustc-cfg=...` - conditional compilation flags
//! - `cargo:rustc-link-lib=...` - libraries to link
//! - `cargo:rustc-link-search=...` - library search paths
//! - `cargo:rustc-env=...` - environment variables for rustc
//! - `cargo:rerun-if-changed=...` - rebuild triggers
//!
//! In nix-cargo-unit, build scripts become two derivations:
//! 1. **Compile derivation**: Compiles build.rs to a binary (same as any other bin)
//! 2. **Run derivation**: Executes the binary and captures output directives

use crate::nix_gen::{NixAttrSet, escape_nix_multiline};
use crate::rustc_flags::RustcFlags;
use crate::unit_graph::Unit;

/// Information about a build script unit.
#[derive(Debug, Clone)]
pub struct BuildScriptInfo {
    /// The package that owns this build script.
    pub package_name: String,

    /// Package version.
    pub version: String,

    /// The source path to build.rs (remapped for Nix).
    pub src_path: String,

    /// Unique derivation name for the compiled build script binary.
    pub compile_drv_name: String,

    /// Unique derivation name for the build script execution.
    pub run_drv_name: String,

    /// Rustc flags for compiling the build script.
    pub rustc_flags: RustcFlags,

    /// Features enabled for this build script.
    pub features: Vec<String>,

    /// Whether to use content-addressed derivations.
    pub content_addressed: bool,
}

impl BuildScriptInfo {
    /// Extracts build script information from a unit.
    ///
    /// Returns `None` if the unit is not a build script.
    pub fn from_unit(unit: &Unit, workspace_root: &str, content_addressed: bool) -> Option<Self> {
        if !unit.is_build_script() {
            return None;
        }

        let package_name = unit.package_name().to_string();
        let version = unit.package_version().unwrap_or("0.0.0").to_string();

        // Remap source path
        let src_path =
            crate::source_filter::remap_source_path(&unit.target.src_path, workspace_root, "src");

        // Generate unique derivation names
        let base_hash = unit.identity_hash();
        let compile_drv_name = format!("{package_name}-build-script-{version}-{base_hash}");
        let run_drv_name = format!("{package_name}-build-script-run-{version}-{base_hash}");

        let rustc_flags = RustcFlags::from_unit(unit);

        Some(Self {
            package_name,
            version,
            src_path,
            compile_drv_name,
            run_drv_name,
            rustc_flags,
            features: unit.features.clone(),
            content_addressed,
        })
    }

    /// Generates the Nix derivation for compiling the build script.
    ///
    /// This produces a binary that can be executed.
    pub fn compile_derivation(&self) -> String {
        let mut attrs = NixAttrSet::new();

        attrs.string("pname", &format!("{}-build-script", self.package_name));
        attrs.string("version", &self.version);
        attrs.expr("buildInputs", "[]");
        attrs.expr("nativeBuildInputs", "[ rustToolchain ]");

        if self.content_addressed {
            attrs.bool("__contentAddressed", true);
            attrs.string("outputHashMode", "recursive");
            attrs.string("outputHashAlgo", "sha256");
        }

        let build_phase = self.generate_compile_phase();
        attrs.multiline("buildPhase", &build_phase);
        attrs.multiline("installPhase", "mkdir -p $out");

        attrs.render(2)
    }

    /// Generates the build phase for compiling the build script.
    fn generate_compile_phase(&self) -> String {
        let mut script = String::new();

        script.push_str("mkdir -p $out/bin\n");
        script.push_str("rustc \\\n");

        for arg in self.rustc_flags.args() {
            script.push_str("  ");
            if arg.contains(' ') || arg.contains('"') || arg.contains('$') {
                script.push('\'');
                script.push_str(&arg.replace('\'', "'\\''"));
                script.push('\'');
            } else {
                script.push_str(arg);
            }
            script.push_str(" \\\n");
        }

        script.push_str("  ");
        script.push_str(&self.src_path);
        script.push_str(" \\\n");

        // Build script outputs to bin/build-script
        script.push_str("  -o $out/bin/build-script");

        script
    }

    /// Generates the Nix derivation for running the build script.
    ///
    /// This executes the compiled build script and captures its output directives.
    /// The output is stored in structured files:
    /// - `$out/rustc-cfg` - one cfg per line
    /// - `$out/rustc-link-lib` - one lib per line
    /// - `$out/rustc-link-search` - one path per line
    /// - `$out/rustc-env` - KEY=VALUE per line
    /// - `$out/out-dir` - files generated by the build script
    pub fn run_derivation(&self, compile_drv_var: &str) -> String {
        let mut attrs = NixAttrSet::new();

        attrs.string(
            "pname",
            &format!("{}-build-script-output", self.package_name),
        );
        attrs.string("version", &self.version);

        // Depend on the compiled build script
        attrs.expr("buildInputs", &format!("[ {} ]", compile_drv_var));
        attrs.expr("nativeBuildInputs", "[]");

        if self.content_addressed {
            attrs.bool("__contentAddressed", true);
            attrs.string("outputHashMode", "recursive");
            attrs.string("outputHashAlgo", "sha256");
        }

        let build_phase = self.generate_run_phase(compile_drv_var);
        attrs.multiline("buildPhase", &build_phase);
        attrs.multiline("installPhase", "mkdir -p $out");

        attrs.render(2)
    }

    /// Generates the build phase for running the build script.
    fn generate_run_phase(&self, compile_drv_var: &str) -> String {
        let mut script = String::new();

        // Create output directories
        script.push_str("mkdir -p $out/out-dir\n");

        // Set up environment variables that build scripts expect
        script.push_str("export OUT_DIR=$out/out-dir\n");
        script.push_str(&format!(
            "export CARGO_MANIFEST_DIR=${{src}}/{}\n",
            self.package_name
        ));
        script.push_str(&format!(
            "export CARGO_PKG_NAME=\"{}\"\n",
            self.package_name
        ));
        script.push_str(&format!("export CARGO_PKG_VERSION=\"{}\"\n", self.version));

        // Set feature flags as environment variables
        for feature in &self.features {
            let env_name = format!("CARGO_FEATURE_{}", feature.to_uppercase().replace('-', "_"));
            script.push_str(&format!("export {env_name}=1\n"));
        }

        // Target info (hardcoded for now, should come from config)
        script.push_str("export TARGET=\"$system\"\n");
        script.push_str("export HOST=\"$system\"\n");
        script.push_str("export PROFILE=\"release\"\n");

        // Run the build script and capture output
        script.push_str(&format!(
            "\n# Run build script and parse output\n{}/bin/build-script 2>&1 | while IFS= read -r line; do\n",
            compile_drv_var
        ));

        // Parse cargo: directives
        let parse_script = r#"  case "$line" in
    cargo:rustc-cfg=*)
      echo "''${line#cargo:rustc-cfg=}" >> $out/rustc-cfg
      ;;
    cargo:rustc-link-lib=*)
      echo "''${line#cargo:rustc-link-lib=}" >> $out/rustc-link-lib
      ;;
    cargo:rustc-link-search=*)
      echo "''${line#cargo:rustc-link-search=}" >> $out/rustc-link-search
      ;;
    cargo:rustc-env=*)
      echo "''${line#cargo:rustc-env=}" >> $out/rustc-env
      ;;
    cargo:rustc-cdylib-link-arg=*)
      echo "''${line#cargo:rustc-cdylib-link-arg=}" >> $out/rustc-cdylib-link-arg
      ;;
    cargo:warning=*)
      echo "Build script warning: ''${line#cargo:warning=}" >&2
      ;;
    cargo:rerun-if-changed=*|cargo:rerun-if-env-changed=*)
      # Ignored in Nix (content-addressed handles this)
      ;;
    cargo:*)
      echo "Unknown cargo directive: $line" >&2
      ;;
  esac
done

# Create empty files if they don't exist (for consistent interface)
touch $out/rustc-cfg $out/rustc-link-lib $out/rustc-link-search $out/rustc-env
"#;
        script.push_str(&escape_nix_multiline(parse_script));

        script
    }
}

/// Checks if a unit is a build script that needs special handling.
pub fn is_build_script_unit(unit: &Unit) -> bool {
    unit.is_build_script()
}

/// Checks if a unit's mode is "run-custom-build" (build script execution).
pub fn is_build_script_run(unit: &Unit) -> bool {
    unit.mode == "run-custom-build"
}

/// Checks if a unit's target kind is "custom-build" (build script compilation).
pub fn is_build_script_compile(unit: &Unit) -> bool {
    unit.target.kind.contains(&"custom-build".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_graph::UnitGraph;

    fn parse_unit_graph(json: &str) -> UnitGraph {
        serde_json::from_str(json).expect("failed to parse unit graph")
    }

    #[test]
    fn test_build_script_detection() {
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
                    "features": ["feature-a"],
                    "mode": "run-custom-build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        assert!(is_build_script_unit(unit));
        assert!(is_build_script_run(unit));
        assert!(is_build_script_compile(unit));

        let info = BuildScriptInfo::from_unit(unit, "/workspace", false);
        assert!(info.is_some());

        let info = info.unwrap();
        assert_eq!(info.package_name, "my-crate");
        assert_eq!(info.version, "0.1.0");
        assert!(info.compile_drv_name.starts_with("my-crate-build-script-"));
        assert!(info.run_drv_name.starts_with("my-crate-build-script-run-"));
        assert_eq!(info.features, vec!["feature-a"]);
    }

    #[test]
    fn test_non_build_script_returns_none() {
        let json = r#"{
            "version": 1,
            "units": [
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
                    "features": [],
                    "mode": "build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        assert!(!is_build_script_unit(unit));
        let info = BuildScriptInfo::from_unit(unit, "/workspace", false);
        assert!(info.is_none());
    }

    #[test]
    fn test_compile_derivation() {
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
                    "features": [],
                    "mode": "run-custom-build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];
        let info = BuildScriptInfo::from_unit(unit, "/workspace", false).unwrap();

        let nix = info.compile_derivation();

        assert!(nix.contains("pname = \"my-crate-build-script\""));
        assert!(nix.contains("version = \"0.1.0\""));
        assert!(nix.contains("rustc"));
        assert!(nix.contains("$out/bin/build-script"));
    }

    #[test]
    fn test_run_derivation() {
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
                    "features": ["serde"],
                    "mode": "run-custom-build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];
        let info = BuildScriptInfo::from_unit(unit, "/workspace", false).unwrap();

        let nix = info.run_derivation("buildScript");

        assert!(nix.contains("pname = \"my-crate-build-script-output\""));
        assert!(nix.contains("buildInputs = [ buildScript ]"));
        assert!(nix.contains("OUT_DIR"));
        assert!(nix.contains("CARGO_FEATURE_SERDE"));
        assert!(nix.contains("cargo:rustc-cfg"));
        assert!(nix.contains("cargo:rustc-link-lib"));
    }

    #[test]
    fn test_content_addressed_build_script() {
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
                    "features": [],
                    "mode": "run-custom-build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];
        let info = BuildScriptInfo::from_unit(unit, "/workspace", true).unwrap();

        let compile_nix = info.compile_derivation();
        assert!(compile_nix.contains("__contentAddressed = true"));
        assert!(compile_nix.contains("outputHashMode = \"recursive\""));

        let run_nix = info.run_derivation("buildScript");
        assert!(run_nix.contains("__contentAddressed = true"));
    }
}
