#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "text-plugin",
    });

    struct Plugin;

    impl Guest for Plugin {
        fn metadata() -> Metadata {
            Metadata {
                name: "uppercase".to_owned(),
                version: "1.1.0".to_owned(),
            }
        }

        fn transform(input: String) -> String {
            input.to_uppercase().replace("PLUGIN", "PLUGIN 1.1")
        }
    }

    export!(Plugin);
}
