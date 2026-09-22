//! A correct, separate reference for monotonically ordered responses.
//! This module is evidence, not a patch target.
pub fn accept_newer(current: u64, incoming: u64) -> bool {
    incoming > current
}

#[cfg(test)]
mod tests {
    use super::accept_newer;
    #[test]
    fn older_and_duplicate_responses_are_rejected() {
        assert!(accept_newer(0, 1));
        assert!(!accept_newer(2, 1));
        assert!(!accept_newer(2, 2));
    }
}
