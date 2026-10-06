//! APB1-to-AFS2 immutable install foundation (ADR-0082).
//!
//! This is a bounded filesystem-engine core, not a guest filesd endpoint. The
//! caller must hold exact AFS2 authority to the protected install/staging
//! roots and must resolve `trusted_key` through the current receiver-side
//! signer policy. IDs, object numbers, path components and receipts are data,
//! never authority. Caller-owned `InstallWorkspace` is roughly 14 KiB and
//! `RegistryScanWorkspace` roughly 9.75 KiB; with the verifier they belong in
//! static/BSS or runtime-managed memory rather than the initial one-page stack.
//! Double-buffering the fixed catalog costs additional manager-owned memory.
//!
//! Files are written below the private staging root. A durable record stores
//! only signed metadata and signature; file payloads are copied to their
//! canonical paths. The entire staged tree is read back through the same
//! APB1 verifier before one AFS2 directory rename publishes the immutable
//! `<application-id>/<version>` tree. The registry must scan only the install
//! root and call [`verify_installed`] before using any descriptive metadata.

use crate::bundle::{
    self, BundleClaim, BundleSource, Error as BundleError, FileEntry, MAX_FILES,
    MAX_METADATA_BYTES, SourceError, VerifiedBundle, Workspace,
};
use crate::manifest::MANIFEST_BYTES;
use crate::package_policy::{self as apkg, Chain};
use crate::policy;
use crate::registry::{AppDefinition, AppRegistry};
use arena_afs2::{self as afs2, DIR, Device, FILE, NAME_MAX, Volume};

const RECORD_NAME: &[u8] = b"APB1.record";
const STAGE_PREFIX: &[u8] = b"apb1-";
const HEX: &[u8; 16] = b"0123456789abcdef";
const RECORD_MAX: usize = MAX_METADATA_BYTES + 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Bundle(BundleError),
    Policy(apkg::Error),
    Fs(afs2::Error),
    BadRecord,
    InvalidRoot,
    AlreadyInstalled,
    StagingCollision,
    ReadbackMismatch,
    SourceChangedOrShort,
    BadInstalledTree,
    NamespaceMismatch,
    ScanLimit,
    Registry(crate::registry::Error),
    Bounds,
}

/// Descriptive receipt returned after activation. `directory` is an AFS2
/// object ID, not a capability and not permission to launch the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallReceipt {
    pub application_id: [u8; 32],
    pub package_id: [u8; 32],
    pub version: u64,
    pub signer_id: [u8; 32],
    pub bundle_digest: [u8; 32],
    pub directory: u64,
}

/// Reusable streaming scratch. Must not be constructed on the initial guest
/// stack; the largest payload transfer is one 4 KiB chunk.
pub struct InstallWorkspace {
    record: [u8; RECORD_MAX],
    chunk: [u8; bundle::IO_CHUNK_BYTES],
}

/// Exact directory objects and timestamp selected by the trusted filesd-side
/// caller. These integers are names inside the already-authorized Volume;
/// they are not capabilities or authorization tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallTarget {
    pub applications_root: u64,
    pub staging_root: u64,
    pub wall_us: u64,
}
impl InstallTarget {
    pub const fn new(applications_root: u64, staging_root: u64, wall_us: u64) -> Self {
        Self {
            applications_root,
            staging_root,
            wall_us,
        }
    }
}

#[derive(Clone, Copy)]
enum InstallTrust<'a> {
    ExplicitKey,
    PolicyBound {
        chain: &'a Chain,
        claim: BundleClaim,
        digest: [u8; 32],
    },
}
impl InstallWorkspace {
    pub const fn new() -> Self {
        Self {
            record: [0; RECORD_MAX],
            chunk: [0; bundle::IO_CHUNK_BYTES],
        }
    }
}
impl Default for InstallWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

const MAX_STAGE_DEPTH: usize = 48;

#[derive(Clone, Copy)]
struct CleanupFrame {
    directory: u64,
    parent: u64,
    name: [u8; 96],
    name_len: u8,
    cursor: [u8; 96],
    cursor_len: u8,
}
impl CleanupFrame {
    const EMPTY: Self = Self {
        directory: 0,
        parent: 0,
        name: [0; 96],
        name_len: 0,
        cursor: [0; 96],
        cursor_len: 0,
    };
}

/// Fixed-depth postorder scratch for boot-time staging cleanup. Its bound is
/// derived from APB1's 95-byte complete path limit (at most 47 parent
/// components plus the staging root).
pub struct CleanupWorkspace {
    frames: [CleanupFrame; MAX_STAGE_DEPTH],
}
impl CleanupWorkspace {
    pub const fn new() -> Self {
        Self {
            frames: [CleanupFrame::EMPTY; MAX_STAGE_DEPTH],
        }
    }
}
impl Default for CleanupWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

const MAX_TREE_DEPTH: usize = 48;
#[derive(Clone, Copy)]
struct TreeFrame {
    directory: u64,
    path: [u8; 96],
    path_len: u8,
    cursor: [u8; 96],
    cursor_len: u8,
}
impl TreeFrame {
    const EMPTY: Self = Self {
        directory: 0,
        path: [0; 96],
        path_len: 0,
        cursor: [0; 96],
        cursor_len: 0,
    };
}

/// Caller-owned bounded scratch for exact installed-tree/registry validation.
/// Keep in manager static/runtime memory; never construct on the initial stack.
pub struct RegistryScanWorkspace {
    frames: [TreeFrame; MAX_TREE_DEPTH],
}
impl RegistryScanWorkspace {
    pub const fn new() -> Self {
        Self {
            frames: [TreeFrame::EMPTY; MAX_TREE_DEPTH],
        }
    }
}
impl Default for RegistryScanWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistryScanReport {
    pub application_directories: usize,
    pub version_directories: usize,
    pub verified_versions: usize,
    pub rejected_versions: usize,
    pub published_applications: usize,
}

/// Receiver-selected signer policy and caller-owned scan workspaces. The
/// context carries no filesystem or launch authority; it only binds this one
/// bounded catalog pass to the already-verified policy chain.
pub struct RegistryScanContext<'a> {
    verifier: &'a mut Workspace,
    install_scratch: &'a mut InstallWorkspace,
    tree_scratch: &'a mut RegistryScanWorkspace,
    chain: &'a Chain,
}
impl<'a> RegistryScanContext<'a> {
    pub fn new(
        verifier: &'a mut Workspace,
        install_scratch: &'a mut InstallWorkspace,
        tree_scratch: &'a mut RegistryScanWorkspace,
        chain: &'a Chain,
    ) -> Self {
        Self {
            verifier,
            install_scratch,
            tree_scratch,
            chain,
        }
    }
}

/// Remove incomplete or unactivated APB1 stage entries after a remount. Only
/// exact `apb1-` plus 64 lowercase hexadecimal storage labels are considered;
/// unknown names are left untouched. The caller must have trusted authority
/// to the private staging root. Re-running after any crash is safe.
pub fn purge_abandoned_staging<D: Device>(
    volume: &mut Volume<D>,
    staging_root: u64,
    cleanup: &mut CleanupWorkspace,
    wall_us: u64,
) -> Result<usize, Error> {
    require_directory(volume, staging_root)?;
    let mut after = [0u8; NAME_MAX];
    let mut after_len = 0usize;
    let mut removed = 0usize;
    loop {
        let mut entry = [afs2::Entry::EMPTY; 1];
        let count = volume
            .list(staging_root, &after[..after_len], &mut entry)
            .map_err(Error::Fs)?;
        if count == 0 {
            return Ok(removed);
        }
        let entry = entry[0];
        let name = entry.name();
        after[..name.len()].copy_from_slice(name);
        after_len = name.len();
        if !is_stage_label(name) {
            continue;
        }
        match entry.typ {
            FILE => volume
                .unlink(staging_root, name, wall_us)
                .map_err(Error::Fs)?,
            DIR => remove_stage_tree(volume, staging_root, name, entry.object, cleanup, wall_us)?,
            _ => return Err(Error::BadRecord),
        }
        removed += 1;
    }
}

fn remove_stage_tree<D: Device>(
    volume: &mut Volume<D>,
    parent: u64,
    name: &[u8],
    directory: u64,
    cleanup: &mut CleanupWorkspace,
    wall_us: u64,
) -> Result<(), Error> {
    if name.len() > 95
        || !afs2::valid_name(name)
        || volume.stat(directory).map_err(Error::Fs)?.typ != DIR
    {
        return Err(Error::BadRecord);
    }
    cleanup.frames.fill(CleanupFrame::EMPTY);
    let mut root_name = [0u8; 96];
    root_name[..name.len()].copy_from_slice(name);
    cleanup.frames[0] = CleanupFrame {
        directory,
        parent,
        name: root_name,
        name_len: name.len() as u8,
        ..CleanupFrame::EMPTY
    };
    let mut depth = 1usize;
    while depth != 0 {
        let frame = cleanup.frames[depth - 1];
        let mut entries = [afs2::Entry::EMPTY; 1];
        let count = volume
            .list(
                frame.directory,
                &frame.cursor[..usize::from(frame.cursor_len)],
                &mut entries,
            )
            .map_err(Error::Fs)?;
        if count == 0 {
            volume
                .rmdir(
                    frame.parent,
                    &frame.name[..usize::from(frame.name_len)],
                    wall_us,
                )
                .map_err(Error::Fs)?;
            depth -= 1;
            cleanup.frames[depth] = CleanupFrame::EMPTY;
            continue;
        }
        let entry = entries[0];
        let child_name = entry.name();
        if child_name.is_empty() || child_name.len() > 95 {
            return Err(Error::BadRecord);
        }
        let parent_directory = cleanup.frames[depth - 1].directory;
        {
            let frame = &mut cleanup.frames[depth - 1];
            frame.cursor.fill(0);
            frame.cursor[..child_name.len()].copy_from_slice(child_name);
            frame.cursor_len = child_name.len() as u8;
        }
        match entry.typ {
            FILE => volume
                .unlink(parent_directory, child_name, wall_us)
                .map_err(Error::Fs)?,
            DIR => {
                if depth == cleanup.frames.len() || !afs2::valid_name(child_name) {
                    return Err(Error::Bounds);
                }
                let mut stored_name = [0u8; 96];
                stored_name[..child_name.len()].copy_from_slice(child_name);
                cleanup.frames[depth] = CleanupFrame {
                    directory: entry.object,
                    parent: parent_directory,
                    name: stored_name,
                    name_len: child_name.len() as u8,
                    ..CleanupFrame::EMPTY
                };
                depth += 1;
            }
            _ => return Err(Error::BadRecord),
        }
    }
    Ok(())
}

fn is_stage_label(name: &[u8]) -> bool {
    name.len() == STAGE_PREFIX.len() + 64
        && name.starts_with(STAGE_PREFIX)
        && name[STAGE_PREFIX.len()..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

/// Verify, stage, read back, and atomically publish one immutable APB1
/// version. Per-object writes can leave private staging residue on I/O or
/// space failure; the staging root must remain outside registry discovery and
/// a trusted boot-time janitor must eventually reclaim abandoned entries.
/// Install using a key already selected by an external, receiver-verified
/// signer policy. The production service should prefer
/// [`install_with_policy`]; this lower-level entry point exists for callers
/// whose policy authority is outside this crate and for host proofs.
pub fn install<D: Device, S: BundleSource>(
    volume: &mut Volume<D>,
    verifier: &mut Workspace,
    scratch: &mut InstallWorkspace,
    source: &mut S,
    trusted_key: &[u8; 32],
    target: InstallTarget,
) -> Result<InstallReceipt, Error> {
    install_inner(
        volume,
        verifier,
        scratch,
        source,
        trusted_key,
        target,
        InstallTrust::ExplicitKey,
    )
}

/// Resolve the candidate package-ID/signer namespace through the existing
/// receiver policy chain, verify it, and bind that exact identity+digest
/// across a second pre-mutation verification and the durable staged readback.
pub fn install_with_policy<D: Device, S: BundleSource>(
    volume: &mut Volume<D>,
    verifier: &mut Workspace,
    scratch: &mut InstallWorkspace,
    source: &mut S,
    chain: &Chain,
    target: InstallTarget,
) -> Result<InstallReceipt, Error> {
    let claim = verifier.inspect(source).map_err(Error::Bundle)?;
    let key = policy::select_key(chain, &claim).map_err(Error::Policy)?;
    let verified = verifier.verify(source, &key).map_err(Error::Bundle)?;
    let authenticated = claim_from_bundle(&verified);
    if authenticated != claim {
        return Err(Error::SourceChangedOrShort);
    }
    policy::check_eligible(chain, &verified).map_err(Error::Policy)?;
    let trust = InstallTrust::PolicyBound {
        chain,
        claim: authenticated,
        digest: *verified.bundle_digest(),
    };
    install_inner(volume, verifier, scratch, source, &key, target, trust)
}

fn install_inner<D: Device, S: BundleSource>(
    volume: &mut Volume<D>,
    verifier: &mut Workspace,
    scratch: &mut InstallWorkspace,
    source: &mut S,
    trusted_key: &[u8; 32],
    target: InstallTarget,
    trust: InstallTrust<'_>,
) -> Result<InstallReceipt, Error> {
    let verified = verifier
        .verify(source, trusted_key)
        .map_err(Error::Bundle)?;
    check_trust_bundle(&trust, &verified)?;
    let applications_root = target.applications_root;
    let staging_root = target.staging_root;
    let wall_us = target.wall_us;
    let manifest = *verified.manifest();
    let application_id = *manifest.application_id();
    let package_id = *manifest.package_id();
    let version = manifest.version();
    let expected_signer = *verified.signer_id();
    let expected_digest = *verified.bundle_digest();
    let metadata_len = verified.metadata().len();
    let record_len = metadata_len.checked_add(64).ok_or(Error::Bounds)?;
    if record_len > scratch.record.len() || metadata_len > MAX_METADATA_BYTES {
        return Err(Error::Bounds);
    }
    scratch.record[..metadata_len].copy_from_slice(verified.metadata());
    scratch.record[metadata_len..record_len].copy_from_slice(verified.signature());

    require_directory(volume, applications_root)?;
    require_directory(volume, staging_root)?;
    let id_len = cstr_len(&application_id).ok_or(Error::BadRecord)?;
    let app_name = &application_id[..id_len];
    let mut version_storage = [0u8; 20];
    let version_name = decimal(version, &mut version_storage);
    let mut stage_storage = [0u8; 69];
    stage_storage[..STAGE_PREFIX.len()].copy_from_slice(STAGE_PREFIX);
    for (i, byte) in expected_digest.iter().enumerate() {
        stage_storage[STAGE_PREFIX.len() + i * 2] = HEX[(byte >> 4) as usize];
        stage_storage[STAGE_PREFIX.len() + i * 2 + 1] = HEX[(byte & 15) as usize];
    }
    let stage_name = &stage_storage[..STAGE_PREFIX.len() + 64];

    // Duplicate/collision refusals happen before any namespace mutation.
    let existing_app = lookup_optional(volume, applications_root, app_name)?;
    if let Some((app_dir, typ)) = existing_app {
        if typ != DIR {
            return Err(Error::InvalidRoot);
        }
        if lookup_optional(volume, app_dir, version_name)?.is_some() {
            return Err(Error::AlreadyInstalled);
        }
    }
    if lookup_optional(volume, staging_root, stage_name)?.is_some() {
        return Err(Error::StagingCollision);
    }

    let stage_dir = volume
        .mkdir(staging_root, stage_name, wall_us)
        .map_err(Error::Fs)?;
    let record_id = volume
        .create(stage_dir, RECORD_NAME, wall_us)
        .map_err(Error::Fs)?;
    write_all(volume, record_id, &scratch.record[..record_len], wall_us)?;

    for index in 0..verified.file_count() {
        let entry = verified.file(index).ok_or(Error::BadRecord)?;
        let offset = verified.file_offset(index).ok_or(Error::Bounds)?;
        copy_payload_file(volume, source, scratch, stage_dir, entry, offset, wall_us)?;
    }

    // Preserve only copyable descriptive facts before ending the borrow of the
    // verifier workspace; then verify the actual staged record/files afresh.
    let staged = verify_installed(volume, verifier, scratch, stage_dir, trusted_key)?;
    if staged.application_id != application_id
        || staged.package_id != package_id
        || staged.version != version
        || staged.signer_id != expected_signer
        || staged.bundle_digest != expected_digest
    {
        return Err(Error::ReadbackMismatch);
    }
    check_trust_receipt(&trust, &staged)?;

    let app_dir = match lookup_optional(volume, applications_root, app_name)? {
        Some((object, DIR)) => object,
        Some(_) => return Err(Error::InvalidRoot),
        None => volume
            .mkdir(applications_root, app_name, wall_us)
            .map_err(Error::Fs)?,
    };
    if lookup_optional(volume, app_dir, version_name)?.is_some() {
        return Err(Error::AlreadyInstalled);
    }
    let directory = volume
        .rename(staging_root, stage_name, app_dir, version_name, wall_us)
        .map_err(Error::Fs)?;

    Ok(InstallReceipt {
        directory,
        ..staged
    })
}

fn claim_from_bundle(bundle: &VerifiedBundle<'_>) -> BundleClaim {
    BundleClaim {
        application_id: *bundle.manifest().application_id(),
        package_id: *bundle.manifest().package_id(),
        version: bundle.manifest().version(),
        signer_id: *bundle.signer_id(),
    }
}

fn claim_from_receipt(receipt: &InstallReceipt) -> BundleClaim {
    BundleClaim {
        application_id: receipt.application_id,
        package_id: receipt.package_id,
        version: receipt.version,
        signer_id: receipt.signer_id,
    }
}

fn check_trust_bundle(trust: &InstallTrust<'_>, bundle: &VerifiedBundle<'_>) -> Result<(), Error> {
    check_trust_claim(trust, &claim_from_bundle(bundle), bundle.bundle_digest())
}

fn check_trust_receipt(trust: &InstallTrust<'_>, receipt: &InstallReceipt) -> Result<(), Error> {
    check_trust_claim(trust, &claim_from_receipt(receipt), &receipt.bundle_digest)
}

fn check_trust_claim(
    trust: &InstallTrust<'_>,
    claim: &BundleClaim,
    digest: &[u8; 32],
) -> Result<(), Error> {
    match trust {
        InstallTrust::ExplicitKey => Ok(()),
        InstallTrust::PolicyBound {
            chain,
            claim: expected_claim,
            digest: expected_digest,
        } => {
            if claim != expected_claim || digest != expected_digest {
                return Err(Error::SourceChangedOrShort);
            }
            policy::check_claim_digest(chain, claim, digest).map_err(Error::Policy)
        }
    }
}

/// Independently reconstruct and verify an installed or staged APB1 tree.
/// The caller supplies the exact directory object and a key already selected
/// by the receiver's policy. Registry code must additionally compare the
/// returned app ID/version to the namespace entries it enumerated.
#[derive(Clone, Copy)]
struct InstalledRecord {
    record_len: usize,
    file_count: usize,
    metadata_len: usize,
    payload_bytes: u64,
    source_len: u64,
}

fn read_installed_record<D: Device>(
    volume: &mut Volume<D>,
    scratch: &mut InstallWorkspace,
    directory: u64,
) -> Result<InstalledRecord, Error> {
    let (record_id, typ) = volume.lookup(directory, RECORD_NAME).map_err(Error::Fs)?;
    if typ != FILE {
        return Err(Error::BadRecord);
    }
    let stat = volume.stat(record_id).map_err(Error::Fs)?;
    let record_len = usize::try_from(stat.size).map_err(|_| Error::Bounds)?;
    if !(bundle::HEADER_BYTES + MANIFEST_BYTES + 1 + 64..=RECORD_MAX).contains(&record_len) {
        return Err(Error::BadRecord);
    }
    let mut at = 0usize;
    while at < record_len {
        let take = (record_len - at).min(bundle::IO_CHUNK_BYTES);
        let n = volume
            .read(record_id, at as u64, &mut scratch.chunk[..take])
            .map_err(Error::Fs)?;
        if n != take {
            return Err(Error::BadRecord);
        }
        scratch.record[at..at + take].copy_from_slice(&scratch.chunk[..take]);
        at += take;
    }
    let header = &scratch.record[..bundle::HEADER_BYTES];
    if &header[..4] != b"APB1" {
        return Err(Error::BadRecord);
    }
    let file_count = le32(&header[12..16]) as usize;
    let table_bytes = usize::try_from(le64(&header[16..24])).map_err(|_| Error::Bounds)?;
    let payload_bytes = le64(&header[24..32]);
    if !(1..=MAX_FILES).contains(&file_count)
        || table_bytes
            != file_count
                .checked_mul(bundle::FILE_RECORD_BYTES)
                .ok_or(Error::Bounds)?
    {
        return Err(Error::BadRecord);
    }
    let metadata_len = bundle::HEADER_BYTES
        .checked_add(MANIFEST_BYTES)
        .and_then(|n| n.checked_add(table_bytes))
        .ok_or(Error::Bounds)?;
    if metadata_len
        .checked_add(64)
        .is_none_or(|end| end > record_len)
    {
        return Err(Error::BadRecord);
    }
    let source_len = (record_len as u64)
        .checked_add(payload_bytes)
        .ok_or(Error::Bounds)?;
    Ok(InstalledRecord {
        record_len,
        file_count,
        metadata_len,
        payload_bytes,
        source_len,
    })
}

fn installed_source<'a, D: Device>(
    volume: &'a mut Volume<D>,
    directory: u64,
    record_bytes: &'a [u8],
    info: InstalledRecord,
) -> InstalledSource<'a, D> {
    InstalledSource {
        volume,
        directory,
        record: &record_bytes[..info.record_len],
        file_count: info.file_count,
        metadata_len: info.metadata_len,
        payload_bytes: info.payload_bytes,
        source_len: info.source_len,
    }
}

pub fn verify_installed<D: Device>(
    volume: &mut Volume<D>,
    verifier: &mut Workspace,
    scratch: &mut InstallWorkspace,
    directory: u64,
    trusted_key: &[u8; 32],
) -> Result<InstallReceipt, Error> {
    let info = read_installed_record(volume, scratch, directory)?;
    let mut source = installed_source(volume, directory, &scratch.record, info);
    let verified = verifier
        .verify(&mut source, trusted_key)
        .map_err(Error::Bundle)?;
    Ok(InstallReceipt {
        application_id: *verified.manifest().application_id(),
        package_id: *verified.manifest().package_id(),
        version: verified.manifest().version(),
        signer_id: *verified.signer_id(),
        bundle_digest: *verified.bundle_digest(),
        directory,
    })
}

fn verify_installed_app<D: Device>(
    volume: &mut Volume<D>,
    context: &mut RegistryScanContext<'_>,
    directory: u64,
    expected_application_id: &[u8; 32],
    expected_version: u64,
) -> Result<AppDefinition, Error> {
    let info = read_installed_record(volume, context.install_scratch, directory)?;
    let verified = {
        let mut source = installed_source(volume, directory, &context.install_scratch.record, info);
        let claim = context
            .verifier
            .inspect(&mut source)
            .map_err(Error::Bundle)?;
        let key = policy::select_key(context.chain, &claim).map_err(Error::Policy)?;
        let verified = context
            .verifier
            .verify(&mut source, &key)
            .map_err(Error::Bundle)?;
        if claim_from_bundle(&verified) != claim {
            return Err(Error::SourceChangedOrShort);
        }
        policy::check_eligible(context.chain, &verified).map_err(Error::Policy)?;
        if verified.manifest().application_id() != expected_application_id
            || verified.manifest().version() != expected_version
        {
            return Err(Error::NamespaceMismatch);
        }
        verified
    };
    verify_tree_exact(
        volume,
        directory,
        info.record_len,
        &verified,
        context.tree_scratch,
    )?;
    Ok(AppDefinition::from_verified_bundle(&verified))
}

/// Transactionally rebuild the descriptive installed-app catalog from the
/// exact `/System/Applications` directory object held by the receiver. The
/// live registry is swapped only after a complete bounded scan; staging and
/// invalid/extra tree entries never become launcher candidates.
pub fn rebuild_installed_registry<D: Device>(
    volume: &mut Volume<D>,
    applications_root: u64,
    context: &mut RegistryScanContext<'_>,
    registry: &mut AppRegistry,
    candidate: &mut AppRegistry,
) -> Result<RegistryScanReport, Error> {
    require_directory(volume, applications_root)?;
    candidate.clear();
    let mut report = RegistryScanReport::default();
    let mut budget = volume.statfs().objects as usize;
    let mut after = [0u8; NAME_MAX];
    let mut after_len = 0usize;
    loop {
        let mut entry = [afs2::Entry::EMPTY; 1];
        let count = volume
            .list(applications_root, &after[..after_len], &mut entry)
            .map_err(Error::Fs)?;
        if count == 0 {
            break;
        }
        consume_scan_budget(&mut budget)?;
        let entry = entry[0];
        let name = entry.name();
        advance_cursor(&mut after, &mut after_len, name)?;
        if entry.typ != DIR {
            continue;
        }
        report.application_directories += 1;
        let Some(application_id) = fixed_application_id(name) else {
            continue;
        };
        let mut version_after = [0u8; NAME_MAX];
        let mut version_after_len = 0usize;
        let mut best: Option<AppDefinition> = None;
        loop {
            let mut version_entry = [afs2::Entry::EMPTY; 1];
            let count = volume
                .list(
                    entry.object,
                    &version_after[..version_after_len],
                    &mut version_entry,
                )
                .map_err(Error::Fs)?;
            if count == 0 {
                break;
            }
            consume_scan_budget(&mut budget)?;
            let version_entry = version_entry[0];
            let version_name = version_entry.name();
            advance_cursor(&mut version_after, &mut version_after_len, version_name)?;
            if version_entry.typ != DIR {
                continue;
            }
            report.version_directories += 1;
            let Some(expected_version) = parse_version_name(version_name) else {
                report.rejected_versions += 1;
                continue;
            };
            match verify_installed_app(
                volume,
                context,
                version_entry.object,
                &application_id,
                expected_version,
            ) {
                Ok(definition) => {
                    report.verified_versions += 1;
                    if best
                        .as_ref()
                        .is_none_or(|current| definition.version() > current.version())
                    {
                        best = Some(definition);
                    }
                }
                Err(error) if is_invalid_install_candidate(&error) => {
                    report.rejected_versions += 1;
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(definition) = best {
            candidate.insert(definition).map_err(Error::Registry)?;
            report.published_applications += 1;
        }
    }
    core::mem::swap(registry, candidate);
    candidate.clear();
    Ok(report)
}

fn verify_tree_exact<D: Device>(
    volume: &mut Volume<D>,
    root: u64,
    record_len: usize,
    bundle: &VerifiedBundle<'_>,
    scratch: &mut RegistryScanWorkspace,
) -> Result<(), Error> {
    scratch.frames.fill(TreeFrame::EMPTY);
    scratch.frames[0].directory = root;
    let mut depth = 1usize;
    let mut seen_files = 0usize;
    let mut saw_record = false;
    let mut budget = volume.statfs().objects as usize;
    while depth != 0 {
        let frame = scratch.frames[depth - 1];
        let mut entries = [afs2::Entry::EMPTY; 1];
        let count = volume
            .list(
                frame.directory,
                &frame.cursor[..usize::from(frame.cursor_len)],
                &mut entries,
            )
            .map_err(Error::Fs)?;
        if count == 0 {
            depth -= 1;
            scratch.frames[depth] = TreeFrame::EMPTY;
            continue;
        }
        consume_scan_budget(&mut budget)?;
        let entry = entries[0];
        let name = entry.name();
        let mut cursor_len = usize::from(frame.cursor_len);
        advance_cursor(&mut scratch.frames[depth - 1].cursor, &mut cursor_len, name)?;
        scratch.frames[depth - 1].cursor_len = cursor_len as u8;
        if name.is_empty() || name.len() > 95 || !afs2::valid_name(name) {
            return Err(Error::BadInstalledTree);
        }
        let mut path = frame.path;
        let mut path_len = usize::from(frame.path_len);
        if path_len != 0 {
            if path_len >= 95 {
                return Err(Error::BadInstalledTree);
            }
            path[path_len] = b'/';
            path_len += 1;
        }
        if path_len + name.len() > 95 {
            return Err(Error::BadInstalledTree);
        }
        path[path_len..path_len + name.len()].copy_from_slice(name);
        path_len += name.len();
        let path = &path[..path_len];
        if depth == 1 && path == RECORD_NAME {
            if entry.typ != FILE
                || saw_record
                || volume.stat(entry.object).map_err(Error::Fs)?.size != record_len as u64
            {
                return Err(Error::BadInstalledTree);
            }
            saw_record = true;
            continue;
        }
        match entry.typ {
            FILE => {
                let Some((_, signed)) = bundle.find(path) else {
                    return Err(Error::BadInstalledTree);
                };
                if volume.stat(entry.object).map_err(Error::Fs)?.size != signed.size() {
                    return Err(Error::BadInstalledTree);
                }
                seen_files += 1;
            }
            DIR => {
                if !(0..bundle.file_count()).any(|index| {
                    bundle.file(index).is_some_and(|file| {
                        file.path().len() > path.len()
                            && file.path().starts_with(path)
                            && file.path().get(path.len()) == Some(&b'/')
                    })
                }) {
                    return Err(Error::BadInstalledTree);
                }
                if depth == scratch.frames.len() {
                    return Err(Error::Bounds);
                }
                scratch.frames[depth] = TreeFrame {
                    directory: entry.object,
                    path: {
                        let mut stored = [0; 96];
                        stored[..path.len()].copy_from_slice(path);
                        stored
                    },
                    path_len: path.len() as u8,
                    ..TreeFrame::EMPTY
                };
                depth += 1;
            }
            _ => return Err(Error::BadInstalledTree),
        }
    }
    if !saw_record || seen_files != bundle.file_count() {
        return Err(Error::BadInstalledTree);
    }
    Ok(())
}

fn consume_scan_budget(budget: &mut usize) -> Result<(), Error> {
    *budget = (*budget).checked_sub(1).ok_or(Error::ScanLimit)?;
    Ok(())
}

fn is_invalid_install_candidate(error: &Error) -> bool {
    match error {
        Error::Bundle(bundle_error) => *bundle_error != BundleError::Io,
        Error::Policy(_) => true,
        Error::BadRecord
        | Error::BadInstalledTree
        | Error::NamespaceMismatch
        | Error::Bounds
        | Error::SourceChangedOrShort => true,
        Error::Fs(_)
        | Error::InvalidRoot
        | Error::AlreadyInstalled
        | Error::StagingCollision
        | Error::ReadbackMismatch
        | Error::ScanLimit
        | Error::Registry(_) => false,
    }
}

fn advance_cursor<const N: usize>(
    cursor: &mut [u8; N],
    cursor_len: &mut usize,
    name: &[u8],
) -> Result<(), Error> {
    if name.is_empty() || name.len() > N {
        return Err(Error::BadInstalledTree);
    }
    cursor.fill(0);
    cursor[..name.len()].copy_from_slice(name);
    *cursor_len = name.len();
    Ok(())
}

fn fixed_application_id(name: &[u8]) -> Option<[u8; 32]> {
    if name.is_empty() || name.len() > 31 || name.contains(&0) {
        return None;
    }
    let mut id = [0; 32];
    id[..name.len()].copy_from_slice(name);
    Some(id)
}

fn parse_version_name(name: &[u8]) -> Option<u64> {
    if name.is_empty()
        || name.len() > 20
        || (name.len() > 1 && name[0] == b'0')
        || !name.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    name.iter().try_fold(0u64, |value, byte| {
        value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))
    })
}

fn copy_payload_file<D: Device, S: BundleSource>(
    volume: &mut Volume<D>,
    source: &mut S,
    scratch: &mut InstallWorkspace,
    stage_dir: u64,
    entry: FileEntry,
    bundle_offset: u64,
    wall_us: u64,
) -> Result<(), Error> {
    let path = entry.path();
    let (parent, leaf) = create_parent_dirs(volume, stage_dir, path, wall_us)?;
    let object = volume.create(parent, leaf, wall_us).map_err(Error::Fs)?;
    let mut offset = 0u64;
    while offset < entry.size() {
        let take = (entry.size() - offset).min(bundle::IO_CHUNK_BYTES as u64) as usize;
        let at = bundle_offset.checked_add(offset).ok_or(Error::Bounds)?;
        source
            .read_exact_at(at, &mut scratch.chunk[..take])
            .map_err(|_| Error::SourceChangedOrShort)?;
        let n = volume
            .write(object, offset, &scratch.chunk[..take], wall_us)
            .map_err(Error::Fs)?;
        if n != take {
            return Err(Error::SourceChangedOrShort);
        }
        offset += take as u64;
    }
    Ok(())
}

fn write_all<D: Device>(
    volume: &mut Volume<D>,
    object: u64,
    data: &[u8],
    wall_us: u64,
) -> Result<(), Error> {
    let mut offset = 0usize;
    while offset < data.len() {
        let take = (data.len() - offset).min(bundle::IO_CHUNK_BYTES);
        let n = volume
            .write(object, offset as u64, &data[offset..offset + take], wall_us)
            .map_err(Error::Fs)?;
        if n != take {
            return Err(Error::Bounds);
        }
        offset += take;
    }
    Ok(())
}

fn create_parent_dirs<'a, D: Device>(
    volume: &mut Volume<D>,
    root: u64,
    path: &'a [u8],
    wall_us: u64,
) -> Result<(u64, &'a [u8]), Error> {
    let mut parent = root;
    let mut component_start = 0usize;
    for (i, byte) in path.iter().enumerate() {
        if *byte != b'/' {
            continue;
        }
        let name = &path[component_start..i];
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(Error::BadRecord);
        }
        parent = match volume.lookup(parent, name) {
            Ok((object, DIR)) => object,
            Ok(_) => return Err(Error::BadRecord),
            Err(afs2::Error::NoEnt) => volume.mkdir(parent, name, wall_us).map_err(Error::Fs)?,
            Err(error) => return Err(Error::Fs(error)),
        };
        component_start = i + 1;
    }
    let leaf = &path[component_start..];
    if leaf.is_empty() || leaf.len() > NAME_MAX {
        return Err(Error::BadRecord);
    }
    Ok((parent, leaf))
}

fn resolve_file<D: Device>(
    volume: &mut Volume<D>,
    root: u64,
    path: &[u8],
) -> Result<u64, bundle::SourceError> {
    if path.is_empty() || path.len() > 95 {
        return Err(SourceError);
    }
    let mut parent = root;
    let mut component_start = 0usize;
    for i in 0..=path.len() {
        if i != path.len() && path[i] != b'/' {
            continue;
        }
        let name = path.get(component_start..i).ok_or(SourceError)?;
        if name.is_empty() || name == b"." || name == b".." {
            return Err(SourceError);
        }
        let (object, typ) = volume.lookup(parent, name).map_err(|_| SourceError)?;
        if i == path.len() {
            return (typ == FILE).then_some(object).ok_or(SourceError);
        }
        if typ != DIR {
            return Err(SourceError);
        }
        parent = object;
        component_start = i + 1;
    }
    Err(SourceError)
}

struct InstalledSource<'a, D: Device> {
    volume: &'a mut Volume<D>,
    directory: u64,
    record: &'a [u8],
    file_count: usize,
    metadata_len: usize,
    payload_bytes: u64,
    source_len: u64,
}
impl<D: Device> BundleSource for InstalledSource<'_, D> {
    fn len(&self) -> u64 {
        self.source_len
    }

    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), bundle::SourceError> {
        let end = offset
            .checked_add(u64::try_from(out.len()).map_err(|_| SourceError)?)
            .ok_or(SourceError)?;
        if end > self.source_len {
            return Err(SourceError);
        }
        let mut cursor = usize::try_from(offset).map_err(|_| SourceError)?;
        let mut written = 0usize;
        if cursor < self.record.len() {
            let take = (self.record.len() - cursor).min(out.len());
            out[..take].copy_from_slice(&self.record[cursor..cursor + take]);
            cursor += take;
            written += take;
        }
        if written == out.len() {
            return Ok(());
        }
        let payload_start = self.record.len() as u64;
        if (cursor as u64) < payload_start {
            return Err(SourceError);
        }
        let mut payload_cursor = (cursor as u64)
            .checked_sub(payload_start)
            .ok_or(SourceError)?;
        if payload_cursor >= self.payload_bytes && out.len() != written {
            return Err(SourceError);
        }
        for index in 0..self.file_count {
            let entry = bundle::entry_at(&self.record[..self.metadata_len], index)
                .map_err(|_| SourceError)?;
            if payload_cursor >= entry.size() {
                payload_cursor -= entry.size();
                continue;
            }
            let take = (entry.size() - payload_cursor).min((out.len() - written) as u64) as usize;
            let object = resolve_file(self.volume, self.directory, entry.path())?;
            let n = self
                .volume
                .read(object, payload_cursor, &mut out[written..written + take])
                .map_err(|_| SourceError)?;
            if n != take {
                return Err(SourceError);
            }
            written += take;
            payload_cursor = 0;
            if written == out.len() {
                return Ok(());
            }
        }
        Err(SourceError)
    }
}

fn lookup_optional<D: Device>(
    volume: &mut Volume<D>,
    directory: u64,
    name: &[u8],
) -> Result<Option<(u64, u8)>, Error> {
    match volume.lookup(directory, name) {
        Ok(entry) => Ok(Some(entry)),
        Err(afs2::Error::NoEnt) => Ok(None),
        Err(error) => Err(Error::Fs(error)),
    }
}

fn require_directory<D: Device>(volume: &mut Volume<D>, directory: u64) -> Result<(), Error> {
    if volume.stat(directory).map_err(Error::Fs)?.typ == DIR {
        Ok(())
    } else {
        Err(Error::InvalidRoot)
    }
}

fn cstr_len(field: &[u8; 32]) -> Option<usize> {
    let end = field.iter().position(|byte| *byte == 0)?;
    (end > 0 && field[end..].iter().all(|byte| *byte == 0)).then_some(end)
}

fn decimal(value: u64, out: &mut [u8; 20]) -> &[u8] {
    let mut n = value;
    let mut at = out.len();
    loop {
        at -= 1;
        out[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    &out[at..]
}

fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap_or([0; 4]))
}
fn le64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().unwrap_or([0; 8]))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::bundle::Workspace;
    use std::boxed::Box;
    use std::vec;
    use std::vec::Vec;

    struct Mem {
        image: Vec<u8>,
        writes: usize,
        fail_after: Option<usize>,
    }
    impl Device for Mem {
        fn read(&mut self, block: u64, out: &mut [u8; afs2::BLOCK]) -> afs2::Result<()> {
            let at = block as usize * afs2::BLOCK;
            out.copy_from_slice(
                self.image
                    .get(at..at + afs2::BLOCK)
                    .ok_or(afs2::Error::Io)?,
            );
            Ok(())
        }
        fn write(&mut self, block: u64, data: &[u8; afs2::BLOCK]) -> afs2::Result<()> {
            let at = block as usize * afs2::BLOCK;
            self.image
                .get_mut(at..at + afs2::BLOCK)
                .ok_or(afs2::Error::Io)?
                .copy_from_slice(data);
            self.writes += 1;
            if self.fail_after == Some(self.writes) {
                return Err(afs2::Error::Io);
            }
            Ok(())
        }
        fn write_sector(&mut self, block: u64, data: &[u8; afs2::SECTOR]) -> afs2::Result<()> {
            let at = block as usize * afs2::BLOCK;
            self.image
                .get_mut(at..at + afs2::SECTOR)
                .ok_or(afs2::Error::Io)?
                .copy_from_slice(data);
            self.writes += 1;
            if self.fail_after == Some(self.writes) {
                return Err(afs2::Error::Io);
            }
            Ok(())
        }
    }

    struct SliceSource<'a> {
        bytes: &'a [u8],
        mutate_after: Option<usize>,
        payload_reads: usize,
    }
    impl BundleSource for SliceSource<'_> {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_exact_at(
            &mut self,
            offset: u64,
            out: &mut [u8],
        ) -> Result<(), bundle::SourceError> {
            let start = usize::try_from(offset).map_err(|_| SourceError)?;
            let end = start.checked_add(out.len()).ok_or(SourceError)?;
            out.copy_from_slice(self.bytes.get(start..end).ok_or(SourceError)?);
            let payload_offset = (bundle::HEADER_BYTES
                + crate::manifest::MANIFEST_BYTES
                + le64(&self.bytes[16..24]) as usize
                + 64) as u64;
            if offset >= payload_offset {
                self.payload_reads += 1;
                if self.mutate_after.is_some_and(|at| self.payload_reads > at) {
                    out.last_mut().map(|byte| *byte ^= 1);
                }
            }
            Ok(())
        }
    }

    const ROOT_PUBLIC: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];
    const VALID: &[u8] = include_bytes!("../tests/data/editor.apb1");
    const RESERVED_RECORD: &[u8] = include_bytes!("../tests/data/reserved-record.apb1");
    const SMALL_VOLUME_BLOCKS: u64 = 512;

    fn format_base() -> Vec<u8> {
        let mut volume = Box::new(Volume::<Mem>::empty());
        let device = Mem {
            image: vec![0; SMALL_VOLUME_BLOCKS as usize * afs2::BLOCK],
            writes: 0,
            fail_after: None,
        };
        volume
            .format(device, SMALL_VOLUME_BLOCKS, 0x4152_454e_4132_4653, 0)
            .unwrap();
        let root = volume.root_id().unwrap();
        let system = volume.mkdir(root, b"System", 1).unwrap();
        volume.mkdir(system, b"Applications", 1).unwrap();
        volume.mkdir(system, b".apb1-staging", 1).unwrap();
        volume.device().unwrap().image.clone()
    }

    fn mount(image: Vec<u8>, fail_after: Option<usize>) -> Box<Volume<Mem>> {
        let mut volume = Box::new(Volume::<Mem>::empty());
        volume
            .mount(Mem {
                image,
                writes: 0,
                fail_after,
            })
            .unwrap();
        volume
    }

    fn roots(volume: &mut Volume<Mem>) -> (u64, u64) {
        let root = volume.root_id().unwrap();
        let system = volume.lookup(root, b"System").unwrap().0;
        (
            volume.lookup(system, b"Applications").unwrap().0,
            volume.lookup(system, b".apb1-staging").unwrap().0,
        )
    }

    fn installed_version(volume: &mut Volume<Mem>) -> Option<u64> {
        let (applications, _) = roots(volume);
        let app = volume.lookup(applications, b"com.arena.editor").ok()?.0;
        volume.lookup(app, b"42").ok().map(|entry| entry.0)
    }

    #[test]
    fn staged_multifile_install_readback_and_atomic_activation() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        let chain = Chain::new();
        let receipt = install_with_policy(
            &mut volume,
            &mut verifier,
            &mut scratch,
            &mut source,
            &chain,
            InstallTarget::new(apps, staging, 7),
        )
        .unwrap();
        assert_eq!(receipt.version, 42);
        assert_eq!(&receipt.application_id[..16], b"com.arena.editor");
        let mut staged_entries = [afs2::Entry::EMPTY; 4];
        assert_eq!(volume.list(staging, b"", &mut staged_entries).unwrap(), 0);
        assert_eq!(installed_version(&mut volume), Some(receipt.directory));

        let executable = volume.lookup(receipt.directory, b"bin").unwrap().0;
        let image = volume.lookup(executable, b"editor").unwrap().0;
        let mut bytes = [0; 18];
        assert_eq!(volume.read(image, 0, &mut bytes).unwrap(), bytes.len());
        assert_eq!(&bytes, b"fixture-main-image");
        let mut verify_again = Workspace::new();
        let checked = verify_installed(
            &mut volume,
            &mut verify_again,
            &mut scratch,
            receipt.directory,
            &ROOT_PUBLIC,
        )
        .unwrap();
        assert_eq!(checked.bundle_digest, receipt.bundle_digest);
    }

    #[test]
    fn duplicate_version_refusal_does_not_write_or_change_the_afs2_image() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        install(
            &mut volume,
            &mut verifier,
            &mut scratch,
            &mut source,
            &ROOT_PUBLIC,
            InstallTarget::new(apps, staging, 7),
        )
        .unwrap();
        let before = volume.device().unwrap().image.clone();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        assert_eq!(
            install(
                &mut volume,
                &mut verifier,
                &mut scratch,
                &mut source,
                &ROOT_PUBLIC,
                InstallTarget::new(apps, staging, 8),
            ),
            Err(Error::AlreadyInstalled)
        );
        assert_eq!(volume.device().unwrap().image, before);
    }

    #[test]
    fn reserved_record_path_refuses_before_afs2_mutation() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let before = volume.device().unwrap().image.clone();
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: RESERVED_RECORD,
            mutate_after: None,
            payload_reads: 0,
        };
        let chain = Chain::new();
        assert_eq!(
            install_with_policy(
                &mut volume,
                &mut verifier,
                &mut scratch,
                &mut source,
                &chain,
                InstallTarget::new(apps, staging, 13),
            ),
            Err(Error::Bundle(BundleError::ReservedPath))
        );
        assert_eq!(volume.device().unwrap().image, before);
    }

    #[test]
    fn policy_preflight_binds_the_exact_bundle_before_mutation() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let before = volume.device().unwrap().image.clone();
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        // inspect() reads only bounded metadata; the first full verify sees
        // authentic payload. The second pre-mutation verification is changed.
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: Some(3),
            payload_reads: 0,
        };
        let chain = Chain::new();
        assert_eq!(
            install_with_policy(
                &mut volume,
                &mut verifier,
                &mut scratch,
                &mut source,
                &chain,
                InstallTarget::new(apps, staging, 13),
            ),
            Err(Error::Bundle(BundleError::BadFileHash))
        );
        assert_eq!(volume.device().unwrap().image, before);
    }

    #[test]
    fn registry_rebuild_is_manifest_derived_and_rejects_extra_tree_entries() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let chain = Chain::new();
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        let receipt = install_with_policy(
            &mut volume,
            &mut verifier,
            &mut scratch,
            &mut source,
            &chain,
            InstallTarget::new(apps, staging, 12),
        )
        .unwrap();
        let mut tree_scratch = RegistryScanWorkspace::new();
        let mut context =
            RegistryScanContext::new(&mut verifier, &mut scratch, &mut tree_scratch, &chain);
        let mut registry = AppRegistry::new();
        let mut candidate = AppRegistry::new();
        let report = rebuild_installed_registry(
            &mut volume,
            apps,
            &mut context,
            &mut registry,
            &mut candidate,
        )
        .unwrap();
        assert_eq!(report.application_directories, 1);
        assert_eq!(report.version_directories, 1);
        assert_eq!(report.verified_versions, 1);
        assert_eq!(report.published_applications, 1);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get(&receipt.application_id).unwrap().version(), 42);

        // Staging and unknown files cannot appear in a signed install. Even
        // with a valid signature and listed payload, the extra entry poisons
        // the candidate; a rebuild atomically removes the stale catalog row.
        let rogue = volume
            .create(receipt.directory, b"unsigned-extra", 13)
            .unwrap();
        volume.write(rogue, 0, b"not in APB1", 13).unwrap();
        let report = rebuild_installed_registry(
            &mut volume,
            apps,
            &mut context,
            &mut registry,
            &mut candidate,
        )
        .unwrap();
        assert_eq!(report.verified_versions, 0);
        assert_eq!(report.rejected_versions, 1);
        assert_eq!(report.published_applications, 0);
        assert!(registry.is_empty());
    }

    #[test]
    fn receiver_policy_denial_precedes_all_afs2_mutation() {
        use arena_phase84_crypto_audit::sha256;

        fn id(value: &[u8]) -> [u8; 32] {
            let mut out = [0; 32];
            out[..value.len()].copy_from_slice(value);
            out
        }
        fn chain(minimum: u64, revoked: Option<[u8; 32]>, package: [u8; 32]) -> Chain {
            let mut values = [[0; 32]; 8];
            let count = if let Some(digest) = revoked {
                values[0] = digest;
                1
            } else {
                0
            };
            let mut chain = Chain::new();
            chain
                .add(apkg::Policy {
                    id: package,
                    generation: 1,
                    subordinate: [0x42; 32],
                    minimum,
                    allow: true,
                    revoked_count: count,
                    revoked: values,
                })
                .unwrap();
            chain
        }

        let package_id = id(b"org.arena.editor");
        let cases = [
            (
                chain(43, None, package_id),
                Error::Policy(apkg::Error::Downgrade),
            ),
            (
                chain(42, Some(sha256(VALID)), package_id),
                Error::Policy(apkg::Error::Revoked),
            ),
            (
                chain(42, None, id(b"org.other.editor")),
                Error::Policy(apkg::Error::Collision),
            ),
        ];
        for (policy_chain, expected) in cases {
            let mut volume = mount(format_base(), None);
            let (apps, staging) = roots(&mut volume);
            let before = volume.device().unwrap().image.clone();
            let mut verifier = Workspace::new();
            let mut scratch = InstallWorkspace::new();
            let mut source = SliceSource {
                bytes: VALID,
                mutate_after: None,
                payload_reads: 0,
            };
            assert_eq!(
                install_with_policy(
                    &mut volume,
                    &mut verifier,
                    &mut scratch,
                    &mut source,
                    &policy_chain,
                    InstallTarget::new(apps, staging, 14),
                ),
                Err(expected)
            );
            assert_eq!(volume.device().unwrap().image, before);
        }
    }

    #[test]
    fn a_source_swap_never_activates_unverified_destination_bytes() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        // The first verifier's three payload reads are valid. During staging,
        // a later payload read changes. Destination re-verification must fail.
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: Some(3),
            payload_reads: 0,
        };
        assert!(matches!(
            install(
                &mut volume,
                &mut verifier,
                &mut scratch,
                &mut source,
                &ROOT_PUBLIC,
                InstallTarget::new(apps, staging, 9),
            ),
            Err(Error::Bundle(BundleError::BadFileHash)) | Err(Error::Bundle(BundleError::Io))
        ));
        assert!(installed_version(&mut volume).is_none());
        let mut cleanup = CleanupWorkspace::new();
        assert_eq!(
            purge_abandoned_staging(&mut volume, staging, &mut cleanup, 10).unwrap(),
            1
        );
        let mut entries = [afs2::Entry::EMPTY; 1];
        assert_eq!(volume.list(staging, b"", &mut entries).unwrap(), 0);
    }

    #[test]
    fn wrong_signer_is_refused_before_afs2_mutation() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let before = volume.device().unwrap().image.clone();
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        assert_eq!(
            install(
                &mut volume,
                &mut verifier,
                &mut scratch,
                &mut source,
                &[0; 32],
                InstallTarget::new(apps, staging, 12),
            ),
            Err(Error::Bundle(BundleError::UnknownSigner))
        );
        assert_eq!(volume.device().unwrap().image, before);
    }

    #[test]
    fn corrupted_installed_payload_is_not_a_registry_candidate() {
        let mut volume = mount(format_base(), None);
        let (apps, staging) = roots(&mut volume);
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        let receipt = install(
            &mut volume,
            &mut verifier,
            &mut scratch,
            &mut source,
            &ROOT_PUBLIC,
            InstallTarget::new(apps, staging, 12),
        )
        .unwrap();
        let bin = volume.lookup(receipt.directory, b"bin").unwrap().0;
        let image = volume.lookup(bin, b"editor").unwrap().0;
        volume.write(image, 0, b"X", 13).unwrap();

        let mut verify_again = Workspace::new();
        assert_eq!(
            verify_installed(
                &mut volume,
                &mut verify_again,
                &mut scratch,
                receipt.directory,
                &ROOT_PUBLIC,
            ),
            Err(Error::Bundle(BundleError::BadFileHash))
        );
    }

    #[test]
    fn every_afs2_write_prefix_is_old_or_a_complete_reverifiable_install() {
        let base = format_base();
        let mut probe = mount(base.clone(), None);
        let (apps, staging) = roots(&mut probe);
        let mut verifier = Workspace::new();
        let mut scratch = InstallWorkspace::new();
        let mut source = SliceSource {
            bytes: VALID,
            mutate_after: None,
            payload_reads: 0,
        };
        install(
            &mut probe,
            &mut verifier,
            &mut scratch,
            &mut source,
            &ROOT_PUBLIC,
            InstallTarget::new(apps, staging, 10),
        )
        .unwrap();
        let total_writes = probe.device().unwrap().writes;
        assert!(total_writes > 0);

        for fail_after in 1..=total_writes {
            let mut volume = mount(base.clone(), Some(fail_after));
            let (apps, staging) = roots(&mut volume);
            let mut verifier = Workspace::new();
            let mut scratch = InstallWorkspace::new();
            let mut source = SliceSource {
                bytes: VALID,
                mutate_after: None,
                payload_reads: 0,
            };
            let result = install(
                &mut volume,
                &mut verifier,
                &mut scratch,
                &mut source,
                &ROOT_PUBLIC,
                InstallTarget::new(apps, staging, 11),
            );
            if result.is_ok() {
                continue;
            }
            let crashed_image = volume.device().unwrap().image.clone();
            let mut recovered = mount(crashed_image, None);
            if let Some(directory) = installed_version(&mut recovered) {
                let mut check = Workspace::new();
                verify_installed(
                    &mut recovered,
                    &mut check,
                    &mut scratch,
                    directory,
                    &ROOT_PUBLIC,
                )
                .expect("visible version after a crash must be complete and authentic");
            }
            let (_, staging) = roots(&mut recovered);
            let mut cleanup = CleanupWorkspace::new();
            purge_abandoned_staging(&mut recovered, staging, &mut cleanup, 12)
                .expect("staging janitor must tolerate every interrupted install prefix");
            let mut entries = [afs2::Entry::EMPTY; 1];
            assert_eq!(recovered.list(staging, b"", &mut entries).unwrap(), 0);
        }
    }
}
