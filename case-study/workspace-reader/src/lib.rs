#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "workspace-reader-tool",
    });

    use exports::book::secure_agent_tools::workspace_reader::{Document, Guest};

    struct WorkspaceReader;

    impl Guest for WorkspaceReader {
        fn read(path: String) -> Result<Document, String> {
            let guest_path = format!("/workspace/{path}");
            let contents = std::fs::read_to_string(guest_path)
                .map_err(|_| "document unavailable".to_owned())?;
            Ok(Document { path, contents })
        }
    }

    export!(WorkspaceReader);
}
