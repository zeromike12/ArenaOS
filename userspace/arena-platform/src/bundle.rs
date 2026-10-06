//! APB1 (ArenaOS Application Bundle v1): bounded metadata verification and
//! streaming per-file integrity checking (ADR-0080).
//!
//! Wire contract:
//!
//! ```text
//!  0..64                           APB1 header
//! 64..576                          canonical 512-byte AMF1 manifest
//! 576..576+count*144               sorted fixed-width file table
//! metadata_end..metadata_end+64    Ed25519 signature
//! after signature                  packed file bytes in table order
//! ```
//!
//! The signature covers `SIGN_DOMAIN || header || manifest || file table`.
//! Each table row signs a canonical relative path, kind, exact byte length,
//! and SHA-256. Payload data is read through `BundleSource` in at most 4 KiB
//! chunks; neither the verifier nor the kernel needs a whole-application
//! buffer. The bounded metadata/signature workspace is about 10 KiB and must
//! live in caller-owned static/BSS or runtime-managed memory, not on ArenaOS's
//! current one-page initial stack.
//!
//! Signature verification is cryptographic only. The caller must separately
//! resolve the signer through its receiver-verified policy, perform AFS2
//! install/activation transactions, and obtain a held executable/Image cap.
//! `VerifiedBundle` is never launch authority.

use crate::manifest::{self, MANIFEST_BYTES, Manifest};
use arena_phase84_crypto_audit::{Sha256State, sha256, strict_verify};

pub const HEADER_BYTES: usize = 64;
pub const FILE_RECORD_BYTES: usize = 144;
pub const MAX_FILES: usize = 64;
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
pub const IO_CHUNK_BYTES: usize = 4096;
pub const SIGN_DOMAIN: &[u8] = b"ArenaOS.application-bundle.v1\0";
const MAGIC: &[u8; 4] = b"APB1";
const VERSION: u16 = 1;

pub const MAX_METADATA_BYTES: usize = HEADER_BYTES + MANIFEST_BYTES + MAX_FILES * FILE_RECORD_BYTES;
pub const MAX_SIGNED_BYTES: usize = SIGN_DOMAIN.len() + MAX_METADATA_BYTES;
pub const WORKSPACE_BYTES: usize = MAX_SIGNED_BYTES + IO_CHUNK_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Io,
    BadHeader,
    UnsupportedVersion,
    Bounds,
    NonCanonical,
    BadManifest,
    BadPath,
    ReservedPath,
    DuplicatePath,
    PathConflict,
    BadFileTable,
    UnknownSigner,
    BadSignature,
    BadFileHash,
}

/// Random-access source supplied by the trusted installer/filesystem client.
/// It must fill the complete destination or return an error; a path or file
/// name is not authority and is not part of this interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceError;

pub trait BundleSource {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Executable,
    Resource,
}
impl FileKind {
    fn from_byte(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Executable),
            2 => Some(Self::Resource),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileEntry {
    kind: FileKind,
    path_len: u16,
    path: [u8; 96],
    size: u64,
    digest: [u8; 32],
}
impl FileEntry {
    pub fn kind(&self) -> FileKind {
        self.kind
    }
    pub fn path(&self) -> &[u8] {
        &self.path[..self.path_len as usize]
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn sha256(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// Reusable fixed-size workspace. The caller must place it in persistent
/// storage or a runtime allocation; do not construct it as an entry-stack
/// local in current ArenaOS applications.
pub struct Workspace {
    signed: [u8; MAX_SIGNED_BYTES],
    chunk: [u8; IO_CHUNK_BYTES],
}

/// Unauthenticated APB1 identity fields used only to select a candidate key
/// from a trusted policy namespace. Every field is rechecked after signature
/// verification; this record itself never authorizes installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BundleClaim {
    pub application_id: [u8; 32],
    pub package_id: [u8; 32],
    pub version: u64,
    pub signer_id: [u8; 32],
}
impl Workspace {
    pub const fn new() -> Self {
        Self {
            signed: [0; MAX_SIGNED_BYTES],
            chunk: [0; IO_CHUNK_BYTES],
        }
    }

    /// Parse only the bounded header and manifest to select a candidate key
    /// from trusted signer policy. The returned names/version are untrusted
    /// claims and must be bound again to the verified bundle before use.
    pub fn inspect<S: BundleSource>(&mut self, source: &mut S) -> Result<BundleClaim, Error> {
        if source.len() < (HEADER_BYTES + MANIFEST_BYTES + FILE_RECORD_BYTES + 64 + 1) as u64 {
            return Err(Error::BadHeader);
        }
        let start = SIGN_DOMAIN.len();
        let header_end = start + HEADER_BYTES;
        source
            .read_exact_at(0, &mut self.signed[start..header_end])
            .map_err(|_| Error::Io)?;
        let header = &self.signed[start..header_end];
        if &header[..4] != MAGIC {
            return Err(Error::BadHeader);
        }
        if le16(&header[4..6]) != VERSION || le16(&header[6..8]) as usize != HEADER_BYTES {
            return Err(Error::UnsupportedVersion);
        }
        let manifest_bytes = le32(&header[8..12]) as usize;
        let file_count = le32(&header[12..16]) as usize;
        let table_bytes = le64(&header[16..24]);
        let payload_bytes = le64(&header[24..32]);
        if manifest_bytes != MANIFEST_BYTES
            || !(1..=MAX_FILES).contains(&file_count)
            || table_bytes != (file_count as u64) * FILE_RECORD_BYTES as u64
            || payload_bytes == 0
            || payload_bytes > MAX_TOTAL_BYTES
        {
            return Err(Error::Bounds);
        }
        let metadata_bytes = HEADER_BYTES
            .checked_add(manifest_bytes)
            .and_then(|n| n.checked_add(usize::try_from(table_bytes).ok()?))
            .ok_or(Error::Bounds)?;
        if metadata_bytes > MAX_METADATA_BYTES
            || (metadata_bytes as u64)
                .checked_add(64)
                .and_then(|n| n.checked_add(payload_bytes))
                != Some(source.len())
        {
            return Err(Error::Bounds);
        }
        let mut signer_id = [0; 32];
        signer_id.copy_from_slice(&header[32..64]);
        let manifest_start = header_end;
        let manifest_end = manifest_start + MANIFEST_BYTES;
        source
            .read_exact_at(
                HEADER_BYTES as u64,
                &mut self.signed[manifest_start..manifest_end],
            )
            .map_err(|_| Error::Io)?;
        let manifest = Manifest::parse(&self.signed[manifest_start..manifest_end])
            .map_err(|_| Error::BadManifest)?;
        Ok(BundleClaim {
            application_id: *manifest.application_id(),
            package_id: *manifest.package_id(),
            version: manifest.version(),
            signer_id,
        })
    }

    /// Verify exact APB1 metadata and stream every file through SHA-256.
    /// `trusted_key` must already be selected by the caller's current signer
    /// policy; the signed signer ID is checked against its SHA-256 fingerprint.
    pub fn verify<'a, S: BundleSource>(
        &'a mut self,
        source: &mut S,
        trusted_key: &[u8; 32],
    ) -> Result<VerifiedBundle<'a>, Error> {
        if source.len() < (HEADER_BYTES + MANIFEST_BYTES + 1 + 64) as u64 {
            return Err(Error::BadHeader);
        }

        let domain_len = SIGN_DOMAIN.len();
        let meta_start = domain_len;
        let header_end = meta_start + HEADER_BYTES;
        source
            .read_exact_at(0, &mut self.signed[meta_start..header_end])
            .map_err(|_| Error::Io)?;
        let mut header = [0u8; HEADER_BYTES];
        header.copy_from_slice(&self.signed[meta_start..header_end]);
        if &header[..4] != MAGIC {
            return Err(Error::BadHeader);
        }
        if le16(&header[4..6]) != VERSION || le16(&header[6..8]) as usize != HEADER_BYTES {
            return Err(Error::UnsupportedVersion);
        }
        let manifest_bytes = le32(&header[8..12]) as usize;
        let file_count = le32(&header[12..16]) as usize;
        let table_bytes = le64(&header[16..24]);
        let payload_bytes = le64(&header[24..32]);
        if manifest_bytes != MANIFEST_BYTES
            || !(1..=MAX_FILES).contains(&file_count)
            || table_bytes != (file_count as u64) * FILE_RECORD_BYTES as u64
            || payload_bytes == 0
            || payload_bytes > MAX_TOTAL_BYTES
        {
            return Err(Error::Bounds);
        }
        let table_bytes = usize::try_from(table_bytes).map_err(|_| Error::Bounds)?;
        let metadata_bytes = HEADER_BYTES
            .checked_add(manifest_bytes)
            .and_then(|n| n.checked_add(table_bytes))
            .ok_or(Error::Bounds)?;
        if metadata_bytes > MAX_METADATA_BYTES {
            return Err(Error::Bounds);
        }
        let metadata_end = meta_start
            .checked_add(metadata_bytes)
            .ok_or(Error::Bounds)?;
        source
            .read_exact_at(
                HEADER_BYTES as u64,
                &mut self.signed[header_end..metadata_end],
            )
            .map_err(|_| Error::Io)?;

        let signature_offset = metadata_bytes as u64;
        let payload_offset = signature_offset.checked_add(64).ok_or(Error::Bounds)?;
        let expected_length = payload_offset
            .checked_add(payload_bytes)
            .ok_or(Error::Bounds)?;
        if source.len() != expected_length {
            return Err(Error::Bounds);
        }
        let mut signature = [0u8; 64];
        source
            .read_exact_at(signature_offset, &mut signature)
            .map_err(|_| Error::Io)?;

        if sha256(trusted_key) != header[32..64] {
            return Err(Error::UnknownSigner);
        }
        self.signed[..domain_len].copy_from_slice(SIGN_DOMAIN);
        let signed_len = domain_len
            .checked_add(metadata_bytes)
            .ok_or(Error::Bounds)?;
        if !strict_verify(trusted_key, &self.signed[..signed_len], &signature) {
            return Err(Error::BadSignature);
        }

        let metadata = &self.signed[meta_start..metadata_end];
        let manifest = Manifest::parse(&metadata[HEADER_BYTES..HEADER_BYTES + MANIFEST_BYTES])
            .map_err(|_| Error::BadManifest)?;
        validate_file_table(metadata, file_count, payload_bytes, &manifest)?;

        // The full digest names this exact immutable bundle. It is distinct
        // from the manifest signature and is computed incrementally while
        // each signed per-file digest is checked.
        let mut bundle_hash = Sha256State::new();
        bundle_hash.update(metadata);
        bundle_hash.update(&signature);
        let mut consumed = 0u64;
        for index in 0..file_count {
            let entry = entry_at(metadata, index)?;
            let mut file_hash = Sha256State::new();
            let mut offset_in_file = 0u64;
            while offset_in_file < entry.size {
                let take = (entry.size - offset_in_file).min(IO_CHUNK_BYTES as u64) as usize;
                let source_offset = payload_offset
                    .checked_add(consumed)
                    .and_then(|n| n.checked_add(offset_in_file))
                    .ok_or(Error::Bounds)?;
                source
                    .read_exact_at(source_offset, &mut self.chunk[..take])
                    .map_err(|_| Error::Io)?;
                file_hash.update(&self.chunk[..take]);
                bundle_hash.update(&self.chunk[..take]);
                offset_in_file += take as u64;
            }
            if file_hash.finalize() != entry.digest {
                return Err(Error::BadFileHash);
            }
            consumed = consumed.checked_add(entry.size).ok_or(Error::Bounds)?;
        }
        if consumed != payload_bytes {
            return Err(Error::BadFileTable);
        }
        let bundle_digest = bundle_hash.finalize();

        Ok(VerifiedBundle {
            metadata: &self.signed[meta_start..metadata_end],
            manifest,
            file_count,
            payload_offset,
            payload_bytes,
            signer_id: array32(&header[32..64]),
            bundle_digest,
            signature,
        })
    }
}
impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

/// A cryptographically valid APB1 view, borrowing the fixed workspace.
/// It is not proof of current signer policy, durable installation, activation,
/// or executable authority.
pub struct VerifiedBundle<'a> {
    metadata: &'a [u8],
    manifest: Manifest,
    file_count: usize,
    payload_offset: u64,
    payload_bytes: u64,
    signer_id: [u8; 32],
    bundle_digest: [u8; 32],
    signature: [u8; 64],
}
impl VerifiedBundle<'_> {
    /// Exact canonical signed metadata bytes (header, manifest and file table).
    pub fn metadata(&self) -> &[u8] {
        self.metadata
    }
    /// Signature over `SIGN_DOMAIN || metadata`; persistence stores this next
    /// to `metadata` so the installed bundle can be independently rechecked.
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn file_count(&self) -> usize {
        self.file_count
    }
    pub fn payload_offset(&self) -> u64 {
        self.payload_offset
    }
    pub fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }
    pub fn signer_id(&self) -> &[u8; 32] {
        &self.signer_id
    }
    pub fn bundle_digest(&self) -> &[u8; 32] {
        &self.bundle_digest
    }
    pub fn file(&self, index: usize) -> Option<FileEntry> {
        (index < self.file_count)
            .then(|| entry_at(self.metadata, index).ok())
            .flatten()
    }
    pub fn find(&self, path: &[u8]) -> Option<(usize, FileEntry)> {
        (0..self.file_count).find_map(|i| {
            let entry = self.file(i)?;
            (entry.path() == path).then_some((i, entry))
        })
    }
    /// Offset of this payload in the bundle source. Files are packed exactly
    /// in canonical file-table order with no gaps or trailing bytes.
    pub fn file_offset(&self, index: usize) -> Option<u64> {
        if index >= self.file_count {
            return None;
        }
        let mut preceding = 0u64;
        for i in 0..index {
            preceding = preceding.checked_add(self.file(i)?.size)?;
        }
        self.payload_offset.checked_add(preceding)
    }
}

fn validate_file_table(
    metadata: &[u8],
    file_count: usize,
    payload_bytes: u64,
    manifest: &Manifest,
) -> Result<(), Error> {
    let mut previous_path: Option<[u8; 96]> = None;
    let mut previous_len = 0usize;
    let mut total = 0u64;
    let mut executable_count = 0usize;
    let mut entry_found = false;
    let icon_path = fixed_path(manifest.icon_path());
    let manifest_entry = fixed_path(manifest.entry_path());
    let mut icon_found = icon_path.is_none();

    for index in 0..file_count {
        let entry = entry_at(metadata, index)?;
        let path = entry.path();
        if !manifest::valid_relative_path(path) {
            return Err(Error::BadPath);
        }
        // ADR-0082 persists signed metadata/signature in this root-level file.
        // Reject the reserved path before any install-side AFS2 mutation.
        if path == b"APB1.record" {
            return Err(Error::ReservedPath);
        }
        if let Some(prior) = previous_path {
            let prior = &prior[..previous_len];
            if prior == path {
                return Err(Error::DuplicatePath);
            }
            if prior > path {
                return Err(Error::NonCanonical);
            }
            if path.len() > prior.len()
                && path.starts_with(prior)
                && path.get(prior.len()) == Some(&b'/')
            {
                return Err(Error::PathConflict);
            }
        }
        previous_path = Some(entry.path);
        previous_len = entry.path_len as usize;
        if entry.size == 0 || entry.size > MAX_FILE_BYTES {
            return Err(Error::Bounds);
        }
        total = total.checked_add(entry.size).ok_or(Error::Bounds)?;
        if total > MAX_TOTAL_BYTES {
            return Err(Error::Bounds);
        }
        if entry.kind == FileKind::Executable {
            executable_count += 1;
            if manifest_entry == Some(path) {
                entry_found = true;
            }
        }
        if entry.kind == FileKind::Resource && icon_path == Some(path) {
            icon_found = true;
        }
    }
    if executable_count == 0 || !entry_found || !icon_found || total != payload_bytes {
        return Err(Error::BadFileTable);
    }
    Ok(())
}

fn fixed_path<const N: usize>(field: &[u8; N]) -> Option<&[u8]> {
    let end = field.iter().position(|&b| b == 0)?;
    (end > 0 && field[end..].iter().all(|&b| b == 0)).then_some(&field[..end])
}

pub(crate) fn entry_at(metadata: &[u8], index: usize) -> Result<FileEntry, Error> {
    let table_start = HEADER_BYTES + MANIFEST_BYTES;
    let start = table_start
        .checked_add(index.checked_mul(FILE_RECORD_BYTES).ok_or(Error::Bounds)?)
        .ok_or(Error::Bounds)?;
    let record = metadata
        .get(start..start + FILE_RECORD_BYTES)
        .ok_or(Error::Bounds)?;
    if record[0] != 1 && record[0] != 2 {
        return Err(Error::BadFileTable);
    }
    let kind = FileKind::from_byte(record[0]).ok_or(Error::BadFileTable)?;
    let path_len = le16(&record[1..3]) as usize;
    if !(1..=95).contains(&path_len) || record[3..8].iter().any(|&b| b != 0) {
        return Err(Error::NonCanonical);
    }
    let mut size_bytes = [0u8; 8];
    size_bytes.copy_from_slice(&record[8..16]);
    let size = u64::from_le_bytes(size_bytes);
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&record[16..48]);
    let mut path = [0u8; 96];
    path.copy_from_slice(&record[48..144]);
    if path[path_len..].iter().any(|&b| b != 0) {
        return Err(Error::NonCanonical);
    }
    Ok(FileEntry {
        kind,
        path_len: path_len as u16,
        path,
        size,
        digest,
    })
}

fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}
fn array32(b: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    out.copy_from_slice(b);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT_PUBLIC: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];

    struct SliceSource<'a> {
        bytes: &'a [u8],
        largest_read: usize,
    }
    impl BundleSource for SliceSource<'_> {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
            self.largest_read = self.largest_read.max(out.len());
            let start = usize::try_from(offset).map_err(|_| SourceError)?;
            let end = start.checked_add(out.len()).ok_or(SourceError)?;
            let src = self.bytes.get(start..end).ok_or(SourceError)?;
            out.copy_from_slice(src);
            Ok(())
        }
    }

    const VALID: &[u8] = include_bytes!("../tests/data/editor.apb1");
    const TRAVERSAL: &[u8] = include_bytes!("../tests/data/traversal.apb1");
    const DUPLICATE: &[u8] = include_bytes!("../tests/data/duplicate.apb1");
    const PATH_CONFLICT: &[u8] = include_bytes!("../tests/data/prefix-conflict.apb1");
    const LARGE: &[u8] = include_bytes!("../tests/data/large.apb1");
    const NONCANONICAL: &[u8] = include_bytes!("../tests/data/noncanonical.apb1");
    const RESERVED_RECORD: &[u8] = include_bytes!("../tests/data/reserved-record.apb1");
    const OVERSIZED_FILE: &[u8] = include_bytes!("../tests/data/oversized-file.apb1");
    const OVERSIZED_TOTAL: &[u8] = include_bytes!("../tests/data/oversized-total.apb1");

    #[test]
    fn signed_multi_file_bundle_verifies_and_exposes_only_descriptive_records() {
        let mut src = SliceSource {
            bytes: VALID,
            largest_read: 0,
        };
        let mut workspace = Workspace::new();
        let bundle = workspace.verify(&mut src, &ROOT_PUBLIC).unwrap();
        assert_eq!(bundle.file_count(), 3);
        assert_eq!(bundle.payload_bytes(), 32);
        assert_eq!(
            &bundle.manifest().application_id()[..16],
            b"com.arena.editor"
        );
        assert_eq!(bundle.manifest().version(), 42);
        assert!(bundle.manifest().allows_multiple_instances());
        assert_eq!(bundle.file(0).unwrap().path(), b"bin/editor");
        assert_eq!(bundle.file(1).unwrap().path(), b"bin/helper");
        assert_eq!(bundle.file(2).unwrap().kind(), FileKind::Resource);
        assert_eq!(bundle.find(b"bin/helper").unwrap().0, 1);
        assert_eq!(bundle.file_offset(0), Some(bundle.payload_offset()));
        assert_eq!(bundle.file_offset(1), Some(bundle.payload_offset() + 18));
        assert_eq!(bundle.file_offset(2), Some(bundle.payload_offset() + 25));
        assert_eq!(bundle.signer_id(), &sha256(&ROOT_PUBLIC));
        assert_eq!(bundle.bundle_digest(), &sha256(VALID));
        assert!(src.largest_read <= MAX_METADATA_BYTES.max(IO_CHUNK_BYTES));
        assert!(src.largest_read < VALID.len());
    }

    #[test]
    fn large_payload_is_hashed_in_fixed_four_kibibyte_reads() {
        assert_eq!(core::mem::size_of::<Workspace>(), WORKSPACE_BYTES);
        assert_eq!(WORKSPACE_BYTES, 13_918);
        let mut src = SliceSource {
            bytes: LARGE,
            largest_read: 0,
        };
        let mut workspace = Workspace::new();
        let bundle = workspace.verify(&mut src, &ROOT_PUBLIC).unwrap();
        assert_eq!(bundle.file_count(), 1);
        assert_eq!(bundle.payload_bytes(), 32 * 1024);
        assert_eq!(bundle.file(0).unwrap().path(), b"bin/editor");
        assert_eq!(src.largest_read, IO_CHUNK_BYTES);
        assert!(src.largest_read < LARGE.len());
    }

    #[test]
    fn receiver_rejects_wrong_key_signature_and_tampered_payload() {
        let mut wrong_key = [0u8; 32];
        wrong_key[0] = 1;
        let mut src = SliceSource {
            bytes: VALID,
            largest_read: 0,
        };
        let mut workspace = Workspace::new();
        assert!(matches!(
            workspace.verify(&mut src, &wrong_key),
            Err(Error::UnknownSigner)
        ));

        let mut corrupted = VALID.to_vec();
        *corrupted.last_mut().unwrap() ^= 1;
        let mut src = SliceSource {
            bytes: &corrupted,
            largest_read: 0,
        };
        assert!(matches!(
            workspace.verify(&mut src, &ROOT_PUBLIC),
            Err(Error::BadFileHash)
        ));

        let mut bad_signature = VALID.to_vec();
        let signature_at = bad_signature.len() - 32 - 64;
        bad_signature[signature_at] ^= 1;
        let mut src = SliceSource {
            bytes: &bad_signature,
            largest_read: 0,
        };
        assert!(matches!(
            workspace.verify(&mut src, &ROOT_PUBLIC),
            Err(Error::BadSignature)
        ));
    }

    #[test]
    fn valid_signatures_do_not_excuse_noncanonical_or_unsafe_metadata() {
        for (fixture, expected) in [
            (TRAVERSAL, Error::BadPath),
            (DUPLICATE, Error::DuplicatePath),
            (PATH_CONFLICT, Error::PathConflict),
            (NONCANONICAL, Error::BadManifest),
            (RESERVED_RECORD, Error::ReservedPath),
            (OVERSIZED_FILE, Error::Bounds),
            (OVERSIZED_TOTAL, Error::Bounds),
        ] {
            let mut src = SliceSource {
                bytes: fixture,
                largest_read: 0,
            };
            let mut workspace = Workspace::new();
            assert!(
                matches!(workspace.verify(&mut src, &ROOT_PUBLIC), Err(error) if error == expected)
            );
        }
    }

    #[test]
    fn exact_envelope_rejects_trailing_or_truncated_payload() {
        let trailing = [VALID, b"x"].concat();
        for bytes in [&VALID[..VALID.len() - 1], trailing.as_slice()] {
            let mut src = SliceSource {
                bytes,
                largest_read: 0,
            };
            let mut workspace = Workspace::new();
            assert!(matches!(
                workspace.verify(&mut src, &ROOT_PUBLIC),
                Err(Error::Bounds)
            ));
        }
    }
}
