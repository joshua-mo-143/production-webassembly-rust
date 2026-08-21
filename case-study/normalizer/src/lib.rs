#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    use std::hint::black_box;

    wit_bindgen::generate!({
        path: "../wit",
        world: "normalizer-tool",
    });

    use exports::book::secure_agent_tools::normalizer::{Guest, Normalized, Operation, Request};

    struct Normalizer;

    impl Guest for Normalizer {
        fn normalize(request: Request) -> Result<Normalized, String> {
            match request.operation {
                Operation::Normalize => normalized(&request.text),
                Operation::BurnFuel => {
                    let mut state = black_box(0x9e37_79b9_u64);
                    for index in 0..u64::from(request.amount) {
                        state = state.rotate_left(7).wrapping_add(black_box(index));
                    }
                    black_box(state);
                    normalized(&request.text)
                }
                Operation::GrowMemory => {
                    let size = usize::try_from(request.amount).unwrap_or(usize::MAX);
                    black_box(vec![0_u8; size]);
                    normalized(&request.text)
                }
                Operation::InvalidOutput => Ok(Normalized {
                    text: "\u{0007}".repeat(300),
                    word_count: 99,
                }),
            }
        }
    }

    fn normalized(text: &str) -> Result<Normalized, String> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || text.chars().count() > 256 {
            return Err("invalid text".to_owned());
        }
        let word_count = text
            .split_whitespace()
            .count()
            .try_into()
            .map_err(|_| "invalid text".to_owned())?;
        Ok(Normalized { text, word_count })
    }

    export!(Normalizer);
}
