//! Example application that uses:
//! - Core library (with build.rs)
//! - Proc-macros (compiled for host)
//! - Its own build.rs

// Include generated version info
include!(concat!(env!("OUT_DIR"), "/version.rs"));

// Use the derive macro from example-macros
#[derive(example_macros::Describe)]
#[allow(dead_code)] // Fields are used via generated describe() method
struct AppConfig {
    name: String,
    debug: bool,
}

// Use the function-like macro
example_macros::make_greeter!(app_greeter);

// Use the attribute macro
#[example_macros::traced]
fn do_work() {
    println!("Doing important work...");
}

fn main() {
    println!("=== nix-cargo-unit Example App ===");
    println!();

    // Show version from build.rs
    println!("App Version: {APP_VERSION}");
    println!("Build Number: {BUILD_NUMBER}");
    println!();

    // Use core library
    println!("Core Library:");
    println!("  Build script status: {}", example_core::with_build_script());
    println!(
        "  Generated value: {}",
        example_core::get_build_value()
    );
    println!(
        "  Generated greeting: {}",
        example_core::generated_greeting()
    );
    println!();

    // Use proc-macros
    let config = AppConfig {
        name: "test".to_string(),
        debug: true,
    };
    println!("Proc-Macros:");
    println!("  Describe derive: {}", config.describe());
    println!("  make_greeter!: {}", app_greeter());
    println!();

    // Use core Config type
    let core_config = example_core::Config::new("example", 100);
    println!("Core Config: {:?}", core_config);
    println!();

    // Run traced function
    do_work();

    println!();
    println!("=== All features working! ===");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert_eq!(APP_VERSION, "0.1.0");
    }

    #[test]
    fn test_describe() {
        let config = AppConfig {
            name: "test".to_string(),
            debug: false,
        };
        assert_eq!(config.describe(), "AppConfig");
    }

    #[test]
    fn test_greeter() {
        assert_eq!(app_greeter(), "Hello from app_greeter!");
    }

    #[test]
    fn test_core_integration() {
        assert_eq!(example_core::get_build_value(), 42);
    }
}
