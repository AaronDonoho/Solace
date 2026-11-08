#[test]
fn encode_pads_and_decode_round_trips() {
    assert_eq!(crate::base64::encode(b"Man"), "TWFu");
    assert_eq!(crate::base64::encode(b"Ma"), "TWE=");
    assert_eq!(crate::base64::encode(b"M"), "TQ==");
    let cases: [&[u8]; 5] = [b"", b"M", b"Ma", b"Man", b"Many"];
    for bytes in cases {
        assert_eq!(crate::base64::decode(&crate::base64::encode(bytes)), Some(bytes.to_vec()));
    }
}

#[test]
fn decode_refuses_text_that_is_not_base64() {
    for text in ["TWE", "!!!!", "AA=A"] {
        assert_eq!(crate::base64::decode(text), None, "{text}");
    }
}
