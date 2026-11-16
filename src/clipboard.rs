use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

/// Decides whether a timed clear may run. Only an exact match is cleared, so a
/// value the user copied from somewhere else after our copy is never clobbered.
pub fn should_clear(current: Option<&str>, expected: &str) -> bool {
    !expected.is_empty() && current == Some(expected)
}

/// Tracks the most recent value this application put on the clipboard.
///
/// Notes:
/// - We compare the clipboard contents before clearing.
/// - Clipboard managers and receiving applications may still retain history;
///   clearing is best-effort, not secure erasure.
#[derive(Clone, Default)]
pub struct ClipboardGuard {
    expected: Arc<Mutex<Option<Zeroizing<String>>>>,
}

impl ClipboardGuard {
    pub fn remember(&self, value: &str) {
        if let Ok(mut slot) = self.expected.lock() {
            // Dropping the previous Zeroizing value zeroes it.
            *slot = Some(Zeroizing::new(value.to_string()));
        }
    }

    pub fn matches(&self, current: Option<&str>) -> bool {
        let Ok(slot) = self.expected.lock() else {
            return false;
        };
        match (current, slot.as_deref()) {
            (Some(current), Some(expected)) => should_clear(Some(current), expected),
            _ => false,
        }
    }

    /// Forget only if the tracked value is still `expected`.
    pub fn forget_if_matches(&self, expected: &str) {
        if let Ok(mut slot) = self.expected.lock() {
            if slot.as_deref().map(String::as_str) == Some(expected) {
                *slot = None;
            }
        }
    }

    pub fn forget(&self) {
        if let Ok(mut slot) = self.expected.lock() {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_clears_exact_match() {
        assert!(should_clear(Some("secret"), "secret"));
        assert!(!should_clear(Some("something else"), "secret"));
        assert!(!should_clear(None, "secret"));
        assert!(!should_clear(Some(""), ""));
    }

    #[test]
    fn guard_tracks_latest_value() {
        let guard = ClipboardGuard::default();
        guard.remember("one");
        assert!(guard.matches(Some("one")));
        guard.remember("two");
        assert!(!guard.matches(Some("one")));
        guard.forget_if_matches("one");
        assert!(guard.matches(Some("two")));
        guard.forget();
        assert!(!guard.matches(Some("two")));
    }
}
