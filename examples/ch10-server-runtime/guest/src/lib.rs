#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod component {
    use std::hint::black_box;

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
                _ => Ok(Response {
                    status: 404,
                    body: "not found".to_owned(),
                }),
            }
        }
    }

    export!(Handler);
}
