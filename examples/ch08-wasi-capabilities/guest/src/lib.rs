#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "capability-reader",
    });

    struct CapabilityReader;

    impl Guest for CapabilityReader {
        fn read_allowed() -> Result<String, String> {
            std::fs::read_to_string("/data/allowed.txt").map_err(|error| error.to_string())
        }

        fn read_path(path: String) -> Result<String, String> {
            std::fs::read_to_string(path).map_err(|error| error.to_string())
        }

        fn write_allowed_directory() -> Result<(), String> {
            std::fs::write("/data/guest-created.txt", "unexpected")
                .map_err(|error| error.to_string())
        }
    }

    export!(CapabilityReader);
}
