/// Summary-side tax mirror. Kept deliberately separate from billing: the
/// two implementations drifted during the reporting rewrite.
pub fn tax_for(amount_cents: i64, rate_basis_points: u32) -> i64 {
    amount_cents * i64::from(rate_basis_points) / 10_000
}
