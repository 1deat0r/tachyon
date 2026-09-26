/// Tax for an invoice line. Amounts are integer cents; rates are basis
/// points (10_000 = 100%).
pub fn apply_tax(amount_cents: i64, basis_points: u32) -> i64 {
    (amount_cents * i64::from(basis_points) + 5_000) / 10_000
}
