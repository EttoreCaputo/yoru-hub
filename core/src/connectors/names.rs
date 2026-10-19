//! Qualified tool names: `<connector id>__<tool>`, so the agent sees one
//! flat list of tools across connectors.
//!
//! LLM APIs take tool names matching `^[a-zA-Z0-9_-]{1,64}$`, while an MCP
//! tool's name may be longer or use other characters (`.` and `/` are
//! common). Those are replaced with `_`, and a name that had to change or be
//! cut gets a short hash of the original, so two tools never end up with
//! the same name. The mapping is one-way: resolving goes through the table of
//! the tools listed ([`super::Connectors::resolve`]), never by splitting.

/// The longest name LLM APIs accept.
pub const MAX_LEN: usize = 64;
const SEPARATOR: &str = "__";

/// The qualified name of `tool` on connector `id` (a valid id, see
/// [`super::config::is_valid_id`]).
pub fn qualify(id: &str, tool: &str) -> String {
    let clean: String = tool.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let plain = format!("{id}{SEPARATOR}{clean}");
    if clean == tool && !tool.is_empty() && plain.len() <= MAX_LEN {
        return plain;
    }
    // Changed or too long: keep what fits and make it unique with a hash.
    let hash = format!("{:08x}", fnv1a(tool.as_bytes()) as u32);
    let room = MAX_LEN - id.len() - SEPARATOR.len() - 1 - hash.len();
    let head: String = clean.chars().take(room).collect();
    format!("{id}{SEPARATOR}{head}_{hash}")
}

/// Whether `name` is a tool name LLM APIs accept.
pub fn is_valid(name: &str) -> bool {
    (1..=MAX_LEN).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// FNV-1a: stable across runs and builds, unlike `DefaultHasher`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_stay_readable() {
        assert_eq!(qualify("gmail", "search_messages"), "gmail__search_messages");
        assert_eq!(qualify("google-calendar", "list-events"), "google-calendar__list-events");
    }

    #[test]
    fn odd_names_become_valid_and_distinct() {
        let a = qualify("notion", "pages.search");
        let b = qualify("notion", "pages/search");
        assert!(is_valid(&a) && is_valid(&b), "{a} {b}");
        assert_ne!(a, b);
        assert!(a.starts_with("notion__pages_search_"), "{a}");
        // The same name always gives the same qualified name.
        assert_eq!(a, qualify("notion", "pages.search"));
    }

    #[test]
    fn long_names_are_cut_to_64() {
        let long = "x".repeat(100);
        let q = qualify("a-long-connector-id-here", &long);
        assert_eq!(q.len(), MAX_LEN);
        assert!(is_valid(&q));
        assert_ne!(q, qualify("a-long-connector-id-here", &"x".repeat(99)));
    }

    #[test]
    fn an_empty_name_still_qualifies() {
        assert!(is_valid(&qualify("x", "")));
    }

    #[test]
    fn validity() {
        assert!(is_valid("a__b-c_1"));
        assert!(!is_valid(""));
        assert!(!is_valid("a.b"));
        assert!(!is_valid(&"a".repeat(65)));
    }
}
