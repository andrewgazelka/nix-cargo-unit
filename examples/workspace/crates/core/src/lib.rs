//! Core library demonstrating build.rs integration.
//!
//! This crate:
//! - Uses cfg flags from build.rs
//! - Includes generated code from OUT_DIR

// Include the generated code
include!(concat!(env!("OUT_DIR"), "/generated.rs"));

/// A simple function that uses the build script cfg.
#[cfg(has_build_script)]
pub fn with_build_script() -> &'static str {
    "Build script was run!"
}

/// Fallback if build script didn't run (shouldn't happen).
#[cfg(not(has_build_script))]
pub fn with_build_script() -> &'static str {
    "Build script was NOT run!"
}

/// Returns the build-time constant.
pub fn get_build_value() -> u32 {
    BUILD_TIME_VALUE
}

/// A core type that can be enhanced by the proc-macro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn new(name: impl Into<String>, value: i32) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_script_ran() {
        assert_eq!(with_build_script(), "Build script was run!");
    }

    #[test]
    fn test_generated_value() {
        assert_eq!(get_build_value(), 42);
    }

    #[test]
    fn test_generated_greeting() {
        assert_eq!(generated_greeting(), "Hello from generated code!");
    }
}
