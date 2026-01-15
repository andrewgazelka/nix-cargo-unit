//! Build script that emits cfg flags and generates code.
//!
//! This demonstrates:
//! - `cargo:rustc-cfg` directives
//! - `OUT_DIR` code generation

fn main() {
    // Declare expected cfg values for check-cfg lint
    println!("cargo::rustc-check-cfg=cfg(has_build_script)");
    println!("cargo::rustc-check-cfg=cfg(build_profile, values(\"release\", \"dev\"))");

    // Emit a cfg flag that can be checked with #[cfg(has_build_script)]
    println!("cargo:rustc-cfg=has_build_script");

    // Emit a cfg flag with a value
    println!("cargo:rustc-cfg=build_profile=\"release\"");

    // Generate code in OUT_DIR
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest_path = std::path::PathBuf::from(&out_dir).join("generated.rs");

    std::fs::write(
        &dest_path,
        r#"
/// Generated constant from build.rs
pub const BUILD_TIME_VALUE: u32 = 42;

/// Generated function
pub fn generated_greeting() -> &'static str {
    "Hello from generated code!"
}
"#,
    )
    .expect("failed to write generated.rs");

    println!("cargo:rerun-if-changed=build.rs");
}
