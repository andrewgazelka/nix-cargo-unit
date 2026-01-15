//! Cargo unit graph types.
//!
//! These types represent the JSON output from `cargo build --unit-graph -Z unstable-options`.
//! Each unit represents a single rustc invocation in the build graph.

/// The root structure of the unit graph JSON.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct UnitGraph {
    /// JSON structure version (currently 1).
    pub version: u32,

    /// All compilation units in the build graph.
    pub units: Vec<Unit>,

    /// Indices into `units` for the root units (final outputs).
    pub roots: Vec<usize>,
}

/// A single compilation unit (one rustc invocation).
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Unit {
    /// Opaque package identifier in format "name version (source)".
    /// Example: "serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)"
    pub pkg_id: String,

    /// The target being built (lib, bin, test, etc.).
    pub target: Target,

    /// Compilation profile (dev, release, etc.).
    pub profile: Profile,

    /// Resolved features for this compilation.
    pub features: Vec<String>,

    /// Build mode.
    /// Values: "build", "check", "test", "doc", "doctest", "run-custom-build"
    pub mode: String,

    /// Dependencies required for this unit.
    pub dependencies: Vec<Dependency>,

    /// Target triple for cross-compilation, or null for host platform.
    /// Proc-macros always compile for host (this will be the host triple).
    #[serde(default)]
    pub platform: Option<String>,

    /// Whether this unit is from the `build-std` feature.
    #[serde(default)]
    pub is_std: bool,
}

/// A build target (library, binary, test, example, etc.).
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Target {
    /// Target kind(s).
    /// Values: "lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro",
    ///         "bin", "example", "test", "bench", "custom-build"
    pub kind: Vec<String>,

    /// Crate types to generate.
    /// Values: "lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro", "bin"
    pub crate_types: Vec<String>,

    /// Crate name (may differ from package name, e.g., hyphens become underscores).
    pub name: String,

    /// Absolute path to the entry point source file.
    pub src_path: String,

    /// Rust edition.
    /// Values: "2015", "2018", "2021", "2024"
    pub edition: String,

    /// Whether tests are enabled for this target.
    #[serde(default = "default_true")]
    pub test: bool,

    /// Whether doctests are enabled for this target.
    #[serde(default = "default_true")]
    pub doctest: bool,

    /// Whether documentation is enabled for this target.
    #[serde(default = "default_true")]
    pub doc: bool,
}

/// Compilation profile settings.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Profile {
    /// Profile name (e.g., "dev", "release", "test", "bench").
    pub name: String,

    /// Optimization level.
    /// Values: "0", "1", "2", "3", "s", "z"
    pub opt_level: String,

    /// LTO (Link-Time Optimization) setting.
    /// Values: "false", "true", "thin", "fat", "off"
    #[serde(default)]
    pub lto: LtoSetting,

    /// Number of codegen units, or null for default.
    #[serde(default)]
    pub codegen_units: Option<u32>,

    /// Debug information level.
    /// 0 = none, 1 = line tables, 2 = full
    #[serde(default)]
    pub debuginfo: DebugInfo,

    /// Whether debug assertions are enabled.
    #[serde(default)]
    pub debug_assertions: bool,

    /// Whether overflow checks are enabled.
    #[serde(default)]
    pub overflow_checks: bool,

    /// Whether to set rpath.
    #[serde(default)]
    pub rpath: bool,

    /// Whether incremental compilation is enabled.
    #[serde(default)]
    pub incremental: bool,

    /// Panic strategy.
    /// Values: "unwind", "abort"
    #[serde(default)]
    pub panic: PanicStrategy,

    /// Symbol stripping setting.
    #[serde(default)]
    pub strip: StripSetting,

    /// Split debuginfo setting.
    #[serde(default)]
    pub split_debuginfo: Option<String>,
}

/// LTO setting (can be string "false"/"true"/"thin"/"fat" or boolean).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum LtoSetting {
    Off,
    Thin,
    Fat,
}

impl Default for LtoSetting {
    fn default() -> Self {
        Self::Off
    }
}

impl<'de> serde::Deserialize<'de> for LtoSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct LtoVisitor;

        impl serde::de::Visitor<'_> for LtoVisitor {
            type Value = LtoSetting;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a boolean or string LTO setting")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(if v { LtoSetting::Fat } else { LtoSetting::Off })
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match v {
                    "false" | "off" => Ok(LtoSetting::Off),
                    "true" | "fat" => Ok(LtoSetting::Fat),
                    "thin" => Ok(LtoSetting::Thin),
                    other => Err(serde::de::Error::unknown_variant(
                        other,
                        &["false", "true", "thin", "fat", "off"],
                    )),
                }
            }
        }

        deserializer.deserialize_any(LtoVisitor)
    }
}

/// Debug information level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DebugInfo {
    None,
    LineDirectivesOnly,
    LineTablesOnly,
    Limited,
    Full,
}

impl Default for DebugInfo {
    fn default() -> Self {
        Self::None
    }
}

impl<'de> serde::Deserialize<'de> for DebugInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct DebugInfoVisitor;

        impl serde::de::Visitor<'_> for DebugInfoVisitor {
            type Value = DebugInfo;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an integer, boolean, or string debuginfo setting")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(if v { DebugInfo::Full } else { DebugInfo::None })
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match v {
                    0 => Ok(DebugInfo::None),
                    1 => Ok(DebugInfo::Limited),
                    2 => Ok(DebugInfo::Full),
                    _ => Ok(DebugInfo::Full),
                }
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match v {
                    0 => Ok(DebugInfo::None),
                    1 => Ok(DebugInfo::Limited),
                    2 => Ok(DebugInfo::Full),
                    _ => Ok(DebugInfo::Full),
                }
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match v {
                    "0" | "none" | "false" => Ok(DebugInfo::None),
                    "line-directives-only" => Ok(DebugInfo::LineDirectivesOnly),
                    "line-tables-only" => Ok(DebugInfo::LineTablesOnly),
                    "1" | "limited" => Ok(DebugInfo::Limited),
                    "2" | "full" | "true" => Ok(DebugInfo::Full),
                    other => Err(serde::de::Error::unknown_variant(
                        other,
                        &[
                            "0",
                            "1",
                            "2",
                            "none",
                            "limited",
                            "full",
                            "line-directives-only",
                            "line-tables-only",
                        ],
                    )),
                }
            }
        }

        deserializer.deserialize_any(DebugInfoVisitor)
    }
}

/// Panic strategy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanicStrategy {
    #[default]
    Unwind,
    Abort,
}

/// Symbol stripping setting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum StripSetting {
    None,
    Debuginfo,
    Symbols,
}

impl Default for StripSetting {
    fn default() -> Self {
        Self::None
    }
}

impl StripSetting {
    fn from_str_value(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "none" | "false" => StripSetting::None,
            "debuginfo" => StripSetting::Debuginfo,
            "symbols" | "true" => StripSetting::Symbols,
            _ => StripSetting::None,
        }
    }
}

impl<'de> serde::Deserialize<'de> for StripSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // First, try to deserialize as a generic JSON value to handle all formats
        let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;

        Ok(match value {
            serde_json::Value::Bool(b) => {
                if b {
                    StripSetting::Symbols
                } else {
                    StripSetting::None
                }
            }
            serde_json::Value::String(s) => StripSetting::from_str_value(&s),
            serde_json::Value::Object(obj) => {
                // Handle new format: {"resolved": {"Named": "debuginfo"}} or {"resolved": "None"}
                if let Some(resolved) = obj.get("resolved") {
                    match resolved {
                        serde_json::Value::String(s) => StripSetting::from_str_value(s),
                        serde_json::Value::Object(inner) => {
                            if let Some(serde_json::Value::String(s)) = inner.get("Named") {
                                StripSetting::from_str_value(s)
                            } else {
                                StripSetting::None
                            }
                        }
                        _ => StripSetting::None,
                    }
                } else {
                    StripSetting::None
                }
            }
            _ => StripSetting::None,
        })
    }
}

/// A dependency link between units.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Dependency {
    /// Index into the `units` array for the dependency unit.
    pub index: usize,

    /// Name to use for `--extern` flag.
    /// May differ from crate name (e.g., hyphens normalized).
    pub extern_crate_name: String,

    /// Whether this is a public dependency (requires `public-dependency` feature).
    #[serde(default)]
    pub public: bool,

    /// Whether to skip injecting into prelude (used by `build-std`).
    #[serde(default)]
    pub noprelude: bool,
}

fn default_true() -> bool {
    true
}

// Helper methods for Unit

impl Unit {
    /// Returns true if this unit is a build script (build.rs).
    pub fn is_build_script(&self) -> bool {
        self.mode == "run-custom-build" || self.target.kind.contains(&"custom-build".to_string())
    }

    /// Returns true if this unit is a proc-macro.
    pub fn is_proc_macro(&self) -> bool {
        self.target.kind.contains(&"proc-macro".to_string())
    }

    /// Returns true if this unit is a library (any lib type).
    pub fn is_lib(&self) -> bool {
        self.target.kind.iter().any(|k| {
            matches!(
                k.as_str(),
                "lib" | "rlib" | "dylib" | "cdylib" | "staticlib"
            )
        })
    }

    /// Returns true if this unit is a binary.
    pub fn is_bin(&self) -> bool {
        self.target.kind.contains(&"bin".to_string())
    }

    /// Returns true if this unit is a test.
    pub fn is_test(&self) -> bool {
        self.target.kind.contains(&"test".to_string()) || self.mode == "test"
    }

    /// Extracts the package name from pkg_id.
    /// Format (new): "path+file:///...#name@version" -> "name"
    /// Format (old): "name version (source)" -> "name"
    pub fn package_name(&self) -> &str {
        // Handle new Cargo format: "path+file:///...#name@version" or "registry+...#name@version"
        if let Some(hash_pos) = self.pkg_id.find('#') {
            let after_hash = &self.pkg_id[hash_pos + 1..];
            // Split on @ to separate name from version
            if let Some(at_pos) = after_hash.find('@') {
                return &after_hash[..at_pos];
            }
            return after_hash;
        }

        // Fallback to old format: "name version (source)"
        self.pkg_id
            .split_whitespace()
            .next()
            .unwrap_or(&self.pkg_id)
    }

    /// Extracts the package version from pkg_id.
    /// Format (new): "path+file:///...#name@version" -> "version"
    /// Format (old): "name version (source)" -> "version"
    pub fn package_version(&self) -> Option<&str> {
        // Handle new Cargo format: "path+file:///...#name@version"
        if let Some(hash_pos) = self.pkg_id.find('#') {
            let after_hash = &self.pkg_id[hash_pos + 1..];
            if let Some(at_pos) = after_hash.find('@') {
                return Some(&after_hash[at_pos + 1..]);
            }
            // No version in the new format
            return None;
        }

        // Fallback to old format: "name version (source)"
        let mut parts = self.pkg_id.split_whitespace();
        parts.next(); // skip name
        parts.next() // return version
    }

    /// Computes a unique identity hash for this unit.
    ///
    /// The identity is a SHA-256 hash of (pkg_id, sorted features, profile key fields, mode, target name, crate types).
    /// This can be used as a unique derivation key since the same package can appear
    /// multiple times with different features or profiles.
    ///
    /// Returns a 16-character hex string (first 64 bits of SHA-256).
    pub fn identity_hash(&self) -> String {
        use sha2::Digest as _;

        let mut hasher = sha2::Sha256::new();

        // Package identity
        hasher.update(self.pkg_id.as_bytes());
        hasher.update(b"\0");

        // Target name and crate types (same pkg can have multiple targets)
        hasher.update(self.target.name.as_bytes());
        hasher.update(b"\0");
        for ct in &self.target.crate_types {
            hasher.update(ct.as_bytes());
            hasher.update(b"\0");
        }

        // Sorted features for determinism
        let mut features = self.features.clone();
        features.sort();
        for feature in &features {
            hasher.update(feature.as_bytes());
            hasher.update(b"\0");
        }

        // Profile fields that affect compilation output
        hasher.update(self.profile.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.profile.opt_level.as_bytes());
        hasher.update(b"\0");
        hasher.update(format!("{:?}", self.profile.lto).as_bytes());
        hasher.update(b"\0");
        hasher.update(format!("{:?}", self.profile.debuginfo).as_bytes());
        hasher.update(b"\0");
        hasher.update(format!("{:?}", self.profile.panic).as_bytes());
        hasher.update(b"\0");
        hasher.update(if self.profile.debug_assertions {
            b"1"
        } else {
            b"0"
        });
        hasher.update(if self.profile.overflow_checks {
            b"1"
        } else {
            b"0"
        });

        // Codegen units (affects output)
        if let Some(cgu) = self.profile.codegen_units {
            hasher.update(cgu.to_string().as_bytes());
        }
        hasher.update(b"\0");

        // Build mode
        hasher.update(self.mode.as_bytes());
        hasher.update(b"\0");

        // Platform (proc-macros compile for host)
        if let Some(ref platform) = self.platform {
            hasher.update(platform.as_bytes());
        }
        hasher.update(b"\0");

        // Take first 8 bytes (16 hex chars) for a reasonably unique short ID
        let result = hasher.finalize();
        hex::encode(&result[..8])
    }

    /// Returns a Nix-safe derivation name for this unit.
    ///
    /// Format: `{crate_name}-{version}-{identity_hash}`
    /// Example: `serde-1.0.219-a1b2c3d4e5f67890`
    pub fn derivation_name(&self) -> String {
        let name = &self.target.name;
        let version = self.package_version().unwrap_or("0.0.0");
        let hash = self.identity_hash();
        format!("{name}-{version}-{hash}")
    }
}

impl UnitGraph {
    /// Returns an iterator over root units.
    pub fn root_units(&self) -> impl Iterator<Item = &Unit> {
        self.roots.iter().filter_map(|&i| self.units.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_unit_graph() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-package 0.1.0 (path+file:///test)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "my_package",
                        "src_path": "/test/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {
                        "name": "dev",
                        "opt_level": "0"
                    },
                    "features": ["default"],
                    "mode": "build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph: UnitGraph = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(graph.version, 1);
        assert_eq!(graph.units.len(), 1);
        assert_eq!(graph.roots, vec![0]);

        let unit = &graph.units[0];
        assert_eq!(unit.package_name(), "my-package");
        assert_eq!(unit.package_version(), Some("0.1.0"));
        assert!(unit.is_lib());
        assert!(!unit.is_bin());
        assert!(!unit.is_proc_macro());
        assert!(!unit.is_build_script());
    }

    #[test]
    fn test_parse_full_profile() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "test 0.1.0 (path+file:///test)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "test",
                        "src_path": "/test/src/lib.rs",
                        "edition": "2021",
                        "test": true,
                        "doctest": false,
                        "doc": true
                    },
                    "profile": {
                        "name": "release",
                        "opt_level": "3",
                        "lto": "thin",
                        "codegen_units": 16,
                        "debuginfo": 0,
                        "debug_assertions": false,
                        "overflow_checks": false,
                        "rpath": false,
                        "incremental": false,
                        "panic": "abort",
                        "strip": "symbols"
                    },
                    "features": [],
                    "mode": "build",
                    "dependencies": [],
                    "platform": "x86_64-unknown-linux-gnu",
                    "is_std": false
                }
            ],
            "roots": [0]
        }"#;

        let graph: UnitGraph = serde_json::from_str(json).expect("failed to parse");
        let unit = &graph.units[0];

        assert_eq!(unit.profile.opt_level, "3");
        assert_eq!(unit.profile.lto, LtoSetting::Thin);
        assert_eq!(unit.profile.codegen_units, Some(16));
        assert_eq!(unit.profile.debuginfo, DebugInfo::None);
        assert!(!unit.profile.debug_assertions);
        assert!(!unit.profile.overflow_checks);
        assert_eq!(unit.profile.panic, PanicStrategy::Abort);
        assert_eq!(unit.profile.strip, StripSetting::Symbols);

        assert!(unit.target.test);
        assert!(!unit.target.doctest);
        assert!(unit.target.doc);
        assert_eq!(unit.platform, Some("x86_64-unknown-linux-gnu".to_string()));
    }

    #[test]
    fn test_parse_lto_variants() {
        // Test boolean false
        let json = r#"{"name":"dev","opt_level":"0","lto":false}"#;
        let profile: Profile = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(profile.lto, LtoSetting::Off);

        // Test boolean true
        let json = r#"{"name":"dev","opt_level":"0","lto":true}"#;
        let profile: Profile = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(profile.lto, LtoSetting::Fat);

        // Test string "thin"
        let json = r#"{"name":"dev","opt_level":"0","lto":"thin"}"#;
        let profile: Profile = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(profile.lto, LtoSetting::Thin);

        // Test string "false"
        let json = r#"{"name":"dev","opt_level":"0","lto":"false"}"#;
        let profile: Profile = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(profile.lto, LtoSetting::Off);
    }

    #[test]
    fn test_parse_debuginfo_variants() {
        // Test integer
        let json = r#"{"name":"dev","opt_level":"0","debuginfo":2}"#;
        let profile: Profile = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(profile.debuginfo, DebugInfo::Full);

        // Test string
        let json = r#"{"name":"dev","opt_level":"0","debuginfo":"line-tables-only"}"#;
        let profile: Profile = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(profile.debuginfo, DebugInfo::LineTablesOnly);
    }

    #[test]
    fn test_dependency_with_noprelude() {
        let json = r#"{
            "index": 5,
            "extern_crate_name": "core",
            "public": false,
            "noprelude": true
        }"#;

        let dep: Dependency = serde_json::from_str(json).expect("failed to parse");
        assert_eq!(dep.index, 5);
        assert_eq!(dep.extern_crate_name, "core");
        assert!(!dep.public);
        assert!(dep.noprelude);
    }

    #[test]
    fn test_proc_macro_detection() {
        let json = r#"{
            "version": 1,
            "units": [
                {
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
                    "platform": "x86_64-apple-darwin"
                }
            ],
            "roots": [0]
        }"#;

        let graph: UnitGraph = serde_json::from_str(json).expect("failed to parse");
        let unit = &graph.units[0];

        assert!(unit.is_proc_macro());
        assert!(!unit.is_lib());
        assert!(unit.platform.is_some());
    }

    #[test]
    fn test_build_script_detection() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-pkg 0.1.0 (path+file:///test)",
                    "target": {
                        "kind": ["custom-build"],
                        "crate_types": ["bin"],
                        "name": "build-script-build",
                        "src_path": "/test/build.rs",
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

        let graph: UnitGraph = serde_json::from_str(json).expect("failed to parse");
        let unit = &graph.units[0];

        assert!(unit.is_build_script());
        assert!(!unit.is_lib());
        assert!(!unit.is_bin());
    }

    #[test]
    fn test_identity_hash_deterministic() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-package 0.1.0 (path+file:///test)",
                    "target": {
                        "kind": ["lib"],
                        "crate_types": ["lib"],
                        "name": "my_package",
                        "src_path": "/test/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {
                        "name": "dev",
                        "opt_level": "0"
                    },
                    "features": ["default", "std"],
                    "mode": "build",
                    "dependencies": []
                }
            ],
            "roots": [0]
        }"#;

        let graph: UnitGraph = serde_json::from_str(json).expect("failed to parse");
        let unit = &graph.units[0];

        // Hash should be deterministic
        let hash1 = unit.identity_hash();
        let hash2 = unit.identity_hash();
        assert_eq!(hash1, hash2);

        // Hash should be 16 hex chars (8 bytes)
        assert_eq!(hash1.len(), 16);
        assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_identity_hash_feature_order_independent() {
        // Features in different order should produce same hash
        let json1 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["a", "b", "c"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let json2 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["c", "a", "b"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph1: UnitGraph = serde_json::from_str(json1).expect("failed to parse");
        let graph2: UnitGraph = serde_json::from_str(json2).expect("failed to parse");

        assert_eq!(
            graph1.units[0].identity_hash(),
            graph2.units[0].identity_hash()
        );
    }

    #[test]
    fn test_identity_hash_differs_by_features() {
        let json1 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["std"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let json2 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["std", "alloc"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph1: UnitGraph = serde_json::from_str(json1).expect("failed to parse");
        let graph2: UnitGraph = serde_json::from_str(json2).expect("failed to parse");

        assert_ne!(
            graph1.units[0].identity_hash(),
            graph2.units[0].identity_hash()
        );
    }

    #[test]
    fn test_identity_hash_differs_by_profile() {
        let json1 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let json2 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "release", "opt_level": "3"},
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph1: UnitGraph = serde_json::from_str(json1).expect("failed to parse");
        let graph2: UnitGraph = serde_json::from_str(json2).expect("failed to parse");

        assert_ne!(
            graph1.units[0].identity_hash(),
            graph2.units[0].identity_hash()
        );
    }

    #[test]
    fn test_identity_hash_differs_by_mode() {
        let json1 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let json2 = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "test 0.1.0 (path+file:///test)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "test", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": [],
                "mode": "test",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph1: UnitGraph = serde_json::from_str(json1).expect("failed to parse");
        let graph2: UnitGraph = serde_json::from_str(json2).expect("failed to parse");

        assert_ne!(
            graph1.units[0].identity_hash(),
            graph2.units[0].identity_hash()
        );
    }

    #[test]
    fn test_derivation_name() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)",
                "target": {"kind": ["lib"], "crate_types": ["lib"], "name": "serde", "src_path": "/test/src/lib.rs", "edition": "2021"},
                "profile": {"name": "dev", "opt_level": "0"},
                "features": ["default", "std"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph: UnitGraph = serde_json::from_str(json).expect("failed to parse");
        let unit = &graph.units[0];

        let name = unit.derivation_name();
        assert!(name.starts_with("serde-1.0.219-"));
        assert_eq!(name.len(), "serde-1.0.219-".len() + 16); // 16 hex chars
    }
}
