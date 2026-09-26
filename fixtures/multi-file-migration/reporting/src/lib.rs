//! Reporting totals for the multi-file fixture.

mod tax;

pub use tax::tax_for;

pub fn summarize(lines: &[(i64, u32)]) -> i64 {
    lines.iter()
        .map(|(amount, rate)| *amount + tax_for(*amount, *rate))
        .sum()
}
