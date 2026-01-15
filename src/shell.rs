//! Shell escaping utilities.

/// Quotes a shell argument if it contains special characters.
///
/// Arguments containing spaces, quotes, or dollar signs are wrapped in single
/// quotes with internal single quotes escaped as `'\''`.
pub fn quote_arg(arg: &str) -> std::borrow::Cow<'_, str> {
    if arg.contains(' ') || arg.contains('"') || arg.contains('$') || arg.contains('\'') {
        std::borrow::Cow::Owned(format!("'{}'", arg.replace('\'', "'\\''")))
    } else {
        std::borrow::Cow::Borrowed(arg)
    }
}
