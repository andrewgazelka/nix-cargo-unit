//! Proc-macro crate demonstrating host compilation.
//!
//! This crate provides:
//! - A derive macro that generates a `describe` method
//! - A function-like macro that generates greeting code

extern crate proc_macro;

use proc_macro::TokenStream;

/// Derive macro that generates a `describe` method for structs.
///
/// # Example
///
/// ```ignore
/// #[derive(Describe)]
/// struct MyStruct {
///     field: i32,
/// }
///
/// let s = MyStruct { field: 42 };
/// assert_eq!(s.describe(), "MyStruct");
/// ```
#[proc_macro_derive(Describe)]
pub fn derive_describe(input: TokenStream) -> TokenStream {
    let input_str = input.to_string();

    // Simple parsing - find the struct name
    let struct_name = input_str
        .split_whitespace()
        .skip_while(|s| *s != "struct")
        .nth(1)
        .map(|s| s.trim_end_matches('{').trim_end_matches('<'))
        .unwrap_or("Unknown");

    // Generate the impl
    let output = format!(
        r#"
impl {struct_name} {{
    pub fn describe(&self) -> &'static str {{
        "{struct_name}"
    }}
}}
"#
    );

    output.parse().unwrap()
}

/// Function-like macro that generates a greeting function.
///
/// # Example
///
/// ```ignore
/// make_greeter!(hello);
/// assert_eq!(hello(), "Hello from hello!");
/// ```
#[proc_macro]
pub fn make_greeter(input: TokenStream) -> TokenStream {
    let name = input.to_string().trim().to_string();

    let output = format!(
        r#"
fn {name}() -> &'static str {{
    "Hello from {name}!"
}}
"#
    );

    output.parse().unwrap()
}

/// Attribute macro that adds tracing to a function (simplified).
///
/// # Example
///
/// ```ignore
/// #[traced]
/// fn my_function() { ... }
/// ```
#[proc_macro_attribute]
pub fn traced(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_str = item.to_string();

    // Find function name for the trace message
    let fn_name = item_str
        .split_whitespace()
        .skip_while(|s| *s != "fn")
        .nth(1)
        .map(|s| s.split('(').next().unwrap_or(s))
        .unwrap_or("unknown");

    // For simplicity, just pass through the original item
    // In a real implementation, we'd wrap the function body
    format!(
        r#"
// Traced: {fn_name}
{item_str}
"#
    )
    .parse()
    .unwrap()
}
