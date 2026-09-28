//! Bounded DNS A/IN codec for the M7.5 resolver. No allocation or ambient
//! name service. This module is also compiled by the host-side parser gate.

pub const QUERY_MAX: usize = 56; // one UDP SEND inline payload
pub const NAME_MAX: usize = 32; // dotted ASCII name; query fits QUERY_MAX

fn word(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// Canonical lower-case wire name from a bounded dotted ASCII input.
/// No trailing dot, empty labels, underscores, or non-ASCII bytes.
pub fn encode_name(name: &[u8], dst: &mut [u8]) -> Option<usize> {
    if name.is_empty() || name.len() > NAME_MAX {
        return None;
    }
    let mut n = 0;
    for label in name.split(|&b| b == b'.') {
        if label.is_empty()
            || label.len() > 63
            || label[0] == b'-'
            || label[label.len() - 1] == b'-'
        {
            return None;
        }
        if n + 1 + label.len() + 1 > dst.len() {
            return None;
        }
        dst[n] = label.len() as u8;
        n += 1;
        for &c in label {
            if !c.is_ascii_alphanumeric() && c != b'-' {
                return None;
            }
            dst[n] = c.to_ascii_lowercase();
            n += 1;
        }
    }
    dst[n] = 0;
    Some(n + 1)
}

pub fn query(name: &[u8], id: u16, out: &mut [u8; QUERY_MAX]) -> Option<usize> {
    out.fill(0);
    out[..2].copy_from_slice(&id.to_be_bytes());
    out[2] = 1; // recursion desired
    out[5] = 1; // one question
    let n = encode_name(name, &mut out[12..QUERY_MAX - 4])?;
    let end = 12 + n;
    out[end + 1] = 1; // A
    out[end + 3] = 1; // IN
    Some(end + 4)
}

/// Decode a compressed wire name with a strict bound on indirection.
/// Return the offset AFTER the name in the enclosing record, and the
/// canonical uncompressed wire name. Only backward pointers are accepted:
/// forward references and cycles have no use in a reply to our query.
fn name(b: &[u8], start: usize) -> Option<(usize, [u8; QUERY_MAX], usize)> {
    let mut out = [0u8; QUERY_MAX];
    let mut pos = start;
    let mut next = None;
    let mut used = 0;
    let mut hops = 0;
    loop {
        let tag = *b.get(pos)?;
        if tag & 0xc0 == 0xc0 {
            let ptr = (((tag as usize) & 0x3f) << 8) | *b.get(pos + 1)? as usize;
            if ptr >= pos || hops >= 8 {
                return None;
            }
            if next.is_none() {
                next = Some(pos + 2);
            }
            pos = ptr;
            hops += 1;
            continue;
        }
        if tag & 0xc0 != 0 || tag as usize > 63 {
            return None;
        }
        pos += 1;
        if used + 1 + tag as usize > out.len() {
            return None;
        }
        out[used] = tag;
        used += 1;
        if tag == 0 {
            return Some((next.unwrap_or(pos), out, used));
        }
        for _ in 0..tag {
            out[used] = b.get(pos)?.to_ascii_lowercase();
            used += 1;
            pos += 1;
        }
    }
}

/// Match the transaction AND question, then accept only an A/IN answer
/// for that name. No CNAME chasing or caching in v1. Refuse truncation,
/// non-success RCODE, malformed compression and unrelated answer owners.
pub fn answer(b: &[u8], id: u16, q: &[u8]) -> Option<[u8; 4]> {
    if b.len() < 12
        || word(b, 0)? != id
        || b[2] & 0xfa != 0x80
        || b[3] & 0x7f != 0
        || word(b, 4)? != 1
        || word(b, 6)? == 0
    {
        return None;
    }
    let (end, qname, qlen) = name(b, 12)?;
    let qend = q.len().checked_sub(4)?;
    if qend < 13
        || qlen != qend - 12
        || qname[..qlen] != q[12..qend]
        || word(b, end)? != 1
        || word(b, end + 2)? != 1
    {
        return None;
    }
    let mut at = end + 4;
    for _ in 0..word(b, 6)?.min(16) {
        let (end, owner, olen) = name(b, at)?;
        let typ = word(b, end)?;
        let class = word(b, end + 2)?;
        let len = word(b, end + 8)? as usize;
        at = end.checked_add(10)?;
        let data = b.get(at..at.checked_add(len)?)?;
        if owner[..olen] == qname[..qlen] && typ == 1 && class == 1 && len == 4 {
            return Some([data[0], data[1], data[2], data[3]]);
        }
        at += len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_names() {
        let mut q = [0; QUERY_MAX];
        assert_eq!(query(b"ExAmPlE.com", 0xa7e5, &mut q), Some(29));
        assert_eq!(&q[12..25], b"\x07example\x03com\x00");
        for bad in [b"".as_slice(), b"a..b", b"a.", b"-a.org", b"a_.org", b"a/b"] {
            assert!(query(bad, 1, &mut q).is_none());
        }
    }
    #[test]
    fn compressed_answer_and_negative_space() {
        let mut q = [0; QUERY_MAX];
        let n = query(b"example.com", 0xa7e5, &mut q).unwrap();
        let mut b = [0u8; 64];
        b[..n].copy_from_slice(&q[..n]);
        b[2] = 0x81;
        b[3] = 0x80;
        b[7] = 1; // response + one answer
        b[n..n + 2].copy_from_slice(&[0xc0, 12]);
        b[n + 3] = 1;
        b[n + 5] = 1; // A IN
        b[n + 11] = 4;
        b[n + 12..n + 16].copy_from_slice(&[1, 2, 3, 4]);
        let good = &b[..n + 16];
        assert_eq!(answer(good, 0xa7e5, &q[..n]), Some([1, 2, 3, 4]));
        assert_eq!(answer(good, 0xa7e4, &q[..n]), None);
        assert_eq!(answer(&good[..good.len() - 1], 0xa7e5, &q[..n]), None);
        b[3] = 0x83;
        assert_eq!(answer(&b[..n + 16], 0xa7e5, &q[..n]), None);
        b[3] = 0x80;
        b[n + 1] = n as u8; // compression cycle, not an answer
        assert_eq!(answer(&b[..n + 16], 0xa7e5, &q[..n]), None);
        b[n + 1] = 12;
        b[13] = b'X'; // valid reply to a DIFFERENT question
        assert_eq!(answer(&b[..n + 16], 0xa7e5, &q[..n]), None);
    }
}
