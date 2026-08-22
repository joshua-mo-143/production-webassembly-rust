#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
#[allow(clippy::same_length_and_capacity)]
mod component {
    const OVERSIZED_OUTPUT_BYTES: usize = 64 * 1024 + 1;

    wit_bindgen::generate!({
        path: "../wit",
        world: "text-plugin",
    });

    struct Plugin;

    impl Guest for Plugin {
        fn metadata() -> Metadata {
            Metadata {
                name: "uppercase".to_owned(),
                version: "1.0.0".to_owned(),
            }
        }

        fn transform(input: String) -> String {
            assert_ne!(input, "trap", "deliberate third-party plugin failure");
            match input.as_str() {
                "unsafe-control" => "unsafe\u{7f}control".to_owned(),
                "oversized-output" => "x".repeat(OVERSIZED_OUTPUT_BYTES),
                _ => input.to_uppercase(),
            }
        }
    }

    export!(Plugin);
}
