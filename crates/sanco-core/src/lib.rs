//! Sanco core: database, sync and business rules. No Tauri dependency, so
//! everything here is testable with `cargo test -p sanco-core`.

/// Greeting used by the M0 end-to-end check.
pub fn greeting(name: &str) -> String {
    let name = name.trim();
    let name = if name.is_empty() { "there" } else { name };
    format!("Hello, {name}! Sanco core is connected.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_trims_and_defaults() {
        assert_eq!(greeting("  Ana "), "Hello, Ana! Sanco core is connected.");
        assert_eq!(greeting(""), "Hello, there! Sanco core is connected.");
    }
}
