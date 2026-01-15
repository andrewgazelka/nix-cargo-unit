//! Source file filtering utilities for per-crate isolation.
//!
//! This module extracts source directory information from cargo's unit graph
//! and provides utilities for filtering source files to only what's needed
//! for a specific crate compilation.
//!
//! In Nix, we want to minimize the source tree passed to each derivation to
//! improve cache hits. Two compilations of the same crate with identical source
//! should produce identical outputs (with CA-derivations).

use crate::unit_graph::Unit;

/// Parsed package source location information.
///
/// Extracted from the `pkg_id` field and `target.src_path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// Package name (e.g., "serde").
    pub name: String,

    /// Package version (e.g., "1.0.219").
    pub version: String,

    /// Source type and path.
    pub source: SourceType,

    /// The entry point source file (e.g., "src/lib.rs").
    /// This is the relative path from the crate root.
    pub entry_point: String,

    /// The crate root directory.
    /// For path sources, this is the directory containing Cargo.toml.
    /// For registry sources, this is the extracted crate directory.
    pub crate_root: String,
}

/// The type of source for a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceType {
    /// Local path source: `path+file:///absolute/path`
    Path {
        /// Absolute filesystem path to the crate.
        path: String,
    },

    /// Git source: `git+https://...?rev=...#commit`
    Git {
        /// Git URL.
        url: String,
        /// Git reference (branch, tag, or commit).
        reference: Option<String>,
        /// Exact commit hash.
        commit: Option<String>,
    },

    /// Registry source: `registry+https://...`
    Registry {
        /// Registry URL (usually crates.io).
        url: String,
    },
}

impl SourceLocation {
    /// Extracts source location from a unit.
    ///
    /// Parses the `pkg_id` to determine source type and combines with
    /// `target.src_path` to determine the crate root and entry point.
    pub fn from_unit(unit: &Unit) -> Option<Self> {
        let (name, version, source) = parse_pkg_id(&unit.pkg_id)?;
        let (crate_root, entry_point) = extract_crate_root(&unit.target.src_path, &source)?;

        Some(Self {
            name,
            version,
            source,
            entry_point,
            crate_root,
        })
    }

    /// Returns true if this is a local path source.
    pub fn is_path(&self) -> bool {
        matches!(self.source, SourceType::Path { .. })
    }

    /// Returns true if this is a registry source (crates.io).
    pub fn is_registry(&self) -> bool {
        matches!(self.source, SourceType::Registry { .. })
    }

    /// Returns true if this is a git source.
    pub fn is_git(&self) -> bool {
        matches!(self.source, SourceType::Git { .. })
    }

    /// Returns the source directory for use in Nix `lib.fileset`.
    ///
    /// For path sources, returns the directory containing the crate.
    /// This can be used with `lib.fileset.toSource` to create minimal source trees.
    pub fn source_dir(&self) -> &str {
        &self.crate_root
    }

    /// Returns a Nix expression for the source filter.
    ///
    /// This generates a `lib.fileset.toSource` expression that includes
    /// only the files needed for this crate.
    ///
    /// # Arguments
    /// * `src_var` - The Nix variable name containing the full source (e.g., "src")
    /// * `include_cargo_toml` - Whether to include Cargo.toml (needed for most builds)
    pub fn to_nix_fileset(&self, src_var: &str, include_cargo_toml: bool) -> String {
        let mut files = vec![];

        // Always include the source directory
        files.push(format!(
            "(${{{}}}{})",
            src_var,
            self.relative_source_dir()
                .map(|d| format!("/{d}"))
                .unwrap_or_default()
        ));

        if include_cargo_toml {
            // Include Cargo.toml at crate root
            files.push(format!(
                "(${{{}}}{})",
                src_var,
                self.relative_crate_root()
                    .map(|d| format!("/{d}/Cargo.toml"))
                    .unwrap_or("/Cargo.toml".to_string())
            ));
        }

        format!(
            "lib.fileset.toSource {{\n      root = ${{{}}};\n      fileset = lib.fileset.unions [\n        {}\n      ];\n    }}",
            src_var,
            files.join("\n        ")
        )
    }

    /// Returns the crate root relative to the workspace root, if it can be determined.
    pub fn relative_crate_root(&self) -> Option<&str> {
        // For workspace crates, the path might be like /workspace/crates/foo
        // We want "crates/foo" relative to workspace root
        // This is a heuristic - exact relative path depends on workspace structure
        // For path sources, caller should compute from workspace Cargo.toml
        // For registry/git crates, there is no relative path in the workspace
        None
    }

    /// Returns the source directory (containing .rs files) relative to crate root.
    fn relative_source_dir(&self) -> Option<&str> {
        // Entry point like "src/lib.rs" -> source dir is "src"
        std::path::Path::new(&self.entry_point)
            .parent()
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty())
    }
}

/// Parses a pkg_id into (name, version, source_type).
///
/// Supports two formats:
/// - Old format: `"name version (source)"`
/// - New format: `"source#name@version"`
///
/// Examples:
/// - `"serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)"` (old)
/// - `"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.219"` (new)
/// - `"my-crate 0.1.0 (path+file:///home/user/project)"`
/// - `"path+file:///home/user/project#my-crate@0.1.0"`
fn parse_pkg_id(pkg_id: &str) -> Option<(String, String, SourceType)> {
    // Try new format first: "source#name@version" or "git+url#version"
    if let Some(hash_pos) = pkg_id.find('#') {
        let source_str = &pkg_id[..hash_pos];
        let name_version = &pkg_id[hash_pos + 1..];

        // Parse name@version
        if let Some(at_pos) = name_version.find('@') {
            let name = name_version[..at_pos].to_string();
            let version = name_version[at_pos + 1..].to_string();
            let source = parse_source_type(source_str)?;
            return Some((name, version, source));
        }

        // Git format: "git+url#version" - extract name from URL
        if source_str.starts_with("git+") {
            let version = name_version.to_string();
            // Extract name from git URL (last path segment before any query/fragment)
            let url_part = source_str.strip_prefix("git+").unwrap_or(source_str);
            let url_without_query = url_part.split('?').next().unwrap_or(url_part);
            let name = url_without_query
                .rsplit('/')
                .next()
                .map(|s| s.strip_suffix(".git").unwrap_or(s))
                .unwrap_or("unknown")
                .to_string();
            let source = parse_source_type(source_str)?;
            return Some((name, version, source));
        }
    }

    // Try old format: "name version (source)"
    let paren_start = pkg_id.find('(')?;
    let paren_end = pkg_id.rfind(')')?;

    if paren_start >= paren_end {
        return None;
    }

    let name_version = pkg_id[..paren_start].trim();
    let source_str = &pkg_id[paren_start + 1..paren_end];

    // Split name and version
    let mut parts = name_version.split_whitespace();
    let name = parts.next()?.to_string();
    let version = parts.next()?.to_string();

    // Parse source type
    let source = parse_source_type(source_str)?;

    Some((name, version, source))
}

/// Parses the source type string.
fn parse_source_type(source: &str) -> Option<SourceType> {
    if let Some(path) = source.strip_prefix("path+file://") {
        Some(SourceType::Path {
            path: path.to_string(),
        })
    } else if let Some(rest) = source.strip_prefix("registry+") {
        Some(SourceType::Registry {
            url: rest.to_string(),
        })
    } else if let Some(rest) = source.strip_prefix("git+") {
        // Git URLs can have ?rev=..., ?branch=..., ?tag=..., and #commit
        let (url, commit) = if let Some(hash_pos) = rest.rfind('#') {
            (
                rest[..hash_pos].to_string(),
                Some(rest[hash_pos + 1..].to_string()),
            )
        } else {
            (rest.to_string(), None)
        };

        let (url, reference) = if let Some(q_pos) = url.find('?') {
            let query = &url[q_pos + 1..];
            let base_url = url[..q_pos].to_string();

            // Parse query params for rev/branch/tag
            let reference = query
                .split('&')
                .find_map(|param| {
                    param
                        .strip_prefix("rev=")
                        .or_else(|| param.strip_prefix("branch="))
                        .or_else(|| param.strip_prefix("tag="))
                })
                .map(|s| s.to_string());

            (base_url, reference)
        } else {
            (url, None)
        };

        Some(SourceType::Git {
            url,
            reference,
            commit,
        })
    } else {
        None
    }
}

/// Extracts the crate root and entry point from the source path.
///
/// Given an absolute src_path like `/home/user/project/crates/foo/src/lib.rs`,
/// determines:
/// - crate_root: `/home/user/project/crates/foo`
/// - entry_point: `src/lib.rs`
fn extract_crate_root(src_path: &str, source: &SourceType) -> Option<(String, String)> {
    let path = std::path::Path::new(src_path);

    // For path sources, we can compute from the source URL
    if let SourceType::Path { path: source_path } = source {
        // The source path in pkg_id is the crate root
        let crate_root = source_path.clone();

        // Entry point is src_path relative to crate root
        let entry_point = path
            .strip_prefix(source_path)
            .ok()?
            .to_str()?
            .trim_start_matches('/')
            .to_string();

        return Some((crate_root, entry_point));
    }

    // For registry/git sources, use heuristics based on common patterns
    // Look for "src/lib.rs", "src/main.rs", etc.

    // Find the "src/" component and work backwards
    if let Some(src_pos) = src_path.rfind("/src/") {
        let crate_root = src_path[..src_pos].to_string();
        let entry_point = src_path[src_pos + 1..].to_string();
        return Some((crate_root, entry_point));
    }

    // Fallback: entry point is the file itself, crate root is parent
    let parent = path.parent()?.to_str()?;
    let file_name = path.file_name()?.to_str()?;

    Some((parent.to_string(), file_name.to_string()))
}

/// Utility to convert an absolute path to a workspace-relative path.
///
/// Given a workspace root and an absolute path, returns the relative path.
pub fn make_relative(workspace_root: &str, absolute_path: &str) -> Option<String> {
    let abs = std::path::Path::new(absolute_path);
    let root = std::path::Path::new(workspace_root);

    abs.strip_prefix(root)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string())
}

/// Generates a Nix expression for source remapping.
///
/// Cargo's unit graph contains absolute paths from the machine that ran cargo.
/// In Nix, we need to remap these to paths within the source derivation.
///
/// # Arguments
/// * `src_path` - The absolute path from unit graph (e.g., `/home/user/project/src/lib.rs`)
/// * `workspace_root` - The workspace root path
/// * `nix_src_var` - The Nix variable containing the source (e.g., `src` or `${src}`)
pub fn remap_source_path(src_path: &str, workspace_root: &str, nix_src_var: &str) -> String {
    // First, try remapping to workspace source
    if let Some(relative) = make_relative(workspace_root, src_path) {
        return format!("${{{nix_src_var}}}/{relative}");
    }

    // Try to detect and remap registry crate paths
    // Pattern: /.cargo/registry/src/index.crates.io-xxxxx/cratename-version/...
    if let Some(remapped) = remap_registry_path(src_path) {
        return remapped;
    }

    // Fallback: use the original path (might fail in Nix sandbox)
    src_path.to_string()
}

/// Remaps a unit's manifest directory (CARGO_MANIFEST_DIR) to Nix paths.
///
/// For workspace/local crates: Returns `${src}` or `${src}/relative/path`
/// For registry crates: Returns `${vendorDir}/cratename-version`
///
/// # Arguments
/// * `unit` - The cargo unit to get manifest dir for
/// * `workspace_root` - The workspace root path
/// * `nix_src_var` - Nix variable for workspace source (e.g., "src")
/// * `nix_vendor_var` - Nix variable for vendored crates (e.g., "vendorDir")
pub fn remap_manifest_dir(
    unit: &Unit,
    workspace_root: &str,
    nix_src_var: &str,
    nix_vendor_var: &str,
) -> String {
    let source_loc = SourceLocation::from_unit(unit);

    match source_loc {
        Some(loc) if loc.is_registry() || loc.is_git() => {
            // Registry and git crates: ${vendorDir}/cratename-version
            // Both are vendored by cargo with the same naming scheme
            format!("${{{}}}/{}-{}", nix_vendor_var, loc.name, loc.version)
        }
        Some(loc) if loc.is_path() => {
            // Workspace/local crates: compute relative path from crate_root
            if let Some(relative) = make_relative(workspace_root, &loc.crate_root) {
                if relative.is_empty() {
                    // Root crate - just ${src}
                    format!("${{{}}}", nix_src_var)
                } else {
                    format!("${{{}}}/{}", nix_src_var, relative)
                }
            } else {
                // Fallback to just ${src}
                format!("${{{}}}", nix_src_var)
            }
        }
        _ => {
            // Fallback: just ${src}
            format!("${{{}}}", nix_src_var)
        }
    }
}

/// Attempts to remap a cargo registry path to vendorDir.
///
/// Registry paths look like:
/// `/home/user/.cargo/registry/src/index.crates.io-1234567890abcdef/cratename-1.2.3/src/lib.rs`
///
/// These get remapped to:
/// `${vendorDir}/cratename-1.2.3/src/lib.rs`
fn remap_registry_path(src_path: &str) -> Option<String> {
    // Look for registry/src/ in the path
    let registry_marker = "/registry/src/";
    let registry_pos = src_path.find(registry_marker)?;

    // Skip to after registry/src/
    let after_registry = &src_path[registry_pos + registry_marker.len()..];

    // The next component is the index hash (e.g., index.crates.io-1234567890abcdef)
    // Skip it to get to cratename-version
    let slash_pos = after_registry.find('/')?;
    let remainder = &after_registry[slash_pos + 1..];

    // remainder is now: cratename-version/src/lib.rs
    // We want to remap to: ${vendorDir}/cratename-version/src/lib.rs
    Some(format!("${{vendorDir}}/{remainder}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit_graph::parse_test_unit_graph;

    #[test]
    fn test_parse_path_pkg_id() {
        let (name, version, source) =
            parse_pkg_id("my-crate 0.1.0 (path+file:///home/user/project)").unwrap();

        assert_eq!(name, "my-crate");
        assert_eq!(version, "0.1.0");
        assert!(matches!(source, SourceType::Path { path } if path == "/home/user/project"));
    }

    #[test]
    fn test_parse_registry_pkg_id() {
        let (name, version, source) =
            parse_pkg_id("serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)")
                .unwrap();

        assert_eq!(name, "serde");
        assert_eq!(version, "1.0.219");
        assert!(matches!(
            source,
            SourceType::Registry { url } if url == "https://github.com/rust-lang/crates.io-index"
        ));
    }

    #[test]
    fn test_parse_registry_pkg_id_new_format() {
        // New cargo format: "source#name@version"
        let (name, version, source) =
            parse_pkg_id("registry+https://github.com/rust-lang/crates.io-index#httparse@1.10.1")
                .unwrap();

        assert_eq!(name, "httparse");
        assert_eq!(version, "1.10.1");
        assert!(matches!(
            source,
            SourceType::Registry { url } if url == "https://github.com/rust-lang/crates.io-index"
        ));
    }

    #[test]
    fn test_parse_path_pkg_id_new_format() {
        // New cargo format for path sources
        let (name, version, source) =
            parse_pkg_id("path+file:///home/user/project#my-crate@0.1.0").unwrap();

        assert_eq!(name, "my-crate");
        assert_eq!(version, "0.1.0");
        assert!(matches!(source, SourceType::Path { path } if path == "/home/user/project"));
    }

    #[test]
    fn test_parse_git_pkg_id() {
        let (name, version, source) =
            parse_pkg_id("dep 0.1.0 (git+https://github.com/user/repo?rev=abc123#abc123def)")
                .unwrap();

        assert_eq!(name, "dep");
        assert_eq!(version, "0.1.0");
        match source {
            SourceType::Git {
                url,
                reference,
                commit,
            } => {
                assert_eq!(url, "https://github.com/user/repo");
                assert_eq!(reference, Some("abc123".to_string()));
                assert_eq!(commit, Some("abc123def".to_string()));
            }
            _ => panic!("expected Git source type"),
        }
    }

    #[test]
    fn test_source_location_from_unit() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "my-crate 0.1.0 (path+file:///home/user/project)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "my_crate",
                    "src_path": "/home/user/project/src/lib.rs",
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
        let loc = SourceLocation::from_unit(unit).unwrap();

        assert_eq!(loc.name, "my-crate");
        assert_eq!(loc.version, "0.1.0");
        assert_eq!(loc.crate_root, "/home/user/project");
        assert_eq!(loc.entry_point, "src/lib.rs");
        assert!(loc.is_path());
    }

    #[test]
    fn test_source_location_workspace_crate() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "core 0.1.0 (path+file:///workspace/crates/core)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "core",
                    "src_path": "/workspace/crates/core/src/lib.rs",
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
        let unit = &graph.units[0];
        let loc = SourceLocation::from_unit(unit).unwrap();

        assert_eq!(loc.crate_root, "/workspace/crates/core");
        assert_eq!(loc.entry_point, "src/lib.rs");
    }

    #[test]
    fn test_source_location_bin() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "my-app 0.1.0 (path+file:///home/user/project)",
                "target": {
                    "kind": ["bin"],
                    "crate_types": ["bin"],
                    "name": "my-app",
                    "src_path": "/home/user/project/src/main.rs",
                    "edition": "2021"
                },
                "profile": {"name": "release", "opt_level": "3"},
                "features": ["default"],
                "mode": "build",
                "dependencies": []
            }],
            "roots": [0]
        }"#;

        let graph = parse_test_unit_graph(json);
        let unit = &graph.units[0];
        let loc = SourceLocation::from_unit(unit).unwrap();

        assert_eq!(loc.entry_point, "src/main.rs");
    }

    #[test]
    fn test_make_relative() {
        assert_eq!(
            make_relative("/workspace", "/workspace/crates/foo/src/lib.rs"),
            Some("crates/foo/src/lib.rs".to_string())
        );

        assert_eq!(
            make_relative("/workspace", "/other/path/file.rs"),
            None // Not within workspace
        );
    }

    #[test]
    fn test_remap_source_path() {
        let remapped = remap_source_path("/workspace/crates/foo/src/lib.rs", "/workspace", "src");

        assert_eq!(remapped, "${src}/crates/foo/src/lib.rs");
    }

    #[test]
    fn test_nix_fileset_generation() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "my-crate 0.1.0 (path+file:///home/user/project)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "my_crate",
                    "src_path": "/home/user/project/src/lib.rs",
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
        let loc = SourceLocation::from_unit(unit).unwrap();

        let fileset = loc.to_nix_fileset("src", true);
        assert!(fileset.contains("lib.fileset.toSource"));
        assert!(fileset.contains("lib.fileset.unions"));
    }

    #[test]
    fn test_registry_source_detection() {
        let json = r#"{
            "version": 1,
            "units": [{
                "pkg_id": "serde 1.0.219 (registry+https://github.com/rust-lang/crates.io-index)",
                "target": {
                    "kind": ["lib"],
                    "crate_types": ["lib"],
                    "name": "serde",
                    "src_path": "/home/user/.cargo/registry/src/index.crates.io-1234/serde-1.0.219/src/lib.rs",
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
        let loc = SourceLocation::from_unit(unit).unwrap();

        assert!(loc.is_registry());
        assert!(!loc.is_path());
        assert_eq!(loc.entry_point, "src/lib.rs");
        assert!(loc.crate_root.ends_with("serde-1.0.219"));
    }

    #[test]
    fn test_source_type_predicates() {
        let path_loc = SourceLocation {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            source: SourceType::Path {
                path: "/test".to_string(),
            },
            entry_point: "src/lib.rs".to_string(),
            crate_root: "/test".to_string(),
        };

        assert!(path_loc.is_path());
        assert!(!path_loc.is_registry());
        assert!(!path_loc.is_git());

        let registry_loc = SourceLocation {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            source: SourceType::Registry {
                url: "https://crates.io".to_string(),
            },
            entry_point: "src/lib.rs".to_string(),
            crate_root: "/test".to_string(),
        };

        assert!(!registry_loc.is_path());
        assert!(registry_loc.is_registry());
        assert!(!registry_loc.is_git());

        let git_loc = SourceLocation {
            name: "test".to_string(),
            version: "0.1.0".to_string(),
            source: SourceType::Git {
                url: "https://github.com/test/repo".to_string(),
                reference: None,
                commit: Some("abc123".to_string()),
            },
            entry_point: "src/lib.rs".to_string(),
            crate_root: "/test".to_string(),
        };

        assert!(!git_loc.is_path());
        assert!(!git_loc.is_registry());
        assert!(git_loc.is_git());
    }
}
