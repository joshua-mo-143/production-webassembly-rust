#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "catalog-component",
    });

    use book::composition::domain::{Availability, LookupError, Product};
    use exports::book::composition::catalog::Guest;

    struct Catalog;

    impl Guest for Catalog {
        fn lookup(sku: String) -> Result<Product, LookupError> {
            if !sku
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(LookupError::InvalidSku);
            }
            match sku.as_str() {
                "RUST-WASM" => Ok(Product {
                    sku,
                    name: "Production WebAssembly with Rust".to_owned(),
                    availability: Availability::InStock(7),
                }),
                "COMP-MODEL" => Ok(Product {
                    sku,
                    name: "Component Model Field Notes".to_owned(),
                    availability: Availability::BackOrder("2026-09-01".to_owned()),
                }),
                "OLD-ABI" => Ok(Product {
                    sku,
                    name: "Raw Pointer ABI Guide".to_owned(),
                    availability: Availability::Discontinued,
                }),
                _ => Err(LookupError::NotFound),
            }
        }
    }

    export!(Catalog);
}
