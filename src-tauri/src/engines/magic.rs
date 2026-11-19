//! "Magic decode" for CTF work: peels encodings one layer at a time (base64, base32, hex, binary,
//! decimal codes, URL encoding, gzip/zlib/deflate, ROT13) until readable text is left, decodes JWTs,
//! and points out anything shaped like a flag. Runs in-process on at most 1 MB of input.

use anyhow::{bail, Result};
use std::io::Read;

const MAX_INPUT: usize = 1 << 20;
const MAX_LAYERS: usize = 16;
/// a layer may not grow the data past this (zip-bomb style input)
const MAX_OUTPUT: usize = 16 << 20;

fn printable_ratio(b: &[u8]) -> f64 {
    if b.is_empty() {
        return 0.0;
    }
    let text = std::str::from_utf8(b);
    let n = match text {
        Ok(s) => s.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).count() as f64 / s.chars().count().max(1) as f64,
        Err(_) => b.iter().filter(|&&c| (0x20..0x7f).contains(&c) || matches!(c, b'\n' | b'\r' | b'\t')).count() as f64 / b.len() as f64,
    };
    n
}

fn is_text(b: &[u8]) -> bool {
    printable_ratio(b) > 0.95
}

fn compact(b: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(b).ok()?;
    Some(s.chars().filter(|c| !c.is_whitespace()).collect())
}

fn b64(b: &[u8]) -> Option<Vec<u8>> {
    use base64::Engine;
    let s = compact(b)?;
    if s.len() < 4 || !s.bytes().all(|c| c.is_ascii_alphanumeric() || b"+/=-_".contains(&c)) {
        return None;
    }
    let url = s.contains(['-', '_']);
    let trimmed = s.trim_end_matches('=');
    let e = if url { &base64::engine::general_purpose::URL_SAFE_NO_PAD } else { &base64::engine::general_purpose::STANDARD_NO_PAD };
    e.decode(trimmed).ok().filter(|v| !v.is_empty())
}

fn b32(b: &[u8]) -> Option<Vec<u8>> {
    let s = compact(b)?.to_ascii_uppercase();
    let s = s.trim_end_matches('=');
    if s.len() < 8 || !s.bytes().all(|c| c.is_ascii_uppercase() || (b'2'..=b'7').contains(&c)) {
        return None;
    }
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u64, 0u32);
    for c in s.bytes() {
        let v = if c.is_ascii_uppercase() { c - b'A' } else { c - b'2' + 26 } as u64;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn hex(b: &[u8]) -> Option<Vec<u8>> {
    let s = compact(b)?.to_ascii_lowercase().replace("0x", "").replace(['\\', ':', ','], "").replace('x', "");
    if s.len() < 4 || s.len() % 2 != 0 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

fn binary(b: &[u8]) -> Option<Vec<u8>> {
    let s = compact(b)?;
    if s.len() < 8 || s.len() % 8 != 0 || !s.bytes().all(|c| c == b'0' || c == b'1') {
        return None;
    }
    (0..s.len()).step_by(8).map(|i| u8::from_str_radix(&s[i..i + 8], 2).ok()).collect()
}

fn decimal(b: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(b).ok()?;
    let parts: Vec<&str> = s.split(|c: char| c.is_whitespace() || c == ',').filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 || !parts.iter().all(|p| p.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    parts.iter().map(|p| p.parse::<u8>().ok()).collect()
}

fn url(b: &[u8]) -> Option<Vec<u8>> {
    let s = std::str::from_utf8(b).ok()?;
    if !s.contains('%') {
        return None;
    }
    let mut out = Vec::with_capacity(s.len());
    let by = s.as_bytes();
    let mut i = 0;
    let mut decoded = false;
    while i < by.len() {
        if by[i] == b'%' {
            if let Some(v) = by.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                decoded = true;
                continue;
            }
        }
        out.push(if by[i] == b'+' { b' ' } else { by[i] });
        i += 1;
    }
    decoded.then_some(out)
}

fn inflate(b: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let read = |mut r: Box<dyn Read + '_>| -> Option<Vec<u8>> {
        let mut out = Vec::new();
        r.by_ref().take(MAX_OUTPUT as u64 + 1).read_to_end(&mut out).ok()?;
        (!out.is_empty() && out.len() <= MAX_OUTPUT).then_some(out)
    };
    if b.starts_with(&[0x1f, 0x8b]) {
        return read(Box::new(flate2::read::GzDecoder::new(b))).map(|o| (o, "gzip"));
    }
    if b.len() > 2 && b[0] & 0x0f == 8 && ((b[0] as u16) << 8 | b[1] as u16).is_multiple_of(31) {
        if let Some(o) = read(Box::new(flate2::read::ZlibDecoder::new(b))) {
            return Some((o, "zlib"));
        }
    }
    None
}

fn rot13(b: &[u8]) -> Vec<u8> {
    b.iter()
        .map(|&c| match c {
            b'a'..=b'z' => (c - b'a' + 13) % 26 + b'a',
            b'A'..=b'Z' => (c - b'A' + 13) % 26 + b'A',
            _ => c,
        })
        .collect()
}

/// Spans shaped like CTF flags: word{...}.
pub fn flags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    for (i, _) in text.match_indices('{') {
        let start = (0..i).rev().take_while(|&k| b[k].is_ascii_alphanumeric() || b[k] == b'_').last();
        let Some(start) = start else { continue };
        let prefix = &text[start..i];
        if prefix.len() < 2 || prefix.len() > 16 {
            continue;
        }
        if let Some(end) = text[i..].find('}').map(|e| i + e) {
            let inner = &text[i + 1..end];
            if (2..=200).contains(&inner.len()) && !inner.contains(['\n', '{']) {
                let f = text[start..=end].to_string();
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
    }
    out
}

const FLAG_WORDS: &[&str] = &["flag", "ctf", "htb", "thm", "picoctf", "key", "secret"];

fn looks_like_flag(text: &str) -> bool {
    flags(text).iter().any(|f| FLAG_WORDS.iter().any(|w| f.to_ascii_lowercase().starts_with(w)))
}

type Decoder = fn(&[u8]) -> Option<Vec<u8>>;

/// One decoding step: what changed and the new bytes.
fn step(cur: &[u8]) -> Option<(&'static str, Vec<u8>)> {
    if let Some((o, name)) = inflate(cur) {
        return Some((name, o));
    }
    let tries: [(&str, Decoder); 6] = [("binary", binary), ("decimal codes", decimal), ("hex", hex), ("base32", b32), ("base64", b64), ("URL encoding", url)];
    for (name, f) in tries {
        if let Some(o) = f(cur) {
            // keep a layer only when it leads somewhere: text, compressed data, or another layer
            if is_text(&o) || inflate(&o).is_some() || o.len() >= 4 {
                if !is_text(&o) && inflate(&o).is_none() && step_light(&o).is_none() {
                    continue;
                }
                return Some((name, o));
            }
        }
    }
    let text = std::str::from_utf8(cur).ok()?;
    if !looks_like_flag(text) && looks_like_flag(&String::from_utf8_lossy(&rot13(cur))) {
        return Some(("ROT13", rot13(cur)));
    }
    None
}

/// Whether one more layer can be peeled (without recursing further), to tell a real layer from noise.
fn step_light(cur: &[u8]) -> Option<()> {
    if inflate(cur).is_some() {
        return Some(());
    }
    [binary as fn(&[u8]) -> Option<Vec<u8>>, decimal, hex, b32, b64, url].iter().find_map(|f| f(cur).filter(|o| is_text(o))).map(|_| ())
}

fn jwt(text: &str) -> Option<String> {
    use base64::Engine;
    let t = text.trim();
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() != 3 || !parts[0].starts_with("eyJ") {
        return None;
    }
    let dec = |p: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let (h, p) = (dec(parts[0])?, dec(parts[1])?);
    let alg = h["alg"].as_str().unwrap_or("?");
    let mut out = format!(
        "JSON Web Token (alg {alg}; the signature is not checked here)\n\nHeader:\n{}\n\nPayload:\n{}\n",
        serde_json::to_string_pretty(&h).ok()?,
        serde_json::to_string_pretty(&p).ok()?
    );
    if alg.eq_ignore_ascii_case("none") {
        out.push_str("\nNote: alg \"none\" means the token is not signed at all.\n");
    }
    if let Some(exp) = p["exp"].as_u64() {
        out.push_str(&format!("\nexp: {exp} (seconds since 1970)\n"));
    }
    Some(out)
}

pub struct Decoded {
    pub layers: Vec<&'static str>,
    pub result: Vec<u8>,
    pub flags: Vec<String>,
}

pub fn decode(input: &[u8]) -> Result<Decoded> {
    if input.len() > MAX_INPUT {
        bail!("this file is too big to decode here (limit 1 MB)");
    }
    let mut cur: Vec<u8> = input.to_vec();
    // a trailing newline from an editor isn't part of the data
    while cur.last().is_some_and(|c| c.is_ascii_whitespace()) {
        cur.pop();
    }
    if let Some(j) = std::str::from_utf8(&cur).ok().and_then(jwt) {
        let fl = flags(&j);
        return Ok(Decoded { layers: vec!["JWT"], result: j.into_bytes(), flags: fl });
    }
    let mut layers = Vec::new();
    while layers.len() < MAX_LAYERS {
        if is_text(&cur) && looks_like_flag(&String::from_utf8_lossy(&cur)) {
            break;
        }
        let Some((name, next)) = step(&cur) else { break };
        if next == cur {
            break;
        }
        layers.push(name);
        cur = next;
        while cur.last().is_some_and(|c| c.is_ascii_whitespace()) {
            cur.pop();
        }
    }
    if layers.is_empty() {
        bail!("no encoding found: this doesn't look like base64, base32, hex, binary, URL encoding, compressed data or a JWT");
    }
    let fl = flags(&String::from_utf8_lossy(&cur));
    Ok(Decoded { layers, result: cur, flags: fl })
}

/// The report written next to the file.
pub fn report(d: &Decoded) -> String {
    let mut s = format!("Magic decode\nLayers: {}\n", d.layers.join(" → "));
    if !d.flags.is_empty() {
        s.push_str(&format!("Possible flags: {}\n", d.flags.join(", ")));
    }
    s.push('\n');
    if is_text(&d.result) {
        s.push_str(&String::from_utf8_lossy(&d.result));
    } else {
        s.push_str(&format!("Binary result ({} bytes), first 1 KB as hex:\n", d.result.len()));
        for row in d.result.chunks(16).take(64) {
            let hexes: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
            let ascii: String = row.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
            s.push_str(&format!("{:<48} {ascii}\n", hexes.join(" ")));
        }
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn gz(b: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn peels_layers() {
        let flag = b"flag{m4g1c_d3c0d3}";
        let hexed: String = base64::engine::general_purpose::STANDARD.encode(flag).bytes().map(|b| format!("{b:02x}")).collect();
        let d = decode(hexed.as_bytes()).unwrap();
        assert_eq!(d.layers, vec!["hex", "base64"]);
        assert_eq!(d.result, flag);
        assert_eq!(d.flags, vec!["flag{m4g1c_d3c0d3}"]);

        let packed = base64::engine::general_purpose::STANDARD.encode(gz(b"HTB{zipped_and_encoded}"));
        let d = decode(packed.as_bytes()).unwrap();
        assert_eq!(d.layers, vec!["base64", "gzip"]);
        assert_eq!(d.flags, vec!["HTB{zipped_and_encoded}"]);

        let d = decode(b"synt{ebg_guvegrra}\n").unwrap();
        assert_eq!(d.layers, vec!["ROT13"]);
        assert_eq!(d.result, b"flag{rot_thirteen}");

        let d = decode(b"01100110 01101100 01100001 01100111 01111011 01100010 01111101").unwrap();
        assert_eq!((d.layers[0], d.result.as_slice()), ("binary", b"flag{b}".as_slice()));

        let d = decode(b"102 108 97 103 123 100 125").unwrap();
        assert_eq!(d.result, b"flag{d}");

        let d = decode(b"JBSWY3DPEE======").unwrap();
        assert_eq!((d.layers[0], d.result.as_slice()), ("base32", b"Hello!".as_slice()));

        let d = decode(b"flag%7Burl%5Fenc%7D").unwrap();
        assert_eq!(d.result, b"flag{url_enc}");
    }

    #[test]
    fn jwt_and_refusals() {
        let e = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let tok = format!("{}.{}.sig", e(r#"{"alg":"none","typ":"JWT"}"#), e(r#"{"sub":"admin","exp":1}"#));
        let d = decode(tok.as_bytes()).unwrap();
        let r = report(&d);
        assert!(r.contains("\"sub\": \"admin\"") && r.contains("not signed"), "{r}");
        assert!(decode(b"just some ordinary words here").is_err());
        assert!(decode(&vec![b'A'; MAX_INPUT + 1]).is_err());
    }
}
