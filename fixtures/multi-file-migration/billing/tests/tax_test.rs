use billing::{apply_tax, invoice_total};

#[test]
fn tax_rounds_half_up_on_cents() {
    assert_eq!(apply_tax(199, 1_000), 20, "10% of 199c is 19.9c -> 20c");
    assert_eq!(apply_tax(1, 2_500), 0, "25% of 1c is 0.25c -> 0c");
    assert_eq!(apply_tax(2, 2_500), 1, "25% of 2c is 0.5c -> 1c");
}

#[test]
fn invoice_total_includes_rounded_tax() {
    assert_eq!(invoice_total(199, 1_000), 219);
}
