//! Unrelated code: affected verification should not select this crate.
pub fn label() -> &'static str {
    "unrelated"
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_unrelated_check() {
        assert_eq!(super::label(), "unrelated");
    }
}
