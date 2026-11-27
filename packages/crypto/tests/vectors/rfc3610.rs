//! RFC 3610 §8, the 24 CCM packet vectors.

use orbe_crypto::aes::AesKey;
use orbe_crypto::ccm::{open, seal};

use super::decode;

struct Vector {
    key: &'static str,
    nonce: &'static str,
    /// Cleartext header octets at the start of `input`, the associated data.
    header: usize,
    /// M, the tag length.
    tag: usize,
    input: &'static str,
    output: &'static str,
}

const VECTORS: [Vector; 24] = [
    // Packet Vector #1
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000003020100a0a1a2a3a4a5",
        header: 8,
        tag: 8,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e",
        output: "0001020304050607588c979a61c663d2f066d0c2c0f989806d5f6b61dac38417e8d12cfdf926e0",
    },
    // Packet Vector #2
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000004030201a0a1a2a3a4a5",
        header: 8,
        tag: 8,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        output: "000102030405060772c91a36e135f8cf291ca894085c87e3cc15c439c9e43a3ba091d56e10400916",
    },
    // Packet Vector #3
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000005040302a0a1a2a3a4a5",
        header: 8,
        tag: 8,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
        output: "000102030405060751b1e5f44a197d1da46b0f8e2d282ae871e838bb64da8596574adaa76fbd9fb0c5",
    },
    // Packet Vector #4
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000006050403a0a1a2a3a4a5",
        header: 12,
        tag: 8,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e",
        output: "000102030405060708090a0ba28c6865939a9a79faaa5c4c2a9d4a91cdac8c96c861b9c9e61ef1",
    },
    // Packet Vector #5
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000007060504a0a1a2a3a4a5",
        header: 12,
        tag: 8,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        output: "000102030405060708090a0bdcf1fb7b5d9e23fb9d4e131253658ad86ebdca3e51e83f077d9c2d93",
    },
    // Packet Vector #6
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000008070605a0a1a2a3a4a5",
        header: 12,
        tag: 8,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
        output: "000102030405060708090a0b6fc1b011f006568b5171a42d953d469b2570a4bd87405a0443ac91cb94",
    },
    // Packet Vector #7
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "00000009080706a0a1a2a3a4a5",
        header: 8,
        tag: 10,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e",
        output: "00010203040506070135d1b2c95f41d5d1d4fec185d166b8094e999dfed96c048c56602c97acbb7490",
    },
    // Packet Vector #8
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "0000000a090807a0a1a2a3a4a5",
        header: 8,
        tag: 10,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        output: "00010203040506077b75399ac0831dd2f0bbd75879a2fd8f6cae6b6cd9b7db24c17b4433f434963f34b4",
    },
    // Packet Vector #9
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "0000000b0a0908a0a1a2a3a4a5",
        header: 8,
        tag: 10,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
        output: "000102030405060782531a60cc24945a4b8279181ab5c84df21ce7f9b73f42e197ea9c07e56b5eb17e5f4e",
    },
    // Packet Vector #10
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "0000000c0b0a09a0a1a2a3a4a5",
        header: 12,
        tag: 10,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e",
        output: "000102030405060708090a0b07342594157785152b074098330abb141b947b566aa9406b4d999988dd",
    },
    // Packet Vector #11
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "0000000d0c0b0aa0a1a2a3a4a5",
        header: 12,
        tag: 10,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        output: "000102030405060708090a0b676bb20380b0e301e8ab79590a396da78b834934f53aa2e9107a8b6c022c",
    },
    // Packet Vector #12
    Vector {
        key: "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf",
        nonce: "0000000e0d0c0ba0a1a2a3a4a5",
        header: 12,
        tag: 10,
        input: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
        output: "000102030405060708090a0bc0ffa0d6f05bdb67f24d43a4338d2aa4bed7b20e43cd1aa31662e7ad65d6db",
    },
    // Packet Vector #13
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "00412b4ea9cdbe3c9696766cfa",
        header: 8,
        tag: 8,
        input: "0be1a88bace018b108e8cf97d820ea258460e96ad9cf5289054d895ceac47c",
        output: "0be1a88bace018b14cb97f86a2a4689a877947ab8091ef5386a6ffbdd080f8e78cf7cb0cddd7b3",
    },
    // Packet Vector #14
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "0033568ef7b2633c9696766cfa",
        header: 8,
        tag: 8,
        input: "63018f76dc8a1bcb9020ea6f91bdd85afa0039ba4baff9bfb79c7028949cd0ec",
        output: "63018f76dc8a1bcb4ccb1e7ca981befaa0726c55d378061298c85c92814abc33c52ee81d7d77c08a",
    },
    // Packet Vector #15
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "00103fe41336713c9696766cfa",
        header: 8,
        tag: 8,
        input: "aa6cfa36cae86b40b916e0eacc1c00d7dcec68ec0b3bbb1a02de8a2d1aa346132e",
        output: "aa6cfa36cae86b40b1d23a2220ddc0ac900d9aa03c61fcf4a559a4417767089708a776796edb723506",
    },
    // Packet Vector #16
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "00764c63b8058e3c9696766cfa",
        header: 12,
        tag: 8,
        input: "d0d0735c531e1becf049c24412daac5630efa5396f770ce1a66b21f7b2101c",
        output: "d0d0735c531e1becf049c24414d253c3967b70609b7cbb7c499160283245269a6f49975bcadeaf",
    },
    // Packet Vector #17
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "00f8b678094e3b3c9696766cfa",
        header: 12,
        tag: 8,
        input: "77b60f011c03e1525899bcaee88b6a46c78d63e52eb8c546efb5de6f75e9cc0d",
        output: "77b60f011c03e1525899bcae5545ff1a085ee2efbf52b2e04bee1e2336c73e3f762c0c7744fe7e3c",
    },
    // Packet Vector #18
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "00d560912d3f703c9696766cfa",
        header: 12,
        tag: 8,
        input: "cd9044d2b71fdb8120ea60c06435acbafb11a82e2f071d7ca4a5ebd93a803ba87f",
        output: "cd9044d2b71fdb8120ea60c0009769ecabdf48625594c59251e6035722675e04c847099e5ae0704551",
    },
    // Packet Vector #19
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "0042fff8f1951c3c9696766cfa",
        header: 8,
        tag: 10,
        input: "d85bc7e69f944fb88a19b950bcf71a018e5e6701c91787659809d67dbedd18",
        output: "d85bc7e69f944fb8bc218daa947427b6db386a99ac1aef23ade0b52939cb6a637cf9bec2408897c6ba",
    },
    // Packet Vector #20
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "00920f40e56cdc3c9696766cfa",
        header: 8,
        tag: 10,
        input: "74a0ebc9069f5b371761433c37c5a35fc1f39f406302eb907c6163be38c98437",
        output: "74a0ebc9069f5b375810e6fd25874022e80361a478e3e9cf484ab04f447efff6f0a477cc2fc9bf548944",
    },
    // Packet Vector #21
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "0027ca0c7120bc3c9696766cfa",
        header: 8,
        tag: 10,
        input: "44a3aa3aae6475caa434a8e58500c6e41530538862d686ea9e81301b5ae4226bfa",
        output: "44a3aa3aae6475caf2beed7bc5098e83feb5b31608f8e29c38819a89c8e776f1544d4151a4ed3a8b87b9ce",
    },
    // Packet Vector #22
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "005b8ccbcd9af83c9696766cfa",
        header: 12,
        tag: 10,
        input: "ec46bb63b02520c33c49fd70b96b49e21d621741632875db7f6c9243d2d7c2",
        output: "ec46bb63b02520c33c49fd7031d750a09da3ed7fddd49a2032aabf17ec8ebf7d22c8088c666be5c197",
    },
    // Packet Vector #23
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "003ebe94044b9a3c9696766cfa",
        header: 12,
        tag: 10,
        input: "47a65ac78b3d594227e85e71e2fcfbb880442c731bf95167c8ffd7895e337076",
        output: "47a65ac78b3d594227e85e71e882f1dbd38ce3eda7c23f04dd65071eb41342acdf7e00dccec7ae52987d",
    },
    // Packet Vector #24
    Vector {
        key: "d7828d13b2b0bdc325a76236df93cc6b",
        nonce: "008d493b30ae8b3c9696766cfa",
        header: 12,
        tag: 10,
        input: "6e37a6ef546d955d34ab6059abf21c0b02feb88f856df4a37381bce3cc128517d4",
        output: "6e37a6ef546d955d34ab6059f32905b88a641b04b9c9ffb58cc390900f3da12ab16dce9e82efa16da62059",
    },
];

#[test]
fn ccm_matches_rfc_3610() {
    for (i, v) in VECTORS.iter().enumerate() {
        let key = AesKey::new(&decode(v.key)).unwrap();
        let nonce = decode(v.nonce);
        let input = decode(v.input);
        let output = decode(v.output);
        let (aad, plaintext) = input.split_at(v.header);
        assert_eq!(&output[..v.header], aad);
        let sealed = &output[v.header..];
        assert_eq!(
            seal(&key, &nonce, aad, plaintext, v.tag),
            sealed,
            "seal #{}",
            i + 1
        );
        assert_eq!(
            open(&key, &nonce, aad, sealed, v.tag).as_deref(),
            Some(plaintext),
            "open #{}",
            i + 1
        );
    }
}

#[test]
fn ccm_refuses_a_changed_byte() {
    let v = &VECTORS[0];
    let key = AesKey::new(&decode(v.key)).unwrap();
    let nonce = decode(v.nonce);
    let output = decode(v.output);
    let (aad, sealed) = output.split_at(v.header);
    for i in 0..sealed.len() {
        let mut changed = sealed.to_vec();
        changed[i] ^= 0x80;
        assert_eq!(open(&key, &nonce, aad, &changed, v.tag), None, "byte {i}");
    }
    let mut aad = aad.to_vec();
    aad[0] ^= 1;
    assert_eq!(open(&key, &nonce, &aad, sealed, v.tag), None, "header");
    assert_eq!(open(&key, &nonce, &aad, &sealed[..7], v.tag), None, "short");
}
