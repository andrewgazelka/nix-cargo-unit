//! Reconstruct rustc flags from unit graph metadata.
//!
//! This module generates the command-line arguments that rustc needs based on
//! the unit metadata from cargo's unit graph. The goal is to reproduce exactly
//! what cargo would pass to rustc.

use crate::unit_graph::{
    DebugInfo, LtoSetting, PanicStrategy, Profile, StripSetting, Target, Unit,
};

/// A builder for rustc command-line arguments.
///
/// This struct accumulates flags and can produce either a `Vec<String>` of arguments
/// or a formatted string suitable for shell scripts.
#[derive(Debug, Default, Clone)]
pub struct RustcFlags {
    args: Vec<String>,
}

impl RustcFlags {
    /// Creates a new empty flag builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds rustc flags from a unit's metadata.
    ///
    /// This reconstructs the flags that cargo would pass to rustc for this unit,
    /// including edition, crate type, optimization level, features, and codegen options.
    ///
    /// Note: `--extern` and `-L` flags for dependencies are NOT included here;
    /// those must be added separately based on the resolved dependency graph.
    pub fn from_unit(unit: &Unit) -> Self {
        let mut flags = Self::new();

        // Crate name - normalize hyphens to underscores as required by rustc
        flags.push_arg("--crate-name");
        flags.push_arg(&unit.target.name.replace('-', "_"));

        // Edition
        flags.add_edition(&unit.target);

        // Crate types
        flags.add_crate_types(&unit.target);

        // Profile-based codegen options
        flags.add_profile_flags(&unit.profile);

        // Features as --cfg
        flags.add_features(&unit.features);

        // Test harness
        if unit.is_test() {
            flags.push_arg("--test");
        }

        // Allow mismatched_lifetime_syntaxes lint (Rust 1.89+)
        // This lint errors on older crates that don't use explicit `'_` lifetimes.
        // Allow it for compatibility with third-party crates on Rust nightly.
        flags.push_arg("-A");
        flags.push_arg("mismatched_lifetime_syntaxes");

        flags
    }

    /// Adds the edition flag.
    fn add_edition(&mut self, target: &Target) {
        self.push_arg("--edition");
        self.push_arg(&target.edition);
    }

    /// Adds crate type flags.
    fn add_crate_types(&mut self, target: &Target) {
        for crate_type in &target.crate_types {
            self.push_arg("--crate-type");
            self.push_arg(crate_type);
        }
    }

    /// Adds all profile-related codegen flags.
    fn add_profile_flags(&mut self, profile: &Profile) {
        // Optimization level
        self.push_codegen_flag("opt-level", &profile.opt_level);

        // Debug info
        self.add_debuginfo(profile.debuginfo);

        // LTO
        self.add_lto(&profile.lto);

        // Codegen units
        if let Some(cgu) = profile.codegen_units {
            self.push_codegen_flag("codegen-units", &cgu.to_string());
        }

        // Debug assertions
        self.push_codegen_bool("debug-assertions", profile.debug_assertions);

        // Overflow checks
        self.push_codegen_bool("overflow-checks", profile.overflow_checks);

        // Panic strategy
        self.add_panic(&profile.panic);

        // Strip
        self.add_strip(&profile.strip);

        // Split debuginfo
        if let Some(ref split) = profile.split_debuginfo {
            self.push_arg("-C");
            self.push_arg(&format!("split-debuginfo={split}"));
        }

        // Rpath (only add if true, false is default)
        if profile.rpath {
            self.push_arg("-C");
            self.push_arg("rpath=yes");
        }

        // Note: incremental is NOT passed to rustc directly; cargo handles it
    }

    /// Adds debuginfo flag.
    fn add_debuginfo(&mut self, debuginfo: DebugInfo) {
        let value = match debuginfo {
            DebugInfo::None => "0",
            DebugInfo::LineDirectivesOnly => "line-directives-only",
            DebugInfo::LineTablesOnly => "line-tables-only",
            DebugInfo::Limited => "1",
            DebugInfo::Full => "2",
        };
        self.push_codegen_flag("debuginfo", value);
    }

    /// Adds LTO flag.
    fn add_lto(&mut self, lto: &LtoSetting) {
        let value = match lto {
            LtoSetting::Off => "off",
            LtoSetting::Thin => "thin",
            LtoSetting::Fat => "fat",
        };
        self.push_codegen_flag("lto", value);
    }

    /// Adds panic strategy flag.
    fn add_panic(&mut self, panic: &PanicStrategy) {
        let value = match panic {
            PanicStrategy::Unwind => "unwind",
            PanicStrategy::Abort => "abort",
        };
        self.push_arg("-C");
        self.push_arg(&format!("panic={value}"));
    }

    /// Adds strip flag.
    fn add_strip(&mut self, strip: &StripSetting) {
        let value = match strip {
            StripSetting::None => "none",
            StripSetting::Debuginfo => "debuginfo",
            StripSetting::Symbols => "symbols",
        };
        self.push_arg("-C");
        self.push_arg(&format!("strip={value}"));
    }

    /// Adds feature cfg flags.
    fn add_features(&mut self, features: &[String]) {
        for feature in features {
            self.push_arg("--cfg");
            self.push_arg(&format!("feature=\"{feature}\""));
        }
    }

    /// Adds metadata hash and extra filename for stable crate identity.
    ///
    /// This is critical for ensuring crates can find their dependencies across
    /// separate compilation units. Without this, rustc may generate different
    /// StableCrateId values for the same crate, causing "can't find crate" errors.
    ///
    /// This generates: `-C metadata=HASH -C extra-filename=-HASH`
    pub fn add_metadata(&mut self, hash: &str) {
        self.push_codegen_flag("metadata", hash);
        self.push_codegen_flag("extra-filename", &format!("-{hash}"));
    }

    /// Adds an extern crate reference.
    ///
    /// This generates: `--extern name=path`
    pub fn add_extern(&mut self, name: &str, path: &str) {
        self.push_arg("--extern");
        self.push_arg(&format!("{name}={path}"));
    }

    /// Adds an extern crate reference without a path (for proc-macros loaded from sysroot).
    ///
    /// This generates: `--extern name`
    pub fn add_extern_nopath(&mut self, name: &str) {
        self.push_arg("--extern");
        self.push_arg(name);
    }

    /// Adds a library search path.
    ///
    /// This generates: `-L dependency=path`
    pub fn add_lib_path(&mut self, path: &str) {
        self.push_arg("-L");
        self.push_arg(&format!("dependency={path}"));
    }

    /// Adds the source file path.
    pub fn add_source(&mut self, path: &str) {
        self.push_arg(path);
    }

    /// Adds the output path.
    pub fn add_output(&mut self, path: &str) {
        self.push_arg("-o");
        self.push_arg(path);
    }

    /// Adds the output directory (for multiple outputs).
    pub fn add_out_dir(&mut self, path: &str) {
        self.push_arg("--out-dir");
        self.push_arg(path);
    }

    /// Adds a raw argument.
    pub fn push_arg(&mut self, arg: &str) {
        self.args.push(arg.to_string());
    }

    /// Adds a codegen flag in the form `-C key=value`.
    fn push_codegen_flag(&mut self, key: &str, value: &str) {
        self.push_arg("-C");
        self.push_arg(&format!("{key}={value}"));
    }

    /// Adds a codegen flag in the form `-C key=yes` or `-C key=no`.
    fn push_codegen_bool(&mut self, key: &str, value: bool) {
        self.push_codegen_flag(key, if value { "yes" } else { "no" });
    }

    /// Returns the flags as a vector of strings.
    pub fn into_args(self) -> Vec<String> {
        self.args
    }

    /// Returns a reference to the flags.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Formats the flags as a shell command string.
    ///
    /// Arguments containing spaces or special characters are quoted.
    pub fn to_shell_string(&self) -> String {
        self.args
            .iter()
            .map(|arg| crate::shell::quote_arg(arg).into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl std::fmt::Display for RustcFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_shell_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_graph::parse_test_unit_graph;

    #[test]
    fn test_basic_lib_flags() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "my-crate 0.1.0 (path+file:///test)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "my_crate",
                    "src_path": "/test/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {
                    "name": "dev",
                    "opt_level": "0"
                },
                "features": ["std", "alloc"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let flags = RustcFlags::from_unit(unit);
        let args = flags.args();

        // Check crate name
        assert!(args.contains(&"--crate-name".to_string()));
        let name_idx = args.iter().position(|a| a == "--crate-name").unwrap();
        assert_eq!(args[name_idx + 1], "my_crate");

        // Check edition
        assert!(args.contains(&"--edition".to_string()));
        let ed_idx = args.iter().position(|a| a == "--edition").unwrap();
        assert_eq!(args[ed_idx + 1], "2021");

        // Check crate type
        assert!(args.contains(&"--crate-type".to_string()));
        let ct_idx = args.iter().position(|a| a == "--crate-type").unwrap();
        assert_eq!(args[ct_idx + 1], "lib");

        // Check features
        assert!(args.contains(&"--cfg".to_string()));
        assert!(args.contains(&"feature=\"std\"".to_string()));
        assert!(args.contains(&"feature=\"alloc\"".to_string()));

        // Check opt-level
        assert!(args.contains(&"opt-level=0".to_string()));
    }

    #[test]
    fn test_release_profile_flags() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "test",
                    "src_path": "/test/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {
                    "name": "release",
                    "opt_level": "3",
                    "lto": "thin",
                    "debuginfo": 0,
                    "debug_assertions": false,
                    "overflow_checks": false,
                    "panic": "abort",
                    "strip": "symbols",
                    "codegen_units": 16
                },
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let flags = RustcFlags::from_unit(unit);
        let args = flags.args();

        assert!(args.contains(&"opt-level=3".to_string()));
        assert!(args.contains(&"lto=thin".to_string()));
        assert!(args.contains(&"debuginfo=0".to_string()));
        assert!(args.contains(&"debug-assertions=no".to_string()));
        assert!(args.contains(&"overflow-checks=no".to_string()));
        assert!(args.contains(&"panic=abort".to_string()));
        assert!(args.contains(&"strip=symbols".to_string()));
        assert!(args.contains(&"codegen-units=16".to_string()));
    }

    #[test]
    fn test_multiple_crate_types() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib", "cdylib"],
                    "name": "test",
                    "src_path": "/test/src/lib.rs",
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
        let flags = RustcFlags::from_unit(unit);
        let shell = flags.to_shell_string();

        // Should have both crate types
        assert!(shell.contains("--crate-type lib"));
        assert!(shell.contains("--crate-type cdylib"));
    }

    #[test]
    fn test_test_mode() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "test",
                    "src_path": "/test/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "test",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let flags = RustcFlags::from_unit(unit);
        let args = flags.args();

        assert!(args.contains(&"--test".to_string()));
    }

    #[test]
    fn test_extern_and_lib_path() {
        let mut flags = RustcFlags::new();
        flags.add_extern("serde", "/nix/store/abc123/lib/libserde.rlib");
        flags.add_lib_path("/nix/store/abc123/lib");

        let args = flags.args();
        assert!(args.contains(&"--extern".to_string()));
        assert!(args.contains(&"serde=/nix/store/abc123/lib/libserde.rlib".to_string()));
        assert!(args.contains(&"-L".to_string()));
        assert!(args.contains(&"dependency=/nix/store/abc123/lib".to_string()));
    }

    #[test]
    fn test_shell_string_escaping() {
        let mut flags = RustcFlags::new();
        flags.push_arg("--cfg");
        flags.push_arg("feature=\"with spaces\"");

        let shell = flags.to_shell_string();
        // Should be quoted due to spaces
        assert!(shell.contains("'feature=\"with spaces\"'"));
    }

    #[test]
    fn test_to_shell_string() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {
                    "kind": ["bin"],
                    "crate_types": ["bin"],
                    "name": "test",
                    "src_path": "/test/src/main.rs",
                    "edition": "2024"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["default"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let flags = RustcFlags::from_unit(unit);
        let shell = flags.to_shell_string();

        assert!(shell.contains("--crate-name test"));
        assert!(shell.contains("--edition 2024"));
        assert!(shell.contains("--crate-type bin"));
        assert!(shell.contains("--cfg 'feature=\"default\"'"));
    }

    #[test]
    fn test_proc_macro_crate_type() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "my-macro 0.1.0 (path+file:///test)",
                "target": {
                    "kind": ["proc-macro"],
                    "crate_types": ["proc-macro"],
                    "name": "my_macro",
                    "src_path": "/test/src/lib.rs",
                    "edition": "2021"
                },
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "build",
                "dependencies": [],
                "platform": "aarch64-apple-darwin"
            }],
            "roots": [0]
        }"#;

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let flags = RustcFlags::from_unit(unit);
        let args = flags.args();

        // Check proc-macro crate type
        let ct_idx = args.iter().position(|a| a == "--crate-type").unwrap();
        assert_eq!(args[ct_idx + 1], "proc-macro");
    }
}
