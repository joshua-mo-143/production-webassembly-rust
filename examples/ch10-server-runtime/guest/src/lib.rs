#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
#[allow(clippy::same_length_and_capacity)]
mod component {
    use std::hint::black_box;

    const OVERSIZED_RESPONSE_BYTES: usize = 64 * 1024 + 1;

    wit_bindgen::generate!({
        path: "../wit",
        world: "server-handler",
    });

    struct Handler;

    impl Guest for Handler {
        fn handle(request: Request) -> Result<Response, String> {
            let mut state = black_box(0x9e37_79b9_u64);
            for index in 0..request.work {
                state = state.rotate_left(5).wrapping_add(black_box(index));
            }
            black_box(state);

            match request.path.as_str() {
                "/uppercase" => Ok(Response {
                    status: 200,
                    body: request.body.to_uppercase(),
                }),
                "/reject" => Err("request rejected by component".to_owned()),
                "/invalid-status" => Ok(Response {
                    status: 199,
                    body: "invalid status".to_owned(),
                }),
                "/unsafe-control" => Ok(Response {
                    status: 200,
                    body: "unsafe\u{7f}control".to_owned(),
                }),
                "/oversized-output" => Ok(Response {
                    status: 200,
                    body: "x".repeat(OVERSIZED_RESPONSE_BYTES),
                }),
                _ => Ok(Response {
                    status: 404,
                    body: "not found".to_owned(),
                }),
            }
        }
    }

    export!(Handler);
}
