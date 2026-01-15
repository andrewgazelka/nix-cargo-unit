//! Proc-macro detection and host toolchain compilation.
//!
//! Procedural macros are special compilation units that execute at compile time
//! to transform Rust code. Unlike regular libraries, proc-macros have unique
//! requirements:
//!
//! 1. **Host compilation**: Proc-macros must be compiled for the HOST platform
//!    (the machine running rustc), not the target platform. This is critical
//!    for cross-compilation scenarios.
//!
//! 2. **Shared library output**: Proc-macros are compiled as dynamic libraries
//!    (`.so` on Linux, `.dylib` on macOS, `.dll` on Windows) that rustc loads
//!    at compile time.
//!
//! 3. **Platform field**: In the unit graph, proc-macro units have a non-null
//!    `platform` field indicating the host platform they compile for.
//!
//! ## Unit Graph Detection
//!
//! Proc-macros are identified by:
//! - `target.kind` containing `"proc-macro"`
//! - `target.crate_types` containing `"proc-macro"`
//! - Non-null `platform` field (indicates host platform)
//!
//! ```json
//! {
//!   "target": {
//!     "kind": ["proc-macro"],
//!     "crate_types": ["proc-macro"],
//!     "name": "my_macro"
//!   },
//!   "platform": "aarch64-apple-darwin"
//! }
//! ```
//!
//! ## Nix Generation
//!
//! When generating Nix derivations for proc-macros:
//! - Use `hostRustToolchain` instead of `rustToolchain`
//! - Output to `$out/lib/lib{name}.{ext}` where ext is platform-specific
//! - Dependencies that are proc-macros need special `--extern` handling

use crate::unit_graph::Unit;

/// Information about a proc-macro unit.
#[derive(Debug, Clone)]
pub struct ProcMacroInfo {
    /// The package name.
    pub package_name: String,

    /// The crate name (used for library file naming).
    pub crate_name: String,

    /// Package version.
    pub version: String,

    /// The host platform this proc-macro compiles for.
    /// This comes from the unit's `platform` field.
    pub host_platform: String,

    /// Whether this is for a cross-compilation scenario.
    /// True when host platform differs from target platform.
    pub is_cross_compile: bool,
}

impl ProcMacroInfo {
    /// Extracts proc-macro information from a unit.
    ///
    /// Returns `None` if the unit is not a proc-macro.
    pub fn from_unit(unit: &Unit, target_platform: Option<&str>) -> Option<Self> {
        if !unit.is_proc_macro() {
            return None;
        }

        let package_name = unit.package_name().to_string();
        let crate_name = unit.target.name.clone();
        let version = unit.package_version().unwrap_or("0.0.0").to_string();

        // Proc-macros always have a platform field indicating host
        let host_platform = unit.platform.clone().unwrap_or_else(|| {
            // Fallback: if platform is not set, assume current platform
            // This shouldn't happen for proc-macros in practice
            std::env::consts::ARCH.to_string()
        });

        // Detect cross-compilation
        let is_cross_compile = target_platform
            .map(|target| target != host_platform)
            .unwrap_or(false);

        Some(Self {
            package_name,
            crate_name,
            version,
            host_platform,
            is_cross_compile,
        })
    }

    /// Returns the expected library file extension for the host platform.
    pub fn library_extension(&self) -> &'static str {
        platform_library_extension(&self.host_platform)
    }

    /// Returns the full library file name (e.g., `libmy_macro.so`).
    pub fn library_filename(&self) -> String {
        let normalized_name = self.crate_name.replace('-', "_");
        let ext = self.library_extension();
        format!("lib{normalized_name}.{ext}")
    }
}

/// Returns the dynamic library extension for a given platform triple.
///
/// Platform triples have the format: `{arch}-{vendor}-{os}[-{env}]`
/// Examples:
/// - `x86_64-unknown-linux-gnu` -> `so`
/// - `aarch64-apple-darwin` -> `dylib`
/// - `x86_64-pc-windows-msvc` -> `dll`
pub fn platform_library_extension(platform: &str) -> &'static str {
    if platform.contains("darwin") || platform.contains("apple") {
        "dylib"
    } else if platform.contains("windows") {
        "dll"
    } else {
        // Default to Linux/Unix .so
        "so"
    }
}

/// Checks if a unit is a proc-macro.
pub fn is_proc_macro_unit(unit: &Unit) -> bool {
    unit.is_proc_macro()
}

/// Checks if a unit should be compiled with the host toolchain.
///
/// This returns true for:
/// - Proc-macros (always compile for host)
/// - Build scripts (always compile for host)
///
/// In Nix derivations, these units should use `hostRustToolchain`
/// instead of `rustToolchain` when cross-compiling.
pub fn requires_host_toolchain(unit: &Unit) -> bool {
    unit.is_proc_macro() || unit.is_build_script()
}

/// Checks if a dependency is a proc-macro based on the unit graph.
///
/// This is useful when wiring up `--extern` flags, as proc-macros
/// need special handling (dynamic library lookup).
pub fn is_proc_macro_dependency(unit: &Unit) -> bool {
    unit.is_proc_macro()
}

/// Returns the Nix expression for locating a proc-macro library.
///
/// Since proc-macro library extensions vary by platform, we use `find`
/// to locate the actual file regardless of extension.
///
/// # Arguments
/// * `dep_var` - Nix variable referencing the proc-macro derivation
/// * `extern_crate_name` - The name to use in `--extern`
///
/// # Returns
/// A shell command that can be used in the `--extern` flag value.
pub fn proc_macro_extern_expr(dep_var: &str, extern_crate_name: &str) -> String {
    let normalized_name = extern_crate_name.replace('-', "_");
    // Use find to locate the library with any extension
    format!("\"$(find {dep_var}/lib -name 'lib{normalized_name}.*' -type f | head -1)\"")
}

/// Configuration for proc-macro derivation generation.
#[derive(Debug, Clone, Default)]
pub struct ProcMacroConfig {
    /// Whether the build is cross-compiling.
    /// When true, proc-macros use `hostRustToolchain`.
    pub cross_compiling: bool,

    /// The target platform triple (for target crates).
    /// May differ from host platform in cross-compilation.
    pub target_platform: Option<String>,

    /// The host platform triple (for proc-macros and build scripts).
    pub host_platform: Option<String>,
}

impl ProcMacroConfig {
    /// Creates a config for native compilation (no cross-compilation).
    pub fn native() -> Self {
        Self::default()
    }

    /// Creates a config for cross-compilation.
    pub fn cross(host: &str, target: &str) -> Self {
        Self {
            cross_compiling: true,
            target_platform: Some(target.to_string()),
            host_platform: Some(host.to_string()),
        }
    }

    /// Returns the Nix variable for the appropriate toolchain.
    ///
    /// - `"hostRustToolchain"` when cross-compiling for host units
    /// - `"rustToolchain"` otherwise
    pub fn toolchain_var(&self, is_host_unit: bool) -> &'static str {
        if self.cross_compiling && is_host_unit {
            "hostRustToolchain"
        } else {
            "rustToolchain"
        }
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
    fn test_proc_macro_detection() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-macro 0.1.0 (path+file:///workspace)",
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

        assert!(is_proc_macro_unit(unit));
        assert!(requires_host_toolchain(unit));

        let info = ProcMacroInfo::from_unit(unit, None);
        assert!(info.is_some());

        let info = info.unwrap();
        assert_eq!(info.package_name, "my-macro");
        assert_eq!(info.crate_name, "my_macro");
        assert_eq!(info.version, "0.1.0");
        assert_eq!(info.host_platform, "x86_64-unknown-linux-gnu");
        assert!(!info.is_cross_compile);
    }

    #[test]
    fn test_non_proc_macro_returns_none() {
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

        assert!(!is_proc_macro_unit(unit));
        assert!(!requires_host_toolchain(unit));

        let info = ProcMacroInfo::from_unit(unit, None);
        assert!(info.is_none());
    }

    #[test]
    fn test_cross_compile_detection() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-macro 0.1.0 (path+file:///workspace)",
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
                    "platform": "aarch64-apple-darwin"
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];

        // Not cross-compiling when target matches host
        let info_same = ProcMacroInfo::from_unit(unit, Some("aarch64-apple-darwin"));
        assert!(!info_same.unwrap().is_cross_compile);

        // Cross-compiling when target differs from host
        let info_cross = ProcMacroInfo::from_unit(unit, Some("x86_64-unknown-linux-gnu"));
        assert!(info_cross.unwrap().is_cross_compile);
    }

    #[test]
    fn test_platform_library_extension() {
        assert_eq!(platform_library_extension("x86_64-unknown-linux-gnu"), "so");
        assert_eq!(platform_library_extension("aarch64-apple-darwin"), "dylib");
        assert_eq!(platform_library_extension("x86_64-pc-windows-msvc"), "dll");
        assert_eq!(platform_library_extension("x86_64-unknown-freebsd"), "so");
    }

    #[test]
    fn test_library_filename() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-macro 0.1.0 (path+file:///workspace)",
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
        let info = ProcMacroInfo::from_unit(unit, None).unwrap();

        assert_eq!(info.library_filename(), "libmy_macro.so");
    }

    #[test]
    fn test_library_filename_with_hyphen() {
        let json = r#"{
            "version": 1,
            "units": [
                {
                    "pkg_id": "my-derive-macro 0.1.0 (path+file:///workspace)",
                    "target": {
                        "kind": ["proc-macro"],
                        "crate_types": ["proc-macro"],
                        "name": "my-derive-macro",
                        "src_path": "/workspace/src/lib.rs",
                        "edition": "2021"
                    },
                    "profile": {"name": "dev", "opt_level": "0"},
                    "features": [],
                    "mode": "build",
                    "dependencies": [],
                    "platform": "aarch64-apple-darwin"
                }
            ],
            "roots": [0]
        }"#;

        let graph = parse_unit_graph(json);
        let unit = &graph.units[0];
        let info = ProcMacroInfo::from_unit(unit, None).unwrap();

        // Hyphens should be converted to underscores
        assert_eq!(info.library_filename(), "libmy_derive_macro.dylib");
    }

    #[test]
    fn test_proc_macro_extern_expr() {
        let expr = proc_macro_extern_expr("units.\"serde_derive-1.0.0-abc\"", "serde_derive");
        assert!(expr.contains("find"));
        assert!(expr.contains("units.\"serde_derive-1.0.0-abc\""));
        assert!(expr.contains("libserde_derive.*"));
    }

    #[test]
    fn test_proc_macro_config_native() {
        let config = ProcMacroConfig::native();
        assert!(!config.cross_compiling);
        assert_eq!(config.toolchain_var(true), "rustToolchain");
        assert_eq!(config.toolchain_var(false), "rustToolchain");
    }

    #[test]
    fn test_proc_macro_config_cross() {
        let config = ProcMacroConfig::cross("aarch64-apple-darwin", "x86_64-unknown-linux-gnu");
        assert!(config.cross_compiling);
        assert_eq!(
            config.host_platform,
            Some("aarch64-apple-darwin".to_string())
        );
        assert_eq!(
            config.target_platform,
            Some("x86_64-unknown-linux-gnu".to_string())
        );

        // Host units (proc-macros, build scripts) use host toolchain
        assert_eq!(config.toolchain_var(true), "hostRustToolchain");
        // Target units use regular toolchain
        assert_eq!(config.toolchain_var(false), "rustToolchain");
    }

    #[test]
    fn test_build_script_requires_host_toolchain() {
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

        // Build scripts also require host toolchain
        assert!(requires_host_toolchain(unit));
        assert!(!is_proc_macro_unit(unit));
    }
}
