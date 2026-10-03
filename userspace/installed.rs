//! ADR-0054: accepted immutable AINS/AACT v1 byte records. No disk, caps,
//! crypto policy or Image authority lives here. The caller supplies SHA-256
//! and must separately verify signed APKG/current APOL, namespace, chain,
//! fsd crash-prefix and administrator marker at every authorization boundary.
#![allow(dead_code)]

pub const BYTES: usize = 512;
pub type Digest = [u8; 32];
pub type Id = [u8; 32];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Format,
    Sequence,
    Chain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Installed {
    pub generation: u64,
    pub id: Id,
    pub full_digest: Digest,
    pub version: u64,
    pub payload_digest: Digest,
    pub stage: u8,
    pub observed_policy: u8, // audit fact ONLY, not eligibility
    pub previous: Digest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activation {
    pub generation: u64,
    pub id: Id,
    pub installed_hash: Digest, // zero on DEACTIVATE
    pub full_digest: Digest,    // zero on DEACTIVATE
    pub version: u64,           // zero on DEACTIVATE
    pub select: bool,
    pub previous: Digest,
}

fn canonical_id(id: &Id) -> bool {
    let Some(end) = id.iter().position(|&c| c == 0) else {
        return false;
    };
    if !(1..=31).contains(&end) || id[end..].iter().any(|&b| b != 0) {
        return false;
    }
    (id[0].is_ascii_lowercase() || id[0].is_ascii_digit())
        && id[..end]
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}
fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes(b.try_into().expect("bounded slice"))
}
fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().expect("bounded slice"))
}
fn word32(b: &[u8]) -> Digest {
    let mut x = [0; 32];
    x.copy_from_slice(b);
    x
}
fn nz(d: &Digest) -> bool {
    d.iter().any(|&b| b != 0)
}

fn header(
    b: &[u8],
    magic: &[u8; 4],
    limit: u64,
    hash: fn(&[u8]) -> Digest,
) -> Result<(u64, Id, Digest), Error> {
    if b.len() != BYTES || &b[..4] != magic || le16(&b[4..6]) != 1 || le16(&b[6..8]) != 512 {
        return Err(Error::Format);
    }
    let generation = le64(&b[8..16]);
    if !(1..=limit).contains(&generation) {
        return Err(Error::Sequence);
    }
    let id = word32(&b[16..48]);
    if !canonical_id(&id) || b[480..512] != hash(&b[..480]) {
        return Err(Error::Format);
    }
    let previous = word32(&b[128..160]);
    if (generation == 1 && nz(&previous)) || (generation > 1 && !nz(&previous)) {
        return Err(Error::Chain);
    }
    Ok((generation, id, previous))
}
fn valid_installed(r: &Installed) -> Result<(), Error> {
    if !(1..=2).contains(&r.generation)
        || !canonical_id(&r.id)
        || !nz(&r.full_digest)
        || !nz(&r.payload_digest)
        || r.version == 0
        || !(1..=2).contains(&r.stage)
        || r.observed_policy > 4
    {
        return Err(Error::Format);
    }
    if (r.generation == 1 && nz(&r.previous)) || (r.generation > 1 && !nz(&r.previous)) {
        return Err(Error::Chain);
    }
    Ok(())
}
fn valid_activation(r: &Activation) -> Result<(), Error> {
    if !(1..=4).contains(&r.generation) || !canonical_id(&r.id) {
        return Err(Error::Format);
    }
    if r.select {
        if !nz(&r.installed_hash) || !nz(&r.full_digest) || r.version == 0 {
            return Err(Error::Format);
        }
    } else if nz(&r.installed_hash) || nz(&r.full_digest) || r.version != 0 {
        return Err(Error::Format);
    }
    if (r.generation == 1 && nz(&r.previous)) || (r.generation > 1 && !nz(&r.previous)) {
        return Err(Error::Chain);
    }
    Ok(())
}

pub fn encode_installed(
    r: &Installed,
    out: &mut [u8; BYTES],
    hash: fn(&[u8]) -> Digest,
) -> Result<(), Error> {
    valid_installed(r)?; // caller's output is unchanged on refusal
    out.fill(0);
    out[..4].copy_from_slice(b"AINS");
    out[4..6].copy_from_slice(&1u16.to_le_bytes());
    out[6..8].copy_from_slice(&512u16.to_le_bytes());
    out[8..16].copy_from_slice(&r.generation.to_le_bytes());
    out[16..48].copy_from_slice(&r.id);
    out[48..80].copy_from_slice(&r.full_digest);
    out[80..88].copy_from_slice(&r.version.to_le_bytes());
    out[88..120].copy_from_slice(&r.payload_digest);
    out[120] = r.stage;
    out[121] = r.observed_policy;
    out[128..160].copy_from_slice(&r.previous);
    let checksum = hash(&out[..480]);
    out[480..512].copy_from_slice(&checksum);
    Ok(())
}

pub fn parse_installed(b: &[u8], hash: fn(&[u8]) -> Digest) -> Result<Installed, Error> {
    let (generation, id, previous) = header(b, b"AINS", 2, hash)?;
    if b[122..128].iter().any(|&x| x != 0) || b[160..480].iter().any(|&x| x != 0) {
        return Err(Error::Format);
    }
    let r = Installed {
        generation,
        id,
        full_digest: word32(&b[48..80]),
        version: le64(&b[80..88]),
        payload_digest: word32(&b[88..120]),
        stage: b[120],
        observed_policy: b[121],
        previous,
    };
    valid_installed(&r)?;
    Ok(r)
}

pub fn encode_activation(
    r: &Activation,
    out: &mut [u8; BYTES],
    hash: fn(&[u8]) -> Digest,
) -> Result<(), Error> {
    valid_activation(r)?;
    out.fill(0);
    out[..4].copy_from_slice(b"AACT");
    out[4..6].copy_from_slice(&1u16.to_le_bytes());
    out[6..8].copy_from_slice(&512u16.to_le_bytes());
    out[8..16].copy_from_slice(&r.generation.to_le_bytes());
    out[16..48].copy_from_slice(&r.id);
    out[48..80].copy_from_slice(&r.installed_hash);
    out[80..112].copy_from_slice(&r.full_digest);
    out[112..120].copy_from_slice(&r.version.to_le_bytes());
    out[120] = if r.select { 1 } else { 2 };
    out[128..160].copy_from_slice(&r.previous);
    let checksum = hash(&out[..480]);
    out[480..512].copy_from_slice(&checksum);
    Ok(())
}

pub fn parse_activation(b: &[u8], hash: fn(&[u8]) -> Digest) -> Result<Activation, Error> {
    let (generation, id, previous) = header(b, b"AACT", 4, hash)?;
    if b[121..128].iter().any(|&x| x != 0) || b[160..480].iter().any(|&x| x != 0) {
        return Err(Error::Format);
    }
    let select = match b[120] {
        1 => true,
        2 => false,
        _ => return Err(Error::Format),
    };
    let r = Activation {
        generation,
        id,
        installed_hash: word32(&b[48..80]),
        full_digest: word32(&b[80..112]),
        version: le64(&b[112..120]),
        select,
        previous,
    };
    valid_activation(&r)?;
    Ok(r)
}

/// Verify a complete predecessor's exact bytes and full namespace identity.
/// Existence, ordinal names, signed-package digest/current policy and complete
/// namespace scanning remain the caller's responsibility, never a filename hint.
pub fn check_predecessor(
    current_id: &Id,
    current_gen: u64,
    previous: &Digest,
    prior: Option<&[u8; BYTES]>,
    installed: bool,
    hash: fn(&[u8]) -> Digest,
) -> Result<(), Error> {
    if current_gen == 1 {
        return if !nz(previous) && prior.is_none() {
            Ok(())
        } else {
            Err(Error::Chain)
        };
    }
    let p = prior.ok_or(Error::Chain)?;
    let (pgen, pid) = if installed {
        let v = parse_installed(p, hash)?;
        (v.generation, v.id)
    } else {
        let v = parse_activation(p, hash)?;
        (v.generation, v.id)
    };
    if pgen.checked_add(1) != Some(current_gen) || &pid != current_id || hash(p) != *previous {
        return Err(Error::Chain);
    }
    Ok(())
}
