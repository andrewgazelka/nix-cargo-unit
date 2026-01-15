//! App build script demonstrating env vars and versioning.

fn main() {
    // Emit version info as cfg
    let version = env!("CARGO_PKG_VERSION");
    println!("cargo:rustc-cfg=app_version=\"{version}\"");

    // Emit a feature-like cfg
    println!("cargo:rustc-cfg=feature=\"app_build\"");

    // Generate version constant
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest_path = std::path::PathBuf::from(&out_dir).join("version.rs");

    std::fs::write(
        &dest_path,
        format!(
            r#"
/// App version from build script
pub const APP_VERSION: &str = "{version}";

/// Build-time constant
pub const BUILD_NUMBER: u32 = 1;
"#
        ),
    )
    .expect("failed to write version.rs");

    println!("cargo:rerun-if-changed=build.rs");
}
