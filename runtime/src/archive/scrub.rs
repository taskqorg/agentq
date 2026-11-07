//! Length-preserving, best-effort secret recognition before wire chunking.
use serde_json::Value;

pub(super) fn bytes(input: &[u8]) -> (Vec<u8>, u64) {
    bytes_from(input, 0)
}
pub(super) fn bytes_from(input: &[u8], since: usize) -> (Vec<u8>, u64) {
    let mut ranges = Vec::new();
    for prefix in [
        b"sk-".as_slice(),
        b"ghp_",
        b"gho_",
        b"ghu_",
        b"ghs_",
        b"ghr_",
        b"github_pat_",
        b"AKIA",
        b"ASIA",
        b"xoxb-",
        b"xoxp-",
        b"Bearer ",
        b"bearer ",
    ] {
        let mut from = 0;
        while let Some(i) = input[from..]
            .windows(prefix.len())
            .position(|s| s == prefix)
        {
            let start = from + i;
            let value = start + prefix.len();
            let end = input[value..]
                .iter()
                .position(|c| !c.is_ascii_alphanumeric() && !b"_-.+/=".contains(c))
                .map_or(input.len(), |i| value + i);
            if end > value {
                ranges.push((
                    if prefix.eq_ignore_ascii_case(b"Bearer ") {
                        value
                    } else {
                        start
                    },
                    end,
                ));
            }
            from = value;
        }
    }
    // Locate JSON string values without reserializing the record or changing framing.
    let mut pos = 0;
    let mut strings = Vec::new();
    while pos < input.len() {
        if input[pos] != b'"' {
            pos += 1;
            continue;
        }
        let start = pos;
        pos = string_end(input, pos + 1);
        if input.get(pos.saturating_sub(1)) == Some(&b'"') {
            strings.push((start + 1, pos - 1));
        }
        let key = serde_json::from_slice::<String>(&input[start..pos])
            .ok()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let mut next = pos;
        while input.get(next).is_some_and(u8::is_ascii_whitespace) {
            next += 1;
        }
        if input.get(next) != Some(&b':') {
            continue;
        }
        next += 1;
        while input.get(next).is_some_and(u8::is_ascii_whitespace) {
            next += 1;
        }
        if input.get(next) != Some(&b'"') {
            continue;
        }
        let end = string_end(input, next + 1);
        if matches!(
            key.as_str(),
            "api_key"
                | "apikey"
                | "api-key"
                | "access_token"
                | "refresh_token"
                | "id_token"
                | "secret"
                | "secret_key"
                | "private_key"
                | "token"
                | "auth_token"
                | "password"
                | "client_secret"
                | "authorization"
                | "credential"
        ) && end > next + 2
        {
            ranges.push((next + 1, end - 1));
        }
    }
    let begin = b"-----BEGIN ";
    let mut from = 0;
    while let Some(i) = input[from..].windows(begin.len()).position(|s| s == begin) {
        let start = from + i;
        let header_end = input[start..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(input.len(), |i| start + i);
        if input[start..header_end]
            .windows(11)
            .any(|w| w == b"PRIVATE KEY")
        {
            let end = input[start + begin.len()..]
                .windows(9)
                .position(|w| w == b"-----END ")
                .map(|i| {
                    let tail = start + begin.len() + i + 9;
                    input[tail..]
                        .windows(5)
                        .position(|w| w == b"-----")
                        .map_or(input.len(), |i| tail + i + 5)
                })
                .unwrap_or(input.len());
            let end = strings
                .iter()
                .find(|(from, to)| *from <= start && start < *to)
                .map_or(end, |(_, to)| end.min(*to));
            ranges.push((start, end));
        }
        from = header_end.saturating_add(1).min(input.len());
        if from == input.len() {
            break;
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut().filter(|last| start < last.1) {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    let mut out = input.to_vec();
    for &(start, end) in &merged {
        for b in &mut out[start..end] {
            if !matches!(*b, b'\n' | b'\r') {
                *b = b'*';
            }
        }
    }
    let count = merged.iter().filter(|(start, _)| *start >= since).count() as u64;
    (out, count)
}
fn string_end(input: &[u8], mut pos: usize) -> usize {
    while pos < input.len() {
        match input[pos] {
            b'\\' => pos = (pos + 2).min(input.len()),
            b'"' => return pos + 1,
            _ => pos += 1,
        }
    }
    pos
}
pub(super) fn manifest(v: &mut Value) -> u64 {
    match v {
        Value::String(s) => {
            let (bytes, n) = bytes(s.as_bytes());
            *s = String::from_utf8(bytes).expect("ASCII replacement preserves UTF-8");
            n
        }
        Value::Array(a) => a.iter_mut().map(manifest).sum(),
        Value::Object(o) => o.values_mut().map(manifest).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // A6: bytes and offsets stay stable, including a JSON escape, PEM and overlapping recognition.
    #[test]
    fn secrets_preserve_offsets_and_records() {
        let raw=b"{\"api_key\":\"sk-test_credential\",\"password\":\"escaped\\\"value\",\"text\":\"Bearer abc.123==\"}\n-----BEGIN RSA PRIVATE KEY-----\nbody\n-----END RSA PRIVATE KEY-----\n";
        let (out, count) = bytes(raw);
        assert_eq!(out.len(), raw.len());
        assert_eq!(count, 4);
        assert!(out.windows(9).all(|w| w != b"body\n----"));
        assert_eq!(
            out.iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i)
                .collect::<Vec<_>>(),
            raw.iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        );
        assert!(
            serde_json::from_slice::<Value>(out.split(|b| *b == b'\n').next().unwrap()).is_ok()
        );
        let first = b"-----BEGIN PRIVATE KEY-----\nfirst\n";
        let (prefix, count) = bytes(first);
        assert_eq!(count, 1);
        let mut appended = first.to_vec();
        appended.extend_from_slice(b"later secret body\n-----END PRIVATE KEY-----\n");
        let (all, count) = bytes_from(&appended, first.len());
        assert_eq!(count, 0);
        assert_eq!(&all[..first.len()], prefix);
        assert!(!String::from_utf8_lossy(&all).contains("later secret body"));
        let pem=br#"{"text":"-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----","next":"kept"}"#;
        let (out, count) = bytes(pem);
        assert_eq!(count, 1);
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["next"], "kept");
    }
}
