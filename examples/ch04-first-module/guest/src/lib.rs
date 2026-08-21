#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "hello",
    });

    struct Hello;

    impl Guest for Hello {
        fn greet(name: String) -> String {
            format!("Hello, {name}, from WebAssembly!")
        }
    }

    export!(Hello);
}
