//! Invoice arithmetic for the multi-file fixture.

mod tax;

pub use tax::apply_tax;

pub fn invoice_total(amount_cents: i64, basis_points: u32) -> i64 {
    amount_cents + apply_tax(amount_cents, basis_points)
}
