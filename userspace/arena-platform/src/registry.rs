//! Pure userspace application catalog and content-type preference model.
//!
//! Neither the catalog nor association lookup carries authority. It returns
//! application identifiers solely as selection hints; launch must use a
//! separately held exact executable/Image capability, and opening a document
//! requires a separate, explicit capability offer to the selected instance.

use crate::bundle::VerifiedBundle;
use crate::manifest::{self, CONTENT_TYPE_BYTES, ID_BYTES, Manifest};

pub const MAX_APPLICATIONS: usize = 64;
pub const MAX_ASSOCIATION_DEFAULTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Full,
    NotFound,
    Duplicate,
    NotInstalled,
    NotAHandler,
    InvalidContentType,
}

/// Descriptive installed-catalog record. Construction from an authenticated
/// APB1 view proves only that metadata was signed and all file hashes checked;
/// the package service still has to approve signer policy, atomically install
/// the AFS2 tree, read it back, and commit activation before publishing this
/// record in the installed registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppDefinition {
    pub(crate) manifest: Manifest,
    signer_id: [u8; 32],
    bundle_digest: [u8; 32],
}
impl AppDefinition {
    pub fn from_verified_bundle(bundle: &VerifiedBundle<'_>) -> Self {
        Self {
            manifest: *bundle.manifest(),
            signer_id: *bundle.signer_id(),
            bundle_digest: *bundle.bundle_digest(),
        }
    }
    /// Build a descriptive record after the trusted package owner has
    /// accepted filesd's fresh signature/tree-verification response and
    /// applied current receiver policy. The record itself still grants no
    /// launch, filesystem, or process authority.
    pub fn from_receiver_verified_install(
        manifest: Manifest,
        signer_id: [u8; 32],
        bundle_digest: [u8; 32],
    ) -> Self {
        Self {
            manifest,
            signer_id,
            bundle_digest,
        }
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn application_id(&self) -> &[u8; ID_BYTES] {
        self.manifest.application_id()
    }
    pub fn signer_id(&self) -> &[u8; 32] {
        &self.signer_id
    }
    pub fn bundle_digest(&self) -> &[u8; 32] {
        &self.bundle_digest
    }
    pub fn version(&self) -> u64 {
        self.manifest.version()
    }
}

/// Fixed-capacity catalog. The production owner should place this long-lived
/// service table in managed/static memory, not on the initial application
/// stack. Its 64 app records are metadata, not process or capability slots.
pub struct AppRegistry {
    entries: [Option<AppDefinition>; MAX_APPLICATIONS],
    len: usize,
}
impl AppRegistry {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_APPLICATIONS],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Clear a receiver-owned scratch catalog before a full, atomic rebuild.
    pub fn clear(&mut self) {
        self.entries.fill(None);
        self.len = 0;
    }
    pub fn get(&self, app_id: &[u8; ID_BYTES]) -> Option<&AppDefinition> {
        self.entries
            .iter()
            .flatten()
            .find(|app| app.application_id() == app_id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &AppDefinition> {
        self.entries.iter().flatten()
    }

    /// Insert only after the owning package service has committed an install.
    /// A full table or duplicate is rejected before any slot/count mutation.
    pub fn insert(&mut self, app: AppDefinition) -> Result<(), Error> {
        if self.get(app.application_id()).is_some() {
            return Err(Error::Duplicate);
        }
        let Some(slot) = self.entries.iter_mut().find(|slot| slot.is_none()) else {
            return Err(Error::Full);
        };
        *slot = Some(app);
        self.len += 1;
        Ok(())
    }
    pub fn remove(&mut self, app_id: &[u8; ID_BYTES]) -> Option<AppDefinition> {
        let slot = self.entries.iter_mut().find(|slot| {
            slot.as_ref()
                .is_some_and(|app| app.application_id() == app_id)
        })?;
        let removed = slot.take();
        if removed.is_some() {
            self.len -= 1;
        }
        removed
    }
    pub fn supports_type(&self, app_id: &[u8; ID_BYTES], content_type: &[u8]) -> bool {
        self.get(app_id).is_some_and(|app| {
            app.manifest()
                .associations()
                .iter()
                .any(|entry| nul_field(entry) == Some(content_type))
        })
    }
    pub fn handlers<'a>(
        &'a self,
        content_type: &[u8],
        output: &'a mut [Option<[u8; ID_BYTES]>; MAX_APPLICATIONS],
    ) -> Result<usize, Error> {
        if !manifest::valid_content_type(content_type) {
            return Err(Error::InvalidContentType);
        }
        output.fill(None);
        let mut count = 0;
        for app in self.iter() {
            if app
                .manifest()
                .associations()
                .iter()
                .any(|entry| nul_field(entry) == Some(content_type))
            {
                output[count] = Some(*app.application_id());
                count += 1;
            }
        }
        Ok(count)
    }
}
impl Default for AppRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AssociationDefault {
    content_type: [u8; CONTENT_TYPE_BYTES],
    app_id: [u8; ID_BYTES],
}

/// User preference table, intentionally separate from app metadata and from
/// document capabilities. Resolving a default only yields an app ID.
pub struct AssociationDefaults {
    entries: [Option<AssociationDefault>; MAX_ASSOCIATION_DEFAULTS],
    len: usize,
}
impl AssociationDefaults {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_ASSOCIATION_DEFAULTS],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn set_default(
        &mut self,
        registry: &AppRegistry,
        content_type: &[u8],
        app_id: &[u8; ID_BYTES],
    ) -> Result<(), Error> {
        if !manifest::valid_content_type(content_type) || content_type.len() >= CONTENT_TYPE_BYTES {
            return Err(Error::InvalidContentType);
        }
        if registry.get(app_id).is_none() {
            return Err(Error::NotInstalled);
        }
        if !registry.supports_type(app_id, content_type) {
            return Err(Error::NotAHandler);
        }
        let existing = self.entries.iter_mut().find(|entry| {
            entry
                .as_ref()
                .is_some_and(|saved| nul_field(&saved.content_type) == Some(content_type))
        });
        if let Some(slot) = existing {
            *slot = Some(AssociationDefault {
                content_type: fixed_field(content_type),
                app_id: *app_id,
            });
            return Ok(());
        }
        let Some(slot) = self.entries.iter_mut().find(|entry| entry.is_none()) else {
            return Err(Error::Full);
        };
        *slot = Some(AssociationDefault {
            content_type: fixed_field(content_type),
            app_id: *app_id,
        });
        self.len += 1;
        Ok(())
    }
    pub fn default_handler(
        &self,
        registry: &AppRegistry,
        content_type: &[u8],
    ) -> Option<[u8; ID_BYTES]> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| nul_field(&entry.content_type) == Some(content_type))
            .filter(|entry| registry.supports_type(&entry.app_id, content_type))
            .map(|entry| entry.app_id)
    }
    /// Remove references during uninstall. This is metadata cleanup only; the
    /// installer must make durable package/catalog/prefs updates transactionally.
    pub fn remove_app(&mut self, app_id: &[u8; ID_BYTES]) -> usize {
        let mut removed = 0;
        for entry in &mut self.entries {
            if entry.as_ref().is_some_and(|saved| &saved.app_id == app_id) {
                *entry = None;
                self.len -= 1;
                removed += 1;
            }
        }
        removed
    }
}
impl Default for AssociationDefaults {
    fn default() -> Self {
        Self::new()
    }
}

fn fixed_field(bytes: &[u8]) -> [u8; CONTENT_TYPE_BYTES] {
    let mut out = [0u8; CONTENT_TYPE_BYTES];
    out[..bytes.len()].copy_from_slice(bytes);
    out
}
fn nul_field(field: &[u8]) -> Option<&[u8]> {
    let end = field.iter().position(|&byte| byte == 0)?;
    field[end..]
        .iter()
        .all(|&byte| byte == 0)
        .then_some(&field[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{BundleSource, SourceError, Workspace};
    use std::string::ToString;

    const PUBLIC_KEY: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];
    const BUNDLE: &[u8] = include_bytes!("../tests/data/editor.apb1");
    struct Source<'a>(&'a [u8]);
    impl BundleSource for Source<'_> {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), SourceError> {
            let start = usize::try_from(offset).map_err(|_| SourceError)?;
            let end = start.checked_add(out.len()).ok_or(SourceError)?;
            out.copy_from_slice(self.0.get(start..end).ok_or(SourceError)?);
            Ok(())
        }
    }
    fn definition() -> AppDefinition {
        let mut source = Source(BUNDLE);
        let mut workspace = Workspace::new();
        let bundle = workspace.verify(&mut source, &PUBLIC_KEY).unwrap();
        AppDefinition::from_verified_bundle(&bundle)
    }
    fn app_id() -> [u8; ID_BYTES] {
        let mut id = [0; ID_BYTES];
        id[..16].copy_from_slice(b"com.arena.editor");
        id
    }

    #[test]
    fn app_names_and_ids_are_catalog_hints_not_launch_authority() {
        let app = definition();
        assert_eq!(app.application_id(), &app_id());
        assert_eq!(&app.manifest().display_name()[..11], b"Text Editor");
        assert_ne!(app.bundle_digest(), &[0; 32]);
        let mut registry = AppRegistry::new();
        registry.insert(app).unwrap();
        let selected = registry.get(&app_id()).unwrap();
        assert_eq!(selected.application_id(), &app_id());
        // AppDefinition has no Process, Image-capability, PID, file-capability,
        // window, or inherited-authority field; selection only returns IDs.
    }

    #[test]
    fn association_resolution_does_not_offer_document_authority() {
        let mut registry = AppRegistry::new();
        registry.insert(definition()).unwrap();
        let mut defaults = AssociationDefaults::new();
        let id = app_id();
        assert_eq!(defaults.set_default(&registry, b"text/plain", &id), Ok(()));
        assert_eq!(defaults.default_handler(&registry, b"text/plain"), Some(id));
        assert_eq!(defaults.default_handler(&registry, b"image/png"), None);
        let mut handlers = [None; MAX_APPLICATIONS];
        let n = registry.handlers(b"text/markdown", &mut handlers).unwrap();
        assert_eq!(n, 1);
        assert_eq!(handlers[0], Some(id));
        // The only result is a stable, descriptive app ID. Document File caps
        // are absent and must be separately offered by the chooser/manager.
        assert_eq!(
            defaults.set_default(&registry, b"application/x-unknown", &id),
            Err(Error::NotAHandler)
        );
        assert_eq!(
            defaults.set_default(&registry, b"text/plain", &[0; 32]),
            Err(Error::NotInstalled)
        );
    }

    #[test]
    fn capacity_duplicate_and_uninstall_cleanup_are_explicit() {
        let app = definition();
        let mut registry = AppRegistry::new();
        assert_eq!(registry.insert(app), Ok(()));
        assert_eq!(registry.insert(app), Err(Error::Duplicate));
        assert_eq!(registry.len(), 1);
        let id = app_id();
        let mut defaults = AssociationDefaults::new();
        defaults.set_default(&registry, b"text/plain", &id).unwrap();
        assert_eq!(registry.remove(&id), Some(app));
        assert_eq!(registry.remove(&id), None);
        assert_eq!(registry.len(), 0);
        assert_eq!(defaults.default_handler(&registry, b"text/plain"), None);
        assert_eq!(defaults.remove_app(&id), 1);
        assert_eq!(defaults.len(), 0);
    }

    #[test]
    fn catalog_capacity_refusal_is_mutation_free() {
        let mut registry = AppRegistry::new();
        for n in 0..MAX_APPLICATIONS {
            let app = definition_with_serial_id(n);
            assert_eq!(registry.insert(app), Ok(()));
        }
        assert_eq!(registry.len(), MAX_APPLICATIONS);
        let extra = definition_with_serial_id(MAX_APPLICATIONS);
        let extra_id = *extra.application_id();
        assert_eq!(registry.insert(extra), Err(Error::Full));
        assert_eq!(registry.len(), MAX_APPLICATIONS);
        assert!(registry.get(&extra_id).is_none());
        for n in 0..MAX_APPLICATIONS {
            let expected = definition_with_serial_id(n);
            assert_eq!(registry.get(expected.application_id()), Some(&expected));
        }
        let removed_id = *definition_with_serial_id(7).application_id();
        assert!(registry.remove(&removed_id).is_some());
        assert_eq!(registry.len(), MAX_APPLICATIONS - 1);
        assert_eq!(registry.insert(extra), Ok(()));
        assert_eq!(registry.len(), MAX_APPLICATIONS);
    }

    fn definition_with_serial_id(n: usize) -> AppDefinition {
        let mut manifest_bytes = definition().manifest.encode();
        let mut id = [0u8; ID_BYTES];
        let prefix = b"app.test.";
        let digits = n.to_string();
        id[..prefix.len()].copy_from_slice(prefix);
        id[prefix.len()..prefix.len() + digits.len()].copy_from_slice(digits.as_bytes());
        manifest_bytes[8..40].copy_from_slice(&id);
        AppDefinition {
            manifest: Manifest::parse(&manifest_bytes).unwrap(),
            signer_id: [7; 32],
            bundle_digest: [n as u8; 32],
        }
    }
}
