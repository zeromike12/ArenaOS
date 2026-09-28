//! Live, caller-cap-only inventory for ADR-0037's manifest resolver.
//! A symbolic key cannot turn a requested capability into a grant: the
//! kernel returns the kind, object identity and rights of an actual cap
//! in the manager's own slot. Never infer these from the manifest.

use crate::manifest::{self, Held, Key, Kind, MAX_CAPS};

#[path = "../../abi.rs"]
mod abi;

pub const IMAGE_KIND: u64 = 1;
pub const ENDPOINT_KIND: u64 = 2;
pub const NOTIFICATION_KIND: u64 = 3;
pub const PROCESS_KIND: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Description {
    pub kind: u64,
    pub object: u64,
    pub rights: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamedSlot {
    pub key: Key,
    pub slot: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    TooMany,
    DuplicateKey,
    DuplicateSlot,
    NotHeld(i64),
    UnsupportedKind,
    BadRights,
    AmbiguousProcess,
    MissingProcess,
}

/// A compact, bounded, directly resolvable snapshot of live caller caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inventory {
    entries: [Held; MAX_CAPS],
    count: usize,
}
impl Inventory {
    pub fn as_slice(&self) -> &[Held] {
        &self.entries[..self.count]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    Inventory(Error),
    Policy(manifest::Refusal),
}

/// One fail-closed path: kernel-observed caps into the bounded resolver.
/// No child is started, and policy errors never leave a partial plan.
pub fn plan<P: Probe>(
    slots: &[NamedSlot],
    probe: &P,
    services: &[manifest::Service<'_>],
    external: &[manifest::External],
) -> Result<manifest::Plan, PlanError> {
    let inventory = collect(slots, probe).map_err(PlanError::Inventory)?;
    manifest::resolve(services, inventory.as_slice(), external).map_err(PlanError::Policy)
}

pub trait Probe {
    fn describe(&self, slot: u8) -> Result<Description, i64>;
}
pub struct SyscallProbe;
impl Probe for SyscallProbe {
    fn describe(&self, slot: u8) -> Result<Description, i64> {
        let mut out = [0u64; 3];
        // SAFETY: a live 24-byte output buffer, valid for the duration
        // of the synchronous call. The kernel checks caller ownership.
        let status =
            unsafe { abi::syscall2(abi::SYS_CAP_DESCRIBE, slot as u64, out.as_mut_ptr() as u64) };
        if status < 0 {
            Err(status)
        } else if status == 0 {
            Ok(Description {
                kind: out[0],
                object: out[1],
                rights: out[2],
            })
        } else {
            Err(-1) // a nonzero success is not this ABI's contract
        }
    }
}

/// Collect only the fixed symbolic slots assigned by the trusted boot
/// root. A failed query produces no inventory. The result is *data*;
/// neither it nor the manifest can create or forge a kernel capability.
pub fn collect<P: Probe>(slots: &[NamedSlot], probe: &P) -> Result<Inventory, Error> {
    if slots.len() > MAX_CAPS {
        return Err(Error::TooMany);
    }
    let mut out = [Held {
        key: Key(0),
        source_slot: 0,
        kind: Kind::Image,
        object: 0,
        rights: 0,
    }; MAX_CAPS];
    for (i, item) in slots.iter().enumerate() {
        if item.slot as usize >= MAX_CAPS {
            return Err(Error::TooMany);
        }
        if slots[..i].iter().any(|prior| prior.key == item.key) {
            return Err(Error::DuplicateKey);
        }
        if slots[..i].iter().any(|prior| prior.slot == item.slot) {
            return Err(Error::DuplicateSlot);
        }
        let desc = probe.describe(item.slot).map_err(Error::NotHeld)?;
        let kind = match desc.kind {
            IMAGE_KIND => Kind::Image,
            ENDPOINT_KIND => Kind::Endpoint,
            NOTIFICATION_KIND => Kind::Notification,
            _ => return Err(Error::UnsupportedKind), // never reinterpret MMIO or Power
        };
        if desc.rights == 0
            || desc.rights
                & !(manifest::READ | manifest::WRITE | manifest::COPY | manifest::DESTROY) as u64
                != 0
        {
            return Err(Error::BadRights);
        }
        out[i] = Held {
            key: item.key,
            source_slot: item.slot,
            kind,
            object: desc.object,
            rights: desc.rights as u32,
        };
    }
    Ok(Inventory {
        entries: out,
        count: slots.len(),
    })
}

/// Resolve the Process cap created by SYS_SPAWN, which returns a pid but
/// currently puts the cap into the first free parent slot. A pid alone
/// never authorizes a finish operation. Scan only the caller's own cap
/// space; require one exact live Process cap with DESTROY. Empty and
/// unrelated/unsupported slots are ignored. The finish syscall rechecks
/// the chosen slot, so a concurrent cap change fails closed.
pub fn child_handle<P: Probe>(pid: u64, probe: &P) -> Result<u8, Error> {
    let mut found = None;
    for slot in 0..MAX_CAPS {
        let Ok(desc) = probe.describe(slot as u8) else {
            continue;
        };
        if desc.kind == PROCESS_KIND && desc.object == pid {
            if desc.rights & manifest::DESTROY as u64 == 0 {
                return Err(Error::BadRights);
            }
            if found.is_some() {
                return Err(Error::AmbiguousProcess);
            }
            found = Some(slot as u8);
        }
    }
    found.ok_or(Error::MissingProcess)
}

pub fn finish(slot: u8, stop_live_child: bool) -> Result<(), i64> {
    // SAFETY: no user memory is passed. The kernel validates that this
    // slot contains a Process cap with DESTROY and refuses driver pids.
    let status = unsafe {
        abi::syscall2(
            abi::SYS_PROC_FINISH,
            slot as u64,
            u64::from(stop_live_child),
        )
    };
    if status == 0 { Ok(()) } else { Err(status) }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake([Option<Description>; MAX_CAPS]);
    impl Probe for Fake {
        fn describe(&self, slot: u8) -> Result<Description, i64> {
            self.0[slot as usize].ok_or(-2)
        }
    }
    fn fake() -> Fake {
        let mut x = Fake([None; MAX_CAPS]);
        x.0[2] = Some(Description {
            kind: IMAGE_KIND,
            object: 17,
            rights: 1,
        });
        x.0[5] = Some(Description {
            kind: ENDPOINT_KIND,
            object: 7,
            rights: 6,
        });
        x
    }
    #[test]
    fn reads_live_cap_metadata_not_manifest_words() {
        let f = fake();
        let inventory = collect(
            &[
                NamedSlot {
                    key: Key(1),
                    slot: 2,
                },
                NamedSlot {
                    key: Key(2),
                    slot: 5,
                },
            ],
            &f,
        )
        .unwrap();
        assert_eq!(inventory.as_slice().len(), 2);
        assert_eq!(inventory.as_slice()[0].object, 17);
        assert_eq!(inventory.as_slice()[1].rights, 6);
        assert_eq!(inventory.as_slice()[1].source_slot, 5);
    }
    #[test]
    fn observed_authority_goes_directly_to_policy_without_manifest_minting() {
        let mut f = fake();
        let slots = [
            NamedSlot {
                key: Key(1),
                slot: 2,
            },
            NamedSlot {
                key: Key(2),
                slot: 5,
            },
        ];
        let grants = [manifest::Request {
            key: Key(2),
            kind: Kind::Endpoint,
            rights: manifest::WRITE,
            child_slot: 0,
        }];
        let service = [manifest::Service {
            id: 0,
            image: Key(1),
            image_id: 17,
            grants: &grants,
            dependencies: &[],
            restart_limit: 2,
            backoff_us: 1000,
        }];
        let result = plan(&slots, &f, &service, &[]).unwrap();
        assert_eq!(result.steps[0].unwrap().grants[0].unwrap().source_slot, 5);
        f.0[5].as_mut().unwrap().rights = manifest::WRITE as u64; // no COPY
        assert_eq!(
            plan(&slots, &f, &service, &[]),
            Err(PlanError::Policy(manifest::Refusal::NoDelegation))
        );
        f.0[2] = None; // manifest cannot replace a missing Image cap
        assert_eq!(
            plan(&slots, &f, &service, &[]),
            Err(PlanError::Inventory(Error::NotHeld(-2)))
        );
    }

    #[test]
    fn missing_or_unsupported_authority_fails_closed() {
        let mut f = fake();
        assert_eq!(
            collect(
                &[NamedSlot {
                    key: Key(1),
                    slot: 8
                }],
                &f
            ),
            Err(Error::NotHeld(-2))
        );
        f.0[5].as_mut().unwrap().kind = 99; // MMIO is never a service grant
        assert_eq!(
            collect(
                &[NamedSlot {
                    key: Key(1),
                    slot: 5
                }],
                &f
            ),
            Err(Error::UnsupportedKind)
        );
        assert_eq!(
            collect(
                &[
                    NamedSlot {
                        key: Key(1),
                        slot: 2
                    },
                    NamedSlot {
                        key: Key(1),
                        slot: 5
                    }
                ],
                &f
            ),
            Err(Error::DuplicateKey)
        );
    }
    #[test]
    fn child_pid_is_not_authority_but_a_live_process_cap_is() {
        let mut f = fake();
        assert_eq!(child_handle(81, &f), Err(Error::MissingProcess));
        f.0[10] = Some(Description {
            kind: PROCESS_KIND,
            object: 81,
            rights: manifest::DESTROY as u64,
        });
        assert_eq!(child_handle(81, &f), Ok(10));
        f.0[11] = f.0[10];
        assert_eq!(child_handle(81, &f), Err(Error::AmbiguousProcess));
        f.0[11] = None;
        f.0[10].as_mut().unwrap().rights = manifest::READ as u64;
        assert_eq!(child_handle(81, &f), Err(Error::BadRights));
    }
}
