#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    use std::hint::black_box;

    wit_bindgen::generate!({
        path: "../wit",
        world: "agent-tool-logic",
    });

    use exports::book::agent_tools::preprocessing::{
        Guest, Operation, Prepared, Rejection, Request,
    };

    struct ToolLogic;

    impl Guest for ToolLogic {
        fn preprocess(input: Request) -> Result<Prepared, Rejection> {
            match input.operation {
                Operation::Normalize => normalize(&input.text),
                Operation::BurnFuel => {
                    let mut state = black_box(0x9e37_79b9_u64);
                    for index in 0..u64::from(input.amount) {
                        state = state.rotate_left(7).wrapping_add(black_box(index));
                    }
                    black_box(state);
                    normalize(&input.text)
                }
                Operation::GrowMemory => {
                    let bytes = usize::try_from(input.amount).unwrap_or(usize::MAX);
                    let allocation = vec![0_u8; bytes];
                    black_box(allocation);
                    normalize(&input.text)
                }
            }
        }
    }

    fn normalize(text: &str) -> Result<Prepared, Rejection> {
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            return Err(Rejection::Empty);
        }
        if normalized.chars().count() > 256 {
            return Err(Rejection::TooLong(
                normalized.chars().count().try_into().unwrap_or(u32::MAX),
            ));
        }
        let token_estimate = normalized
            .split_whitespace()
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        Ok(Prepared {
            text: normalized,
            token_estimate,
        })
    }

    export!(ToolLogic);
}
