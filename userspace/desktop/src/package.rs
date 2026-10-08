//! Trusted Desktop broker client for the narrow packaged APB1 operation.
//!
//! The broker supplies one exact filesd capability for the selected source
//! file. The file capability is input data, never install approval; packaged
//! resolves receiver-owned APKG policy and filesd owns AFS2 mutation.

use crate::abi::*;

#[path = "../../arena-platform/src/manifest.rs"]
pub mod manifest;

pub const MAX_CATALOG_APPS: usize = 64;
pub const HELPER_FLAG_TIMER: u32 = 1;
pub const HELPER_FLAG_OWNER_SIGNAL: u32 = 2;
pub const HELPER_FLAG_STREAMS: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppEntry {
    pub application_id: [u8; 32],
    pub display_name: [u8; 32],
    pub flags: u32,
    /// `u8::MAX` for APB1-installed apps; otherwise the trusted built-in kind.
    pub builtin_kind: u8,
}

/// Kernel-issued, write-only package receiver endpoint (ADR-0091).
pub const ENDPOINT_SLOT: u64 = 43;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstallReceipt {
    /// Descriptive signed APB1 version; not launch authority.
    pub version: u64,
    /// Whole-bundle digest returned after durable filesd readback.
    pub bundle_digest: [u8; 32],
}

/// A fresh Image capability resolved by packaged from the currently verified
/// protected APB1 installation. The identity/metadata remain descriptive;
/// only `capability` authorizes the broker's exact-image spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeImage {
    pub capability: u64,
    pub entry: u64,
    pub load_base: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeHelper {
    pub image: NativeImage,
    pub flags: u32,
}

fn endpoint_ready() -> bool {
    let mut package = [0u64; 3];
    (unsafe { syscall2(SYS_CAP_DESCRIBE, ENDPOINT_SLOT, package.as_mut_ptr() as u64) }) == 0
        && package[0] == 2
        && package[2] == RIGHTS_WRITE
}

/// Return the current verified installed catalog size. A metadata count is a
/// descriptive result and never a launch grant.
pub fn app_catalog_count() -> Result<(usize, u64), u64> {
    if !endpoint_ready() {
        return Err(PKG_OFFLINE);
    }
    let mut request = [0u8; 64];
    let mut reply = [0u64, 0, CAP_NONE];
    let transport = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT_SLOT,
            PKG_OP_APP_COUNT,
            0,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            request.as_mut_ptr() as u64,
        )
    };
    if transport != 0 {
        return Err(PKG_OFFLINE);
    }
    if reply[2] != CAP_NONE {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        return Err(PKG_CORRUPT);
    }
    let epoch = u64::from_le_bytes(request[..8].try_into().unwrap());
    if reply[0] != PKG_OK || reply[1] > MAX_CATALOG_APPS as u64 || epoch == 0 {
        return Err(reply[0]);
    }
    Ok((reply[1] as usize, epoch))
}

/// Fetch one generation-bound catalog row. A concurrent install/replacement
/// changes the epoch and returns PKG_STALE so the caller can rebuild the view.
pub fn app_catalog_entry(index: usize, epoch: u64) -> Result<AppEntry, u64> {
    if !endpoint_ready() || epoch == 0 || index >= MAX_CATALOG_APPS {
        return Err(PKG_STALE);
    }
    let mut request = [0u8; 64];
    request[..8].copy_from_slice(&epoch.to_le_bytes());
    let mut reply = [0u64, 0, CAP_NONE];
    let transport = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT_SLOT,
            PKG_OP_APP_ENTRY,
            index as u64,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            request.as_mut_ptr() as u64,
        )
    };
    if transport != 0 {
        return Err(PKG_OFFLINE);
    }
    if reply[2] != CAP_NONE {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        return Err(PKG_CORRUPT);
    }
    if reply[0] != PKG_OK || reply[1] & !(manifest::KNOWN_FLAGS as u64) != 0 {
        return Err(reply[0]);
    }
    let mut application_id = [0u8; 32];
    application_id.copy_from_slice(&request[..32]);
    let mut display_name = [0u8; 32];
    display_name.copy_from_slice(&request[32..]);
    Ok(AppEntry {
        application_id,
        display_name,
        flags: reply[1] as u32,
        builtin_kind: u8::MAX,
    })
}

/// Read one exact 64-byte manifest chunk from packaged's current registry.
fn app_metadata_chunk(index: usize, offset: usize, out: &mut [u8; 64]) -> Result<(), u64> {
    if !endpoint_ready() || offset >= manifest::MANIFEST_BYTES || !offset.is_multiple_of(64) {
        return Err(PKG_CORRUPT);
    }
    let mut request = [0u8; 64];
    request[..8].copy_from_slice(&(offset as u64).to_le_bytes());
    let mut reply = [0u64, 0, CAP_NONE];
    let transport = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT_SLOT,
            PKG_OP_APP_METADATA,
            index as u64,
            CAP_NONE,
            reply.as_mut_ptr() as u64,
            request.as_mut_ptr() as u64,
        )
    };
    if transport != 0 {
        return Err(PKG_OFFLINE);
    }
    if reply[2] != CAP_NONE {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        return Err(PKG_CORRUPT);
    }
    if reply[0] != PKG_OK || reply[1] != 64 {
        return Err(reply[0]);
    }
    out.copy_from_slice(&request);
    Ok(())
}

/// Resolve an indexed descriptive catalog entry into canonical metadata.
pub fn app_manifest(index: usize) -> Result<manifest::Manifest, u64> {
    if index >= app_catalog_count()?.0 {
        return Err(PKG_STALE);
    }
    let mut bytes = [0u8; manifest::MANIFEST_BYTES];
    for offset in (0..bytes.len()).step_by(64) {
        let mut chunk = [0u8; 64];
        app_metadata_chunk(index, offset, &mut chunk)?;
        bytes[offset..offset + 64].copy_from_slice(&chunk);
    }
    manifest::Manifest::parse(&bytes).map_err(|_| PKG_CORRUPT)
}

/// Retrieve the signed content-type declarations used by handler filtering.
/// They are descriptive preferences only; each launch still resolves through
/// packaged's fresh verifier and an exact Image capability.
pub fn app_associations(
    index: usize,
    expected_application_id: &[u8; 32],
    expected_flags: u32,
) -> Result<
    (
        [[u8; manifest::CONTENT_TYPE_BYTES]; manifest::MAX_ASSOCIATIONS],
        usize,
    ),
    u64,
> {
    let app_manifest = app_manifest(index)?;
    if app_manifest.application_id() != expected_application_id
        || app_manifest.flags() != expected_flags
    {
        return Err(PKG_STALE);
    }
    let mut associations = [[0u8; manifest::CONTENT_TYPE_BYTES]; manifest::MAX_ASSOCIATIONS];
    for (destination, source) in associations.iter_mut().zip(app_manifest.associations()) {
        *destination = *source;
    }
    Ok((associations, app_manifest.associations().len()))
}

/// Ask packaged to re-resolve and freshly verify the current installed
/// version, then receive only its exact immutable Image capability.
pub fn launch_installed(application_id: &[u8; 32], pool: u64) -> Result<NativeImage, u64> {
    let mut request = [0u8; 64];
    request[..32].copy_from_slice(application_id);
    launch_installed_request(PKG_OP_APP_LAUNCH, request, pool).map(|(image, _)| image)
}

/// Ask packaged to resolve one signed AHL1 helper from the currently verified
/// installed APB1 application. The returned flags only describe the explicit
/// capability grants Desktop must prepare; the exact Image cap authorizes
/// execution.
pub fn launch_installed_helper(
    application_id: &[u8; 32],
    helper_id: &[u8; 32],
    pool: u64,
) -> Result<NativeHelper, u64> {
    let mut request = [0u8; 64];
    request[..32].copy_from_slice(application_id);
    request[32..].copy_from_slice(helper_id);
    let (image, reply) = launch_installed_request(PKG_OP_APP_HELPER_LAUNCH, request, pool)?;
    let flags = u32::from_le_bytes(reply[32..36].try_into().unwrap());
    if flags & !(HELPER_FLAG_TIMER | HELPER_FLAG_OWNER_SIGNAL | HELPER_FLAG_STREAMS) != 0
        || (flags & HELPER_FLAG_OWNER_SIGNAL != 0 && flags & HELPER_FLAG_TIMER == 0)
        || (flags & HELPER_FLAG_STREAMS != 0
            && flags & (HELPER_FLAG_TIMER | HELPER_FLAG_OWNER_SIGNAL)
                != (HELPER_FLAG_TIMER | HELPER_FLAG_OWNER_SIGNAL))
    {
        unsafe { syscall1(SYS_CAP_DESTROY, image.capability) };
        return Err(PKG_CORRUPT);
    }
    Ok(NativeHelper { image, flags })
}

fn launch_installed_request(
    operation: u64,
    mut request: [u8; 64],
    pool: u64,
) -> Result<(NativeImage, [u8; 64]), u64> {
    if !endpoint_ready() {
        return Err(PKG_OFFLINE);
    }
    let mut staging = [0u64; 3];
    let status = unsafe {
        syscall6(
            SYS_SHARED_CREATE,
            pool,
            (NATIVE_IMAGE_BYTES_MAX / 4096) as u64,
            staging.as_mut_ptr() as u64,
            0,
            0,
            0,
        )
    };
    if status != 0 {
        return Err(status as u64);
    }
    let staging_cap = staging[0];
    let mut reply = [0u64, 0, CAP_NONE];
    let transport = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT_SLOT,
            operation,
            0,
            staging_cap,
            reply.as_mut_ptr() as u64,
            request.as_mut_ptr() as u64,
        )
    };
    let _ = unsafe { syscall1(SYS_CAP_DESTROY, staging_cap) };
    if transport != 0 {
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        return Err(PKG_OFFLINE);
    }
    if reply[0] != PKG_LAUNCH_READY || reply[2] == CAP_NONE {
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        return Err(reply[0]);
    }
    let image = reply[2];
    let mut desc = [0u64; 3];
    let mut info = [0u64; 3];
    let valid = unsafe { syscall2(SYS_CAP_DESCRIBE, image, desc.as_mut_ptr() as u64) } == 0
        && desc[0] == 1
        && desc[2] & (RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY)
            == RIGHTS_READ | RIGHTS_COPY | RIGHTS_DESTROY
        && unsafe { syscall6(SYS_IMAGE_INFO, image, info.as_mut_ptr() as u64, 0, 0, 0, 0) } == 0
        && info[0] >= info[1]
        && (1..=NATIVE_IMAGE_BYTES_MAX as u64).contains(&info[2]);
    if !valid {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, image) };
        return Err(PKG_CORRUPT);
    }
    Ok((
        NativeImage {
            capability: image,
            entry: info[0],
            load_base: info[1],
            bytes: info[2],
        },
        request,
    ))
}

/// Submit an exact filesd File capability to packaged. The broker retains its
/// original cap; IPC transfers a copy and the receiving services re-check the
/// endpoint badge, File record, signer policy, digest and AFS2 transaction.
pub fn install_apb1(source_file: u64) -> Result<InstallReceipt, u64> {
    let mut package = [0u64; 3];
    let mut source = [0u64; 3];
    if source_file == CAP_NONE
        || unsafe { syscall2(SYS_CAP_DESCRIBE, ENDPOINT_SLOT, package.as_mut_ptr() as u64) } != 0
        || package[0] != 2
        || package[2] != RIGHTS_WRITE
        || unsafe { syscall2(SYS_CAP_DESCRIBE, source_file, source.as_mut_ptr() as u64) } != 0
        || source[0] != 12
        || source[2] & (RIGHTS_WRITE | RIGHTS_COPY) != RIGHTS_WRITE | RIGHTS_COPY
    {
        return Err(PKG_DENY);
    }

    let mut request = [0u8; 64];
    let mut reply = [0u64, 0, CAP_NONE];
    let transport = unsafe {
        syscall6(
            SYS_IPC_CALL,
            ENDPOINT_SLOT,
            PKG_OP_APB1_INSTALL,
            0,
            source_file,
            reply.as_mut_ptr() as u64,
            request.as_mut_ptr() as u64,
        )
    };
    if transport != 0 {
        if reply[2] != CAP_NONE {
            let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        }
        return Err(PKG_OFFLINE);
    }
    if reply[2] != CAP_NONE {
        let _ = unsafe { syscall1(SYS_CAP_DESTROY, reply[2]) };
        return Err(PKG_CORRUPT);
    }
    if reply[0] != PKG_INSTALLED {
        return Err(reply[0]);
    }
    if reply[1] == 0 {
        return Err(PKG_CORRUPT);
    }
    let mut bundle_digest = [0u8; 32];
    bundle_digest.copy_from_slice(&request[..32]);
    Ok(InstallReceipt {
        version: reply[1],
        bundle_digest,
    })
}
