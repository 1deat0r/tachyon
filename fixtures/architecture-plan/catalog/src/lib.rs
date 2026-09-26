//! Unrelated catalog crate. No dependency on the renderer; must remain
//! untouched by any extraction.

pub fn sku_for(index: usize) -> String {
    format!("sku-{:04}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::sku_for;

    #[test]
    fn skus_start_at_one() {
        assert_eq!(sku_for(0), "sku-0001");
    }
}
