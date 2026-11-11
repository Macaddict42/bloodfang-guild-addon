//! Counts for the upload panel, read from an accepted `!BF:` envelope.
//! The body is LibDeflate EncodeForPrint of raw deflate JSON (export-format.md).

use flate2::read::DeflateDecoder;
use serde_json::Value;
use std::io::Read;

const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789()";
const MAX_STRING: usize = 256 * 1024;
const MAX_DECODED: usize = 512 * 1024;

/// Number of units in one accepted envelope: ranks are handled elsewhere;
/// guild bank is the number of `items` entries across tabs; recipes is the
/// length of the `recipes` list.
pub fn envelope_count(raw: &str) -> Option<(&'static str, usize)> {
    let s = raw.trim();
    if s.len() > MAX_STRING {
        return None;
    }
    let (ty, body) = parse_prefix(s)?;
    let bin = decode_for_print(body)?;
    let json = inflate_json(&bin)?;
    match ty {
        "gbank" => gbank_items(&json).map(|n| ("gbank", n)),
        "recipes" => recipes_ids(&json).map(|n| ("recipes", n)),
        _ => None,
    }
}

fn parse_prefix(s: &str) -> Option<(&'static str, &str)> {
    let rest = s.strip_prefix("!BF:")?;
    let (ty, rest) = rest.split_once(':')?;
    let rest = rest.strip_prefix("v")?;
    let (ver, body) = rest.split_once(':')?;
    if ver.parse::<u32>().ok()? == 0 {
        return None;
    }
    let ty = match ty {
        "gbank" => "gbank",
        "recipes" => "recipes",
        _ => return None,
    };
    Some((ty, body))
}

fn rev(c: u8) -> Option<u32> {
    ALPHA.iter().position(|&x| x == c).map(|i| i as u32)
}

/// LibDeflate DecodeForPrint: 4 chars → 3 bytes little-endian; a 2–3 char tail → 1–2 bytes.
fn decode_for_print(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    if s.len() <= 1 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let v0 = rev(bytes[i])?;
        let v1 = rev(bytes[i + 1])?;
        let v2 = rev(bytes[i + 2])?;
        let v3 = rev(bytes[i + 3])?;
        let n = v0 + v1 * 64 + v2 * 4096 + v3 * 262144;
        out.push((n & 255) as u8);
        out.push(((n >> 8) & 255) as u8);
        out.push(((n >> 16) & 255) as u8);
        i += 4;
    }
    if i < bytes.len() {
        let mut n = 0u32;
        let mut mul = 1u32;
        while i < bytes.len() {
            n += rev(bytes[i])? * mul;
            i += 1;
            mul = mul.checked_mul(64)?;
        }
        while mul > 64 {
            out.push((n & 255) as u8);
            n >>= 8;
            mul /= 256;
        }
    }
    Some(out)
}

fn inflate_json(bin: &[u8]) -> Option<Value> {
    let dec = DeflateDecoder::new(bin);
    let mut buf = Vec::new();
    dec.take(MAX_DECODED as u64 + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() > MAX_DECODED {
        return None;
    }
    let v: Value = serde_json::from_slice(&buf).ok()?;
    if v.is_object() { Some(v) } else { None }
}

fn gbank_items(v: &Value) -> Option<usize> {
    let tabs = v.get("tabs")?.as_array()?;
    let mut n = 0usize;
    for t in tabs {
        n = n.checked_add(t.get("items")?.as_array()?.len())?;
    }
    Some(n)
}

fn recipes_ids(v: &Value) -> Option<usize> {
    Some(v.get("recipes")?.as_array()?.len())
}

#[cfg(test)]
fn encode_for_print(bin: &[u8]) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i + 3 <= bin.len() {
        let n = bin[i] as u32 + (bin[i + 1] as u32) * 256 + (bin[i + 2] as u32) * 65536;
        s.push(ALPHA[(n & 63) as usize] as char);
        s.push(ALPHA[((n >> 6) & 63) as usize] as char);
        s.push(ALPHA[((n >> 12) & 63) as usize] as char);
        s.push(ALPHA[((n >> 18) & 63) as usize] as char);
        i += 3;
    }
    if i < bin.len() {
        let mut n = 0u32;
        let mut mul = 1u32;
        while i < bin.len() {
            n += bin[i] as u32 * mul;
            i += 1;
            mul *= 256;
        }
        let chars = if mul == 256 { 2 } else { 3 };
        for _ in 0..chars {
            s.push(ALPHA[(n & 63) as usize] as char);
            n /= 64;
        }
    }
    s
}

#[cfg(test)]
fn encode_string(ty: &str, json: &Value) -> String {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let raw = serde_json::to_vec(json).unwrap();
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
    enc.write_all(&raw).unwrap();
    let bin = enc.finish().unwrap();
    format!("!BF:{ty}:v1:{}", encode_for_print(&bin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_gbank_items_and_recipe_ids() {
        let bank = json!({"tabs":[
            {"tab":1,"name":"Tab 1","items":[{"itemId":1,"count":2},{"itemId":2,"count":1}]},
            {"tab":2,"name":"Tab 2","items":[{"itemId":3,"count":4}]}
        ],"copper":0});
        let recipes = json!({"profession":"Alchemy","skill":300,"recipes":[1,2,3,4]});
        assert_eq!(envelope_count(&encode_string("gbank", &bank)), Some(("gbank", 3)));
        assert_eq!(envelope_count(&encode_string("recipes", &recipes)), Some(("recipes", 4)));
        assert_eq!(envelope_count("!BF:gbank:v1:$$$$"), None);
        assert_eq!(envelope_count("not-an-envelope"), None);
        assert_eq!(envelope_count(&encode_string("ranks", &json!({"ranks":["a"]}))), None);
    }

    #[test]
    fn encode_for_print_roundtrip_tails() {
        for n in 1..16 {
            let bin: Vec<u8> = (0..n).map(|i| (i * 17) as u8).collect();
            let s = encode_for_print(&bin);
            assert_eq!(decode_for_print(&s).as_deref(), Some(bin.as_slice()), "n={n}");
        }
    }
}
