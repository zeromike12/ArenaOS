//! TCP wire codec, kept separate from connection policy. No allocation.
//! One checksum implementation covers the IPv4 pseudo-header and the
//! entire segment, including options and an odd-length payload.

pub const HDR: usize = 20;
pub const MSS: usize = 400;
pub const SYN: u8 = 0x02;
pub const ACK: u8 = 0x10;
pub const FIN: u8 = 0x01;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;

pub const RTO_US: u64 = 250_000;
pub const MAX_RETRIES: u8 = 3;
#[derive(Debug, PartialEq, Eq)]
pub enum Retry {
    Wait,
    Resend,
    Fail,
}
/// Pure timing policy: an early retransmission manufactures duplicates;
/// an unbounded one leaves clients parked forever on a silent peer.
pub fn retry_action(sent: u64, now: u64, retries: u8) -> Retry {
    if now < sent.saturating_add(RTO_US) {
        Retry::Wait
    } else if retries >= MAX_RETRIES {
        Retry::Fail
    } else {
        Retry::Resend
    }
}

fn word(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn long(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn add(mut sum: u32, b: &[u8]) -> u32 {
    for pair in b.chunks(2) {
        sum += ((pair[0] as u32) << 8) | pair.get(1).copied().unwrap_or(0) as u32;
    }
    sum
}

pub fn checksum(src: [u8; 4], dst: [u8; 4], segment: &[u8]) -> u16 {
    let mut sum = add(0, &src);
    sum = add(sum, &dst);
    sum += 6 + segment.len() as u32;
    sum = add(sum, segment);
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
    Short,
    Header,
    Checksum,
}

pub struct Segment<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    pub data: &'a [u8],
}

pub fn parse<'a>(src: [u8; 4], dst: [u8; 4], b: &'a [u8]) -> Result<Segment<'a>, ParseError> {
    if b.len() < HDR {
        return Err(ParseError::Short);
    }
    let hlen = (b[12] >> 4) as usize * 4;
    if hlen < HDR || hlen > b.len() || b[12] & 0x0f != 0 {
        return Err(ParseError::Header);
    }
    if checksum(src, dst, b) != 0 {
        return Err(ParseError::Checksum);
    }
    Ok(Segment {
        src_port: word(b, 0),
        dst_port: word(b, 2),
        seq: long(b, 4),
        ack: long(b, 8),
        flags: b[13],
        window: word(b, 14),
        data: &b[hlen..],
    })
}

/// Produce a segment whose IPv4 pseudo-header checksum is nonzero on
/// the wire. SYN carries MSS 400; all other packets have a 20-byte
/// header. Caller supplies a buffer of at least 24+payload bytes.
// Fields correspond exactly to the wire header; grouping them would hide offsets.
#[allow(clippy::too_many_arguments)]
pub fn write(
    out: &mut [u8],
    src: [u8; 4],
    dst: [u8; 4],
    sport: u16,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    data: &[u8],
) -> usize {
    let hlen = HDR + if flags & SYN != 0 { 4 } else { 0 };
    let len = hlen + data.len();
    assert!(out.len() >= len);
    out[..len].fill(0);
    out[..2].copy_from_slice(&sport.to_be_bytes());
    out[2..4].copy_from_slice(&dport.to_be_bytes());
    out[4..8].copy_from_slice(&seq.to_be_bytes());
    out[8..12].copy_from_slice(&ack.to_be_bytes());
    out[12] = (hlen as u8 / 4) << 4;
    out[13] = flags;
    out[14..16].copy_from_slice(&window.to_be_bytes());
    if flags & SYN != 0 {
        out[20..24].copy_from_slice(&[2, 4, 1, 144]);
    }
    out[hlen..len].copy_from_slice(data);
    let sum = checksum(src, dst, &out[..len]);
    out[16..18].copy_from_slice(&sum.to_be_bytes());
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    const SRC: [u8; 4] = [10, 0, 2, 2];
    const DST: [u8; 4] = [10, 0, 2, 15];
    #[test]
    fn valid_odd_payload_and_corruption() {
        let mut b = [0u8; 64];
        let n = write(
            &mut b,
            SRC,
            DST,
            54321,
            49152,
            7,
            11,
            ACK | PSH,
            400,
            b"abcde",
        );
        assert_eq!(parse(SRC, DST, &b[..n]).unwrap().data, b"abcde");
        b[n - 1] ^= 1;
        assert!(matches!(
            parse(SRC, DST, &b[..n]),
            Err(ParseError::Checksum)
        ));
        b[n - 1] ^= 1;
        assert!(matches!(
            parse([10, 0, 2, 99], DST, &b[..n]),
            Err(ParseError::Checksum)
        ));
    }
    #[test]
    fn no_early_or_unbounded_retransmission() {
        assert_eq!(retry_action(100, 100 + RTO_US - 1, 0), Retry::Wait);
        assert_eq!(retry_action(100, 100 + RTO_US, 0), Retry::Resend);
        assert_eq!(retry_action(100, 100 + RTO_US, MAX_RETRIES), Retry::Fail);
    }
    #[test]
    fn syn_options_and_header_bounds() {
        let mut b = [0u8; 64];
        let n = write(&mut b, SRC, DST, 54321, 49152, 7, 11, SYN | ACK, 400, b"");
        assert_eq!(n, 24);
        assert_eq!(parse(SRC, DST, &b[..n]).unwrap().flags, SYN | ACK);
        b[12] = 0x40;
        assert!(matches!(parse(SRC, DST, &b[..n]), Err(ParseError::Header)));
        b[12] = 0xf0;
        assert!(matches!(parse(SRC, DST, &b[..n]), Err(ParseError::Header)));
        assert!(matches!(parse(SRC, DST, &b[..19]), Err(ParseError::Short)));
    }
}
