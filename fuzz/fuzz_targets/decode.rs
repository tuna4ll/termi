//! Whatever bytes a file holds, opening and saving it gives back the same
//! bytes — or refuses it as binary — and never panics.

#![no_main]

use libfuzzer_sys::fuzz_target;
use termi::filesystem::{decode, encode};

fuzz_target!(|data: &[u8]| {
    if let Ok((text, encoding)) = decode(data) {
        let bytes = encode(&text, encoding).expect("decoded text encodes again");
        assert_eq!(bytes, data);
    }
});
