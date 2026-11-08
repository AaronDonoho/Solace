//! Base64 encoding and decoding, for bytes stored inside JSON text.
#[cfg(test)]
#[path = "base64_tests.rs"]
mod tests;



const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[((first >> 2) & 63) as usize] as char);
        out.push(TABLE[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[(((second & 15) << 2) | (third >> 6)) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(third & 63) as usize] as char } else { '=' });
    }
    out
}

pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let first = value(chunk[0])?;
        let second = value(chunk[1])?;
        let third = if chunk[2] == b'=' { 0 } else { value(chunk[2])? };
        let fourth = if chunk[3] == b'=' { 0 } else { value(chunk[3])? };
        if chunk[2] == b'=' && chunk[3] != b'=' {
            return None;
        }
        out.push(((first << 2) | (second >> 4)) as u8);
        if chunk[2] != b'=' {
            out.push(((second << 4) | (third >> 2)) as u8);
        }
        if chunk[3] != b'=' {
            out.push(((third << 6) | fourth) as u8);
        }
    }
    Some(out)
}

fn value(byte: u8) -> Option<u32> {
    Some(match byte {
        b'A'..=b'Z' => (byte - b'A') as u32,
        b'a'..=b'z' => (byte - b'a' + 26) as u32,
        b'0'..=b'9' => (byte - b'0' + 52) as u32,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    })
}
