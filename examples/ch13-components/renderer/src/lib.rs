#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "renderer-component",
    });

    use book::composition::domain::{Availability, Product};
    use exports::book::composition::renderer::Guest;

    struct Renderer;

    impl Guest for Renderer {
        fn render(product: Product) -> Result<String, String> {
            if product.name.chars().any(char::is_control) {
                return Err("product name contains control characters".to_owned());
            }
            let status = match product.availability {
                Availability::InStock(count) => format!("{count} in stock"),
                Availability::BackOrder(date) => format!("back-order until {date}"),
                Availability::Discontinued => "discontinued".to_owned(),
            };
            Ok(format!("{} [{}]: {status}", product.name, product.sku))
        }
    }

    export!(Renderer);
}
