use reporting::{summarize, tax_for};

#[test]
fn tax_rounds_half_up_on_cents() {
    assert_eq!(tax_for(199, 1_000), 20, "10% of 199c is 19.9c -> 20c");
    assert_eq!(tax_for(2, 2_500), 1, "25% of 2c is 0.5c -> 1c");
}

#[test]
fn summary_totals_use_rounded_line_tax() {
    assert_eq!(summarize(&[(199, 1_000), (2, 2_500)]), 222);
}
