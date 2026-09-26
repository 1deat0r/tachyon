//! Unrelated audit crate. It has no dependency on billing or reporting and
//! must remain untouched by any repair.

pub fn audit_id(record: u64) -> String {
    format!("audit-{record:06}")
}

#[cfg(test)]
mod tests {
    use super::audit_id;

    #[test]
    fn ids_are_zero_padded() {
        assert_eq!(audit_id(7), "audit-000007");
    }
}
