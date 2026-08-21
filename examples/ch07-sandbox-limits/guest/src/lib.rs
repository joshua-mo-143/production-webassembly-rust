#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    use std::hint::black_box;

    wit_bindgen::generate!({
        path: "../wit",
        world: "sandboxed-work",
    });

    struct SandboxedWork;

    impl Guest for SandboxedWork {
        fn add(left: u32, right: u32) -> u32 {
            left.saturating_add(right)
        }

        fn burn_fuel(iterations: u64) -> u64 {
            let mut state = black_box(0x9e37_79b9_7f4a_7c15_u64);
            for index in 0..iterations {
                state = state
                    .rotate_left(7)
                    .wrapping_add(black_box(index))
                    .wrapping_mul(0x100_0000_01b3);
            }
            black_box(state)
        }

        fn allocate(mebibytes: u32) -> u32 {
            let bytes = usize::try_from(mebibytes)
                .expect("u32 fits in wasm32 usize")
                .checked_mul(1024 * 1024)
                .expect("requested allocation size fits in usize");
            let allocation = vec![0xa5_u8; bytes];
            black_box(&allocation);
            u32::try_from(allocation.len()).expect("example allocation length fits in u32")
        }

        fn fail_deliberately() {
            panic!("deliberate guest failure");
        }
    }

    export!(SandboxedWork);
}
