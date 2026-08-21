#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code, clippy::same_length_and_capacity)]
mod component {
    wit_bindgen::generate!({
        path: "../wit",
        world: "analyzer",
    });

    struct Analyzer;

    impl Guest for Analyzer {
        fn sum_records(samples: Vec<Sample>) -> f64 {
            samples.iter().map(|sample| sample.value).sum()
        }

        fn sum_packed(samples: Vec<u8>) -> Result<f64, String> {
            super::decode_and_sum(&samples)
        }
    }

    export!(Analyzer);
}

#[cfg(any(target_arch = "wasm32", test))]
fn decode_and_sum(bytes: &[u8]) -> Result<f64, String> {
    if !bytes.len().is_multiple_of(16) {
        return Err("packed input length must be a multiple of 16 bytes".to_owned());
    }

    Ok(bytes
        .chunks_exact(16)
        .map(|chunk| {
            let value_bytes: [u8; 8] = chunk[8..16].try_into().expect("chunk is 16 bytes");
            f64::from_le_bytes(value_bytes)
        })
        .sum())
}

#[cfg(test)]
mod tests {
    use super::decode_and_sum;

    #[test]
    fn sums_values_from_fixed_width_records() {
        let mut packed = Vec::new();
        packed.extend_from_slice(&1_u64.to_le_bytes());
        packed.extend_from_slice(&2.5_f64.to_le_bytes());
        packed.extend_from_slice(&2_u64.to_le_bytes());
        packed.extend_from_slice(&3.5_f64.to_le_bytes());

        assert_eq!(decode_and_sum(&packed), Ok(6.0));
        assert!(decode_and_sum(&[0; 15]).is_err());
    }
}
