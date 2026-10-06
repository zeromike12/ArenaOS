//! ADR-0053: bounded, no_std, verify-only signed package/policy byte core.
//!
//! This module owns no filesystem or IPC authority. Its callers MUST verify
//! receiver-side marker possession and complete fsd namespace scans before
//! making a stage decision. `Package` is signature-valid, not installed/active.
//! No private key, signer, RNG, serde, allocator, ELF loader or Image cap here.
use arena_phase84_crypto_audit::{canonical_public_key, sha256, strict_verify};

pub const ROOT: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7,
    0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25,
    0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];
pub const ROOT_ID: [u8; 32] = [
    0x21, 0xfe, 0x31, 0xdf, 0xa1, 0x54, 0xa2, 0x61,
    0x62, 0x6b, 0xf8, 0x54, 0x04, 0x6f, 0xd2, 0x27,
    0x1b, 0x7b, 0xed, 0x4b, 0x6a, 0xbe, 0x45, 0xaa,
    0x58, 0x87, 0x7e, 0xf4, 0x7f, 0x97, 0x21, 0xb9,
];
pub const PACKAGE_MAX: usize = 4288;
// The guest has a SINGLE 4 KiB user stack page. Caller owns this workspace in
// .bss / a mapped buffer; never allocate the 4,239-byte signature message on
// the user stack. This also prevents an unreviewed kernel stack-mapping change.
pub const VERIFY_SCRATCH: usize = 15 + 128 + 4096;
pub const POLICY_BYTES: usize = 512;
pub const MAX_POLICIES: u8 = 4;
pub const MAX_STAGES: u8 = 2;
const PKG_DOMAIN: &[u8; 15] = b"ArenaOS.pkg.v1\0";
const POL_DOMAIN: &[u8; 18] = b"ArenaOS.policy.v1\0";
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { BadFormat, BadSignature, UnknownSigner, Revoked, Downgrade, Conflict, NoSpace, Corrupt, Collision }

fn le16(b: &[u8]) -> u16 { u16::from_le_bytes([b[0], b[1]]) }
fn le32(b: &[u8]) -> u32 { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
fn le64(b: &[u8]) -> u64 { u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) }
fn array32(b: &[u8]) -> [u8; 32] { let mut a = [0; 32]; a.copy_from_slice(b); a }

pub fn canonical_id(raw: &[u8]) -> bool {
    if raw.len() != 32 { return false; }
    let Some(end) = raw.iter().position(|&c| c == 0) else { return false; };
    if !(1..=31).contains(&end) || raw[end..].iter().any(|&c| c != 0) { return false; }
    let head = raw[0];
    if !head.is_ascii_lowercase() && !head.is_ascii_digit() { return false; }
    raw[..end].iter().all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
}

/// Hash-namespace prefix with exactly 20 lowercase hex digits. Internal full
/// signed IDs still govern identity; this prefix never grants authority.
pub fn namespace(id: &[u8; 32]) -> Result<[u8; 20], Error> {
    if !canonical_id(id) { return Err(Error::BadFormat); }
    let hash = sha256(id);
    let mut out = [0u8; 20];
    for (i, b) in hash[..10].iter().enumerate() {
        out[2*i] = HEX[(b >> 4) as usize];
        out[2*i+1] = HEX[(b & 15) as usize];
    }
    Ok(out)
}

pub fn numbered_name(prefix: &[u8; 3], id: &[u8; 32], seq: u8, max: u8) -> Result<[u8; 26], Error> {
    if seq == 0 || seq > max || max > 9 { return Err(Error::BadFormat); }
    let mut out = [0u8; 26];
    out[..3].copy_from_slice(prefix);
    out[3..23].copy_from_slice(&namespace(id)?);
    out[23] = b'-'; out[24] = b'0'; out[25] = b'0' + seq;
    Ok(out)
}

pub fn input_name(id: &[u8; 32], policy_intent: bool) -> Result<[u8; 23], Error> {
    let mut out = [0u8; 23];
    out[..3].copy_from_slice(if policy_intent { b"a8-" } else { b"i8-" });
    out[3..].copy_from_slice(&namespace(id)?);
    Ok(out)
}

#[derive(Clone, Copy)]
pub struct Package<'a> {
    pub id: [u8; 32],
    pub version: u64,
    pub signer_id: [u8; 32],
    pub full_digest: [u8; 32],
    pub file: &'a [u8],
}
impl Package<'_> {
    /// The key ID and canonical key are enforced independently of the signed
    /// bytes; a filename never selects or substitutes a key.
    pub fn verify(&self, key: &[u8; 32], message: &mut [u8; VERIFY_SCRATCH]) -> Result<(), Error> {
        if sha256(key) != self.signer_id { return Err(Error::UnknownSigner); }
        let signed_len = self.file.len() - 64;
        message[..15].copy_from_slice(PKG_DOMAIN);
        message[15..15 + signed_len].copy_from_slice(&self.file[..signed_len]);
        let signature: &[u8; 64] = self.file[signed_len..].try_into().map_err(|_| Error::BadFormat)?;
        if !strict_verify(key, &message[..15 + signed_len], signature) { return Err(Error::BadSignature); }
        Ok(())
    }
}

/// Fully bounds-check before slicing: a package is 128-byte manifest, 1..4096
/// opaque payload bytes and exactly 64 signature bytes, without trailing data.
pub fn parse_package(file: &[u8]) -> Result<Package<'_>, Error> {
    if !(193..=PACKAGE_MAX).contains(&file.len()) || &file[..4] != b"APKG" { return Err(Error::BadFormat); }
    if le16(&file[4..6]) != 1 || le16(&file[6..8]) != 128 || le16(&file[8..10]) != 1
        || le16(&file[10..12]) != 0 || !canonical_id(&file[12..44])
        || file[120..128].iter().any(|&b| b != 0) { return Err(Error::BadFormat); }
    let version = le64(&file[44..52]);
    let n = le32(&file[52..56]) as usize;
    if version == 0 || !(1..=4096).contains(&n) || file.len() != 192 + n { return Err(Error::BadFormat); }
    if sha256(&file[128..128+n]) != file[56..88] { return Err(Error::BadFormat); }
    Ok(Package { id: array32(&file[12..44]), version, signer_id: array32(&file[88..120]),
        full_digest: sha256(file), file })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub id: [u8; 32],
    pub generation: u8,
    pub subordinate: [u8; 32],
    pub minimum: u64,
    pub allow: bool,
    pub revoked_count: u8,
    pub revoked: [[u8; 32]; 8],
}
impl Policy {
    pub fn is_revoked(&self, full_digest: &[u8; 32]) -> bool {
        self.revoked[..self.revoked_count as usize].contains(full_digest)
    }
    /// Host/receiver preflight for a *new root-signed intent*, not an unsigned
    /// policy amendment: ninth distinct digest refuses before any mutation.
    pub fn add_revocation(&mut self, digest: [u8; 32]) -> Result<bool, Error> {
        if digest == [0; 32] { return Err(Error::BadFormat); }
        if self.is_revoked(&digest) { return Ok(false); }
        let count = self.revoked_count as usize;
        if count == self.revoked.len() { return Err(Error::NoSpace); }
        let mut at = 0;
        while at < count && self.revoked[at] < digest { at += 1; }
        for i in (at..count).rev() { self.revoked[i+1] = self.revoked[i]; }
        self.revoked[at] = digest;
        self.revoked_count += 1;
        Ok(true)
    }
}

pub fn parse_policy(file: &[u8]) -> Result<Policy, Error> {
    if file.len() != POLICY_BYTES { return Err(Error::BadFormat); }
    if &file[..4] != b"APOL" || le16(&file[4..6]) != 1 || le16(&file[6..8]) != 448
        || !canonical_id(&file[16..48]) || file[122..128].iter().any(|&b| b != 0)
        || file[384..448].iter().any(|&b| b != 0) { return Err(Error::BadFormat); }
    let generation = le64(&file[8..16]);
    let minimum = le64(&file[112..120]);
    let count = file[121] as usize;
    if !(1..=MAX_POLICIES as u64).contains(&generation) || minimum == 0
        || !matches!(file[120], 1 | 2) || count > 8 { return Err(Error::BadFormat); }
    let key = array32(&file[48..80]);
    // ZIP-215 decompression alone admits aliases: require exact re-encoding.
    if key == ROOT || !canonical_public_key(&key) || sha256(&key) != file[80..112] {
        return Err(Error::BadFormat);
    }
    let mut revoked = [[0u8; 32]; 8];
    let mut previous = [0u8; 32];
    for (i, slot) in revoked.iter_mut().enumerate() {
        slot.copy_from_slice(&file[128+i*32..160+i*32]);
        if i < count {
            if *slot == [0; 32] || (i > 0 && previous >= *slot) { return Err(Error::BadFormat); }
            previous = *slot;
        } else if *slot != [0; 32] { return Err(Error::BadFormat); }
    }
    let mut message = [0u8; 18 + 448];
    message[..18].copy_from_slice(POL_DOMAIN);
    message[18..].copy_from_slice(&file[..448]);
    let sig: &[u8; 64] = file[448..].try_into().map_err(|_| Error::BadFormat)?;
    if !strict_verify(&ROOT, &message, sig) { return Err(Error::BadSignature); }
    Ok(Policy { id: array32(&file[16..48]), generation: generation as u8,
        subordinate: key, minimum, allow: file[120] == 1, revoked_count: count as u8, revoked })
}

/// Ordered verified policy history for the single bounded namespace. Poison
/// is managed by the filesystem caller; this pure core refuses gaps/replay.
pub struct Chain {
    pub current: Option<Policy>,
    pub count: u8,
    id: Option<[u8; 32]>,
    retired: [[u8; 32]; 4],
    retired_len: u8,
    historically_allowed: [[u8; 32]; 4],
    allowed_len: u8,
}
impl Chain {
    pub const fn new() -> Self { Self { current: None, count: 0, id: None,
        retired: [[0; 32]; 4], retired_len: 0,
        historically_allowed: [[0; 32]; 4], allowed_len: 0 } }
    pub fn id(&self) -> Option<[u8; 32]> { self.id }
    pub fn historical_key(&self, key_id: &[u8; 32]) -> Option<[u8; 32]> {
        self.historically_allowed[..self.allowed_len as usize].iter()
            .find(|k| sha256(*k) == *key_id).copied()
    }
    pub fn add(&mut self, p: Policy) -> Result<(), Error> {
        if self.count >= MAX_POLICIES { return Err(Error::NoSpace); }
        if p.generation != self.count + 1 || self.id.is_some_and(|id| id != p.id) { return Err(Error::Corrupt); }
        if let Some(prev) = self.current {
            if p.minimum < prev.minimum { return Err(Error::Corrupt); }
            for digest in &prev.revoked[..prev.revoked_count as usize] {
                if !p.is_revoked(digest) { return Err(Error::Corrupt); }
            }
            if !prev.allow || prev.subordinate != p.subordinate {
                self.retired[self.retired_len as usize] = prev.subordinate;
                self.retired_len += 1;
            }
        }
        if p.allow && self.retired[..self.retired_len as usize].contains(&p.subordinate) {
            return Err(Error::Corrupt);
        }
        if p.allow && !self.historically_allowed[..self.allowed_len as usize].contains(&p.subordinate) {
            self.historically_allowed[self.allowed_len as usize] = p.subordinate;
            self.allowed_len += 1;
        }
        self.id = Some(p.id);
        self.current = Some(p);
        self.count += 1;
        Ok(())
    }

    pub fn verified_signer(&self, pkg: &Package<'_>, scratch: &mut [u8; VERIFY_SCRATCH]) -> Result<(), Error> {
        if self.id.is_some_and(|id| id != pkg.id) { return Err(Error::Collision); }
        if pkg.signer_id == ROOT_ID { pkg.verify(&ROOT, scratch) }
        else if let Some(key) = self.historical_key(&pkg.signer_id) { pkg.verify(&key, scratch) }
        else { Err(Error::UnknownSigner) }
    }

    pub fn eligible(&self, pkg: &Package<'_>, scratch: &mut [u8; VERIFY_SCRATCH]) -> Result<(), Error> {
        self.verified_signer(pkg, scratch)?;
        if let Some(p) = &self.current {
            if pkg.version < p.minimum { return Err(Error::Downgrade); }
            if p.is_revoked(&pkg.full_digest) { return Err(Error::Revoked); }
            if pkg.signer_id != ROOT_ID && (!p.allow || pkg.signer_id != sha256(&p.subordinate)) {
                return Err(Error::Revoked);
            }
        }
        Ok(())
    }
}

/// Select an APB1 candidate key from the same receiver-verified APKG v1
/// policy chain. Inputs are unauthenticated bundle-header claims; callers
/// must compare them with the claim returned by the cryptographic APB1
/// verifier before any mutation.
pub fn apb1_select_key(
    chain: &Chain,
    package_id: &[u8; 32],
    signer_id: &[u8; 32],
    version: u64,
) -> Result<[u8; 32], Error> {
    if chain.id().is_some_and(|id| id != *package_id) {
        return Err(Error::Collision);
    }
    let key = if *signer_id == ROOT_ID {
        ROOT
    } else {
        chain
            .historical_key(signer_id)
            .ok_or(Error::UnknownSigner)?
    };
    if sha256(&key) != *signer_id {
        return Err(Error::UnknownSigner);
    }
    if let Some(current) = chain.current {
        if current.id != *package_id {
            return Err(Error::Collision);
        }
        if version < current.minimum {
            return Err(Error::Downgrade);
        }
        if *signer_id != ROOT_ID
            && (!current.allow || *signer_id != sha256(&current.subordinate))
        {
            return Err(Error::Revoked);
        }
    } else if *signer_id != ROOT_ID {
        return Err(Error::UnknownSigner);
    }
    Ok(key)
}

/// Apply current APKG v1 revocation/minimum/signer policy to a cryptographically
/// verified APB1 claim and whole-bundle digest. This is a policy decision only;
/// it creates no filesystem or launch authority.
pub fn apb1_check_eligible(
    chain: &Chain,
    package_id: &[u8; 32],
    signer_id: &[u8; 32],
    version: u64,
    bundle_digest: &[u8; 32],
) -> Result<(), Error> {
    let _key = apb1_select_key(chain, package_id, signer_id, version)?;
    if chain
        .current
        .is_some_and(|current| current.is_revoked(bundle_digest))
    {
        return Err(Error::Revoked);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage { pub id: [u8; 32], pub version: u64, pub full_digest: [u8; 32] }
pub struct History { pub count: u8, pub latest: Option<Stage> }
impl History {
    pub const fn new() -> Self { Self { count: 0, latest: None } }
    pub fn ingest(&mut self, pkg: &Package<'_>, chain: &Chain, scratch: &mut [u8; VERIFY_SCRATCH]) -> Result<(), Error> {
        if self.count >= MAX_STAGES { return Err(Error::NoSpace); }
        chain.verified_signer(pkg, scratch)?;
        if self.latest.is_some_and(|prev| prev.id != pkg.id || pkg.version <= prev.version) {
            return Err(Error::Corrupt);
        }
        self.count += 1;
        self.latest = Some(Stage { id: pkg.id, version: pkg.version, full_digest: pkg.full_digest });
        Ok(())
    }
    /// Stage-eligible is not installed or active; only the latest stage is
    /// reconsidered against the *current* policy on every fresh scan.
    pub fn decision(&self, pkg: &Package<'_>, chain: &Chain, scratch: &mut [u8; VERIFY_SCRATCH]) -> Result<(), Error> {
        if self.latest != Some(Stage { id: pkg.id, version: pkg.version, full_digest: pkg.full_digest }) {
            return Err(Error::Corrupt);
        }
        chain.eligible(pkg, scratch)
    }
    pub fn stage_candidate(&self, pkg: &Package<'_>, chain: &Chain, scratch: &mut [u8; VERIFY_SCRATCH]) -> Result<bool, Error> {
        chain.eligible(pkg, scratch)?;
        if let Some(prev) = self.latest {
            if pkg.id != prev.id { return Err(Error::Collision); }
            if pkg.version < prev.version { return Err(Error::Downgrade); }
            if pkg.version == prev.version {
                return if pkg.full_digest == prev.full_digest { Ok(false) } else { Err(Error::Conflict) };
            }
        }
        if self.count >= MAX_STAGES { return Err(Error::NoSpace); }
        Ok(true)
    }
}
