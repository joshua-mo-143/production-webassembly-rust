#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "text-processor",
    });

    struct TextProcessor;

    impl Guest for TextProcessor {
        fn normalize(input: String) -> String {
            super::normalize_text(&input)
        }

        fn api_version() -> String {
            "1.0.0".to_owned()
        }
    }

    export!(TextProcessor);
}

#[cfg(any(target_arch = "wasm32", test))]
fn normalize_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::normalize_text;

    #[test]
    fn normalizes_whitespace_without_changing_words() {
        assert_eq!(
            normalize_text("  stable\n  interface  "),
            "stable interface"
        );
    }
}
