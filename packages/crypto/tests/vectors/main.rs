//! Known-answer tests, one module per source. Each vector is copied from the document its
//! module names.

mod fips;
mod ieee80211;
mod rfc2202;
mod rfc3394;
mod rfc3610;
mod rfc6070;

fn decode(s: &str) -> Vec<u8> {
    assert_eq!(s.len() % 2, 0);
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn decode16(s: &str) -> [u8; 16] {
    decode(s).try_into().unwrap()
}
