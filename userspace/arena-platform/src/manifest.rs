//! Canonical descriptive application manifest for APB1 (ADR-0080).
//!
//! This format intentionally has no capability slots, paths to authority, or
//! kernel process IDs. `requested_capabilities` is a permission-request hint;
//! only the launch authority can resolve it into explicitly granted caps.

pub const MANIFEST_BYTES: usize = 512;
pub const MAX_ASSOCIATIONS: usize = 8;
pub const ID_BYTES: usize = 32;
pub const LABEL_BYTES: usize = 32;
pub const PATH_BYTES: usize = 64;
pub const CONTENT_TYPE_BYTES: usize = 32;

pub const FLAG_MULTI_INSTANCE: u32 = 1 << 0;
pub const FLAG_BACKGROUND: u32 = 1 << 1;
pub const FLAG_HEADLESS: u32 = 1 << 2;
/// Request the trusted launcher to supply the native Startup ABI v2 stream
/// set. This signed descriptive bit never grants the stream capability.
pub const FLAG_STANDARD_STREAMS: u32 = 1 << 3;
/// Request an explicitly delegated per-instance native synchronization
/// domain. This signed descriptive bit does not itself grant the capability.
pub const FLAG_NATIVE_SYNC: u32 = 1 << 4;
pub const KNOWN_FLAGS: u32 = FLAG_MULTI_INSTANCE
    | FLAG_BACKGROUND
    | FLAG_HEADLESS
    | FLAG_STANDARD_STREAMS
    | FLAG_NATIVE_SYNC;

/// These bits are requests to userspace policy, never direct authority.
pub const REQUEST_DOCUMENT_READ: u32 = 1 << 0;
pub const REQUEST_DOCUMENT_WRITE: u32 = 1 << 1;
pub const REQUEST_NETWORK_CLIENT: u32 = 1 << 2;
pub const REQUEST_PERSISTENT_BACKGROUND: u32 = 1 << 3;
pub const KNOWN_REQUESTS: u32 = REQUEST_DOCUMENT_READ
    | REQUEST_DOCUMENT_WRITE
    | REQUEST_NETWORK_CLIENT
    | REQUEST_PERSISTENT_BACKGROUND;

const MAGIC: &[u8; 4] = b"AMF1";
const VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadLength,
    BadMagic,
    UnsupportedVersion,
    NonCanonical,
    BadIdentity,
    BadLabel,
    BadPath,
    BadFlags,
    BadDimensions,
    BadAssociation,
}

/// Parsed descriptive metadata. Fixed-width fields retain their canonical
/// zero-padding so a caller can re-encode byte-exactly without allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Manifest {
    application_id: [u8; ID_BYTES],
    package_id: [u8; ID_BYTES],
    display_name: [u8; LABEL_BYTES],
    version: u64,
    flags: u32,
    requested_capabilities: u32,
    entry_path: [u8; PATH_BYTES],
    icon_path: [u8; PATH_BYTES],
    default_width: u16,
    default_height: u16,
    association_count: u32,
    associations: [[u8; CONTENT_TYPE_BYTES]; MAX_ASSOCIATIONS],
}
impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != MANIFEST_BYTES {
            return Err(Error::BadLength);
        }
        if &bytes[..4] != MAGIC {
            return Err(Error::BadMagic);
        }
        if le16(&bytes[4..6]) != VERSION || le16(&bytes[6..8]) as usize != MANIFEST_BYTES {
            return Err(Error::UnsupportedVersion);
        }
        let mut application_id = [0; ID_BYTES];
        application_id.copy_from_slice(&bytes[8..40]);
        let mut package_id = [0; ID_BYTES];
        package_id.copy_from_slice(&bytes[40..72]);
        let mut display_name = [0; LABEL_BYTES];
        display_name.copy_from_slice(&bytes[72..104]);
        let version = le64(&bytes[104..112]);
        let flags = le32(&bytes[112..116]);
        let requested_capabilities = le32(&bytes[116..120]);
        let mut entry_path = [0; PATH_BYTES];
        entry_path.copy_from_slice(&bytes[120..184]);
        let mut icon_path = [0; PATH_BYTES];
        icon_path.copy_from_slice(&bytes[184..248]);
        let default_width = le16(&bytes[248..250]);
        let default_height = le16(&bytes[250..252]);
        let association_count = le32(&bytes[252..256]);
        let mut associations = [[0; CONTENT_TYPE_BYTES]; MAX_ASSOCIATIONS];
        for (i, association) in associations.iter_mut().enumerate() {
            let start = 256 + i * CONTENT_TYPE_BYTES;
            association.copy_from_slice(&bytes[start..start + CONTENT_TYPE_BYTES]);
        }

        if version == 0 || !valid_id(&application_id) || !valid_id(&package_id) {
            return Err(Error::BadIdentity);
        }
        if !valid_display_name(&display_name) {
            return Err(Error::BadLabel);
        }
        if !valid_bundle_path(&entry_path, false) || !valid_bundle_path(&icon_path, true) {
            return Err(Error::BadPath);
        }
        if flags & !KNOWN_FLAGS != 0 || requested_capabilities & !KNOWN_REQUESTS != 0 {
            return Err(Error::BadFlags);
        }
        let headless_dimensions = default_width == 0 && default_height == 0;
        if headless_dimensions {
            if flags & FLAG_HEADLESS == 0 {
                return Err(Error::BadDimensions);
            }
        } else if default_width < 80
            || default_height < 60
            || default_width > 1024
            || default_height > 768
        {
            return Err(Error::BadDimensions);
        }
        if association_count as usize > MAX_ASSOCIATIONS {
            return Err(Error::BadAssociation);
        }
        let count = association_count as usize;
        let mut previous: Option<&[u8]> = None;
        for (index, association) in associations.iter().enumerate() {
            if index < count {
                let value = nul_terminated(association).ok_or(Error::BadAssociation)?;
                if !valid_content_type(value) || previous.is_some_and(|prior| prior >= value) {
                    return Err(Error::BadAssociation);
                }
                previous = Some(value);
            } else if association.iter().any(|&b| b != 0) {
                return Err(Error::NonCanonical);
            }
        }

        Ok(Self {
            application_id,
            package_id,
            display_name,
            version,
            flags,
            requested_capabilities,
            entry_path,
            icon_path,
            default_width,
            default_height,
            association_count,
            associations,
        })
    }

    /// Deterministic APB1 encoding. `parse(encode(x)) == x` for every value
    /// constructible through `parse`; callers cannot use this to manufacture
    /// signer or launch authority.
    pub fn encode(&self) -> [u8; MANIFEST_BYTES] {
        let mut out = [0u8; MANIFEST_BYTES];
        out[..4].copy_from_slice(MAGIC);
        out[4..6].copy_from_slice(&VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&(MANIFEST_BYTES as u16).to_le_bytes());
        out[8..40].copy_from_slice(&self.application_id);
        out[40..72].copy_from_slice(&self.package_id);
        out[72..104].copy_from_slice(&self.display_name);
        out[104..112].copy_from_slice(&self.version.to_le_bytes());
        out[112..116].copy_from_slice(&self.flags.to_le_bytes());
        out[116..120].copy_from_slice(&self.requested_capabilities.to_le_bytes());
        out[120..184].copy_from_slice(&self.entry_path);
        out[184..248].copy_from_slice(&self.icon_path);
        out[248..250].copy_from_slice(&self.default_width.to_le_bytes());
        out[250..252].copy_from_slice(&self.default_height.to_le_bytes());
        out[252..256].copy_from_slice(&self.association_count.to_le_bytes());
        for (i, association) in self.associations.iter().enumerate() {
            let start = 256 + i * CONTENT_TYPE_BYTES;
            out[start..start + CONTENT_TYPE_BYTES].copy_from_slice(association);
        }
        out
    }

    pub fn application_id(&self) -> &[u8; ID_BYTES] {
        &self.application_id
    }
    pub fn package_id(&self) -> &[u8; ID_BYTES] {
        &self.package_id
    }
    pub fn display_name(&self) -> &[u8; LABEL_BYTES] {
        &self.display_name
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn flags(&self) -> u32 {
        self.flags
    }
    /// A permission request only. This value must never be used to create a
    /// capability, authorize an IPC call, or resolve an app ID to an Image.
    pub fn requested_capabilities(&self) -> u32 {
        self.requested_capabilities
    }
    pub fn entry_path(&self) -> &[u8; PATH_BYTES] {
        &self.entry_path
    }
    pub fn icon_path(&self) -> &[u8; PATH_BYTES] {
        &self.icon_path
    }
    pub fn preferred_window(&self) -> (u16, u16) {
        (self.default_width, self.default_height)
    }
    pub fn associations(&self) -> &[[u8; CONTENT_TYPE_BYTES]] {
        &self.associations[..self.association_count as usize]
    }
    pub fn allows_multiple_instances(&self) -> bool {
        self.flags & FLAG_MULTI_INSTANCE != 0
    }
    pub fn allows_background(&self) -> bool {
        self.flags & FLAG_BACKGROUND != 0
    }
    pub fn allows_headless(&self) -> bool {
        self.flags & FLAG_HEADLESS != 0
    }
    pub fn requests_standard_streams(&self) -> bool {
        self.flags & FLAG_STANDARD_STREAMS != 0
    }
}

pub(crate) fn valid_id(raw: &[u8; ID_BYTES]) -> bool {
    let Some(end) = raw.iter().position(|&b| b == 0) else {
        return false;
    };
    if !(1..=31).contains(&end) || raw[end..].iter().any(|&b| b != 0) {
        return false;
    }
    let first = raw[0];
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && raw[..end]
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// Validate a fixed-width, zero-padded, relative bundle path. When `empty` is
/// true the all-zero representation is the canonical "no icon" value.
pub(crate) fn valid_bundle_path<const N: usize>(raw: &[u8; N], empty: bool) -> bool {
    let Some(end) = raw.iter().position(|&b| b == 0) else {
        return false;
    };
    if end == 0 {
        return empty && raw.iter().all(|&b| b == 0);
    }
    if raw[end..].iter().any(|&b| b != 0) {
        return false;
    }
    valid_relative_path(&raw[..end])
}

pub fn valid_relative_path(path: &[u8]) -> bool {
    if path.is_empty() || path.len() > 95 || path[0] == b'/' || path[path.len() - 1] == b'/' {
        return false;
    }
    let mut component_start = 0;
    for (i, &b) in path.iter().enumerate() {
        if b == b'/' {
            let component = &path[component_start..i];
            if component.is_empty() || component == b"." || component == b".." {
                return false;
            }
            component_start = i + 1;
            continue;
        }
        if !(0x21..=0x7e).contains(&b) || b == b'\\' || b == b':' {
            return false;
        }
    }
    let component = &path[component_start..];
    !component.is_empty() && component != b"." && component != b".."
}

fn valid_display_name(raw: &[u8; LABEL_BYTES]) -> bool {
    let Some(end) = raw.iter().position(|&b| b == 0) else {
        return false;
    };
    (1..=31).contains(&end)
        && raw[..end].iter().all(|&b| (0x20..=0x7e).contains(&b))
        && raw[end..].iter().all(|&b| b == 0)
}

pub(crate) fn valid_content_type(value: &[u8]) -> bool {
    let Some(slash) = value.iter().position(|&b| b == b'/') else {
        return false;
    };
    slash > 0
        && slash + 1 < value.len()
        && value.iter().all(|&b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(
                    b,
                    b'/' | b'!' | b'#' | b'$' | b'&' | b'^' | b'_' | b'.' | b'+' | b'-'
                )
        })
        && value[slash + 1..].iter().all(|&b| b != b'/')
}

fn nul_terminated(raw: &[u8]) -> Option<&[u8]> {
    let end = raw.iter().position(|&b| b == 0)?;
    if raw[end..].iter().any(|&b| b != 0) {
        return None;
    }
    Some(&raw[..end])
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

#[cfg(test)]
mod tests {
    use super::*;

    fn path<const N: usize>(s: &[u8]) -> [u8; N] {
        let mut out = [0; N];
        out[..s.len()].copy_from_slice(s);
        out
    }
    fn id(s: &[u8]) -> [u8; 32] {
        path(s)
    }
    fn content_type(s: &[u8]) -> [u8; 32] {
        path(s)
    }
    fn base() -> [u8; MANIFEST_BYTES] {
        let mut b = [0u8; MANIFEST_BYTES];
        b[..4].copy_from_slice(b"AMF1");
        b[4..6].copy_from_slice(&1u16.to_le_bytes());
        b[6..8].copy_from_slice(&(MANIFEST_BYTES as u16).to_le_bytes());
        b[8..40].copy_from_slice(&id(b"com.arena.editor"));
        b[40..72].copy_from_slice(&id(b"org.arena.editor"));
        b[72..104].copy_from_slice(&path::<32>(b"Text Editor"));
        b[104..112].copy_from_slice(&1u64.to_le_bytes());
        b[112..116].copy_from_slice(&(FLAG_MULTI_INSTANCE as u32).to_le_bytes());
        b[116..120].copy_from_slice(&(REQUEST_DOCUMENT_READ as u32).to_le_bytes());
        b[120..184].copy_from_slice(&path::<64>(b"bin/editor"));
        b[184..248].copy_from_slice(&path::<64>(b"icons/editor.bin"));
        b[248..250].copy_from_slice(&640u16.to_le_bytes());
        b[250..252].copy_from_slice(&480u16.to_le_bytes());
        b[252..256].copy_from_slice(&2u32.to_le_bytes());
        b[256..288].copy_from_slice(&content_type(b"text/markdown"));
        b[288..320].copy_from_slice(&content_type(b"text/plain"));
        b
    }

    #[test]
    fn canonical_manifest_roundtrips_without_assigning_authority() {
        let bytes = base();
        let m = Manifest::parse(&bytes).unwrap();
        assert_eq!(m.encode(), bytes);
        assert_eq!(m.application_id(), &id(b"com.arena.editor"));
        assert_eq!(m.package_id(), &id(b"org.arena.editor"));
        assert_eq!(m.display_name(), &path(b"Text Editor"));
        assert_eq!(m.version(), 1);
        assert!(m.allows_multiple_instances());
        assert_eq!(m.preferred_window(), (640, 480));
        assert_eq!(m.associations().len(), 2);
        // A request bit is descriptive; this type has no cap or launch method.
        assert_eq!(m.requested_capabilities(), REQUEST_DOCUMENT_READ);
    }

    #[test]
    fn standard_stream_opt_in_is_descriptive_and_grants_no_capability() {
        let mut bytes = base();
        bytes[112..116]
            .copy_from_slice(&(FLAG_MULTI_INSTANCE | FLAG_STANDARD_STREAMS).to_le_bytes());
        let manifest = Manifest::parse(&bytes).unwrap();
        assert!(manifest.requests_standard_streams());
        assert!(manifest.allows_multiple_instances());
        assert_eq!(manifest.requested_capabilities(), REQUEST_DOCUMENT_READ);
    }

    #[test]
    fn native_sync_opt_in_is_descriptive_and_grants_no_capability() {
        let mut bytes = base();
        bytes[112..116].copy_from_slice(&(FLAG_MULTI_INSTANCE | FLAG_NATIVE_SYNC).to_le_bytes());
        let manifest = Manifest::parse(&bytes).unwrap();
        assert!(manifest.allows_multiple_instances());
        assert_eq!(manifest.flags() & FLAG_NATIVE_SYNC, FLAG_NATIVE_SYNC);
        assert_eq!(manifest.requested_capabilities(), REQUEST_DOCUMENT_READ);
    }

    #[test]
    fn reserved_noncanonical_identity_associations_and_dimensions_refuse() {
        let original = base();
        let cases: &[(usize, u8)] = &[
            (0, b'X'),   // magic
            (4, 2),      // version
            (8, b'/'),   // app identity
            (40, b'.'),  // package identity start
            (72, 0xff),  // label not printable ASCII in v1
            (104, 0),    // zero version
            (112, 0x80), // unknown flag
            (116, 0x80), // unknown permission request bit
            (120, b'/'), // absolute main path
            (184, b'/'), // absolute icon path
            (249, 0xff), // width above the v1 bound
            (252, 9),    // too many content types
            (256, b'X'), // unsorted/non-MIME first type
            (320, b'X'), // nonzero unused association slot
        ];
        for &(offset, value) in cases {
            let mut bad = original;
            bad[offset] = value;
            assert!(Manifest::parse(&bad).is_err(), "offset={offset}");
        }
        let mut bad = original;
        bad[256..288].copy_from_slice(&content_type(b"text/plain"));
        bad[288..320].copy_from_slice(&content_type(b"text/markdown"));
        assert_eq!(Manifest::parse(&bad), Err(Error::BadAssociation));

        let mut headless = original;
        headless[112..116].copy_from_slice(&(FLAG_HEADLESS as u32).to_le_bytes());
        headless[248..252].fill(0);
        assert_eq!(
            Manifest::parse(&headless).unwrap().preferred_window(),
            (0, 0)
        );
    }

    #[test]
    fn bundle_paths_reject_traversal_aliases_and_platform_separators() {
        for bad in [
            b"/bin/app".as_slice(),
            b"../escape",
            b"bin/../escape",
            b"bin//app",
            b"bin/./app",
            b"bin/app/",
            b"bin\\app",
            b"C:/app",
            b"bin app",
            b"",
        ] {
            assert!(!valid_relative_path(bad), "accepted {:?}", bad);
        }
        assert!(valid_relative_path(b"bin/editor"));
        assert!(valid_relative_path(b"assets/icon.v1.bin"));
        let empty = [0u8; 64];
        assert!(valid_bundle_path(&empty, true));
        assert!(!valid_bundle_path(&empty, false));
    }
}
