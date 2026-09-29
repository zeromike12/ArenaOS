//! Bounded manifest resolution for the Phase 8.0 service manager (ADR-0037).
//!
//! This module produces a *request plan*, never capabilities. Its inventory
//! must be built from kernel-observed, caller-held caps; only SYS_SPAWN can
//! perform the final COPY/right-attenuation check. Do not launch any child
//! until `resolve` succeeds for the whole graph. `order` is topological;
//! it does NOT say a spawned dependency is ready. Readiness is a separate,
//! bounded IPC/notification check before each dependent is started.
//!
//! Phase 8.0 implementation substrate only: no service is spawned here.

pub const MAX_SERVICES: usize = 4;
pub const MAX_GRANTS: usize = 5; // matches spawn::MAX_INHERIT
pub const MAX_DEPS: usize = 4;
pub const MAX_CAPS: usize = 16;
pub const MAX_RESTARTS: u8 = 3;
pub const MAX_BACKOFF_US: u64 = 1_000_000;
pub const READ: u32 = 1;
pub const WRITE: u32 = 2;
pub const COPY: u32 = 4;
pub const DESTROY: u32 = 8;
const KNOWN_RIGHTS: u32 = READ | WRITE | COPY | DESTROY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Endpoint,
    Notification,
}
/// Symbolic bootstrap slot name; a name is NOT authority. The boot root
/// decides what object (if any) actually occupies this entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key(pub u8);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dependency {
    Managed(u8),
    External(u8),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub key: Key,
    pub kind: Kind,
    pub rights: u32,
    /// Spawn's v1 spec installs children in input order (0..count).
    pub child_slot: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct Service<'a> {
    pub id: u8,
    pub image: Key,
    pub image_id: u32,
    pub grants: &'a [Request],
    pub dependencies: &'a [Dependency],
    pub restart_limit: u8,
    pub backoff_us: u64,
}
/// An observed authority in the manager's own cap space. Production must
/// obtain kind/object/rights from a caller-cap-only kernel query rather
/// than believing the manifest or a mutable configuration file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub key: Key,
    pub source_slot: u8,
    pub kind: Kind,
    pub object: u64,
    pub rights: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct External {
    pub id: u8,
    /// Set only after an explicit bounded protocol readiness probe.
    pub ready: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Granted {
    pub key: Key,
    pub kind: Kind,
    pub object: u64,
    pub source_slot: u8,
    pub rights: u32,
    pub child_slot: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub service_id: u8,
    pub image_slot: u8,
    pub grants: [Option<Granted>; MAX_GRANTS],
    pub count: u8,
    pub restart_limit: u8,
    pub backoff_us: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub steps: [Option<Step>; MAX_SERVICES],
    pub count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    TooMany,
    DuplicateService,
    UnknownDependency,
    DuplicateDependency,
    Cycle,
    DependencyOffline,
    InvalidPolicy,
    DuplicateInventory,
    DuplicateSourceSlot,
    MissingAuthority,
    WrongKind,
    WrongImage,
    NoDelegation,
    RightsAmplification,
    DuplicateGrant,
    DuplicateServer,
    WrongChildSlot,
}

fn held(inventory: &[Held], key: Key) -> Result<Held, Refusal> {
    let mut found = None;
    for cap in inventory {
        if cap.key == key {
            if found.is_some() {
                return Err(Refusal::DuplicateInventory);
            }
            found = Some(*cap);
        }
    }
    found.ok_or(Refusal::MissingAuthority)
}

/// Validate the entire dependency graph and requested grants before any
/// spawn. The caller must still compare actual kernel-installed child
/// caps to `Granted` during integration tests; this plan is not a grant.
pub fn resolve(
    services: &[Service<'_>],
    inventory: &[Held],
    external: &[External],
) -> Result<Plan, Refusal> {
    if services.is_empty()
        || services.len() > MAX_SERVICES
        || inventory.len() > MAX_CAPS
        || external.len() > MAX_DEPS
    {
        return Err(Refusal::TooMany);
    }
    // Two symbolic keys must not disguise one physical cap-table slot.
    // The query of the *actual* cap is tied to this slot; aliases would
    // otherwise let two different requests assert conflicting facts.
    for (i, cap) in inventory.iter().enumerate() {
        if inventory[..i]
            .iter()
            .any(|previous| previous.key == cap.key)
        {
            return Err(Refusal::DuplicateInventory);
        }
        if cap.source_slot as usize >= MAX_CAPS
            || inventory[..i]
                .iter()
                .any(|previous| previous.source_slot == cap.source_slot)
        {
            return Err(Refusal::DuplicateSourceSlot);
        }
    }
    for (i, service) in services.iter().enumerate() {
        if services[..i].iter().any(|other| other.id == service.id) {
            return Err(Refusal::DuplicateService);
        }
        if service.grants.len() > MAX_GRANTS || service.dependencies.len() > MAX_DEPS {
            return Err(Refusal::TooMany);
        }
        if service.restart_limit == 0
            || service.restart_limit > MAX_RESTARTS
            || service.backoff_us == 0
            || service.backoff_us > MAX_BACKOFF_US
        {
            return Err(Refusal::InvalidPolicy);
        }
        for (j, dep) in service.dependencies.iter().enumerate() {
            if service.dependencies[..j].contains(dep) {
                return Err(Refusal::DuplicateDependency);
            }
            match dep {
                Dependency::Managed(id) => {
                    if !services.iter().any(|s| s.id == *id) {
                        return Err(Refusal::UnknownDependency);
                    }
                }
                Dependency::External(id) => {
                    let mut found = false;
                    for ext in external {
                        if ext.id == *id {
                            if found {
                                return Err(Refusal::DuplicateDependency);
                            }
                            found = true;
                            if !ext.ready {
                                return Err(Refusal::DependencyOffline);
                            }
                        }
                    }
                    if !found {
                        return Err(Refusal::DependencyOffline);
                    }
                }
            }
        }
    }
    // DFS with explicit, bounded state: 0 unvisited, 1 visiting, 2 done.
    let mut marks = [0u8; MAX_SERVICES];
    let mut order = [0usize; MAX_SERVICES];
    let mut n = 0;
    fn visit(
        at: usize,
        services: &[Service<'_>],
        marks: &mut [u8; MAX_SERVICES],
        order: &mut [usize; MAX_SERVICES],
        n: &mut usize,
    ) -> Result<(), Refusal> {
        if marks[at] == 1 {
            return Err(Refusal::Cycle);
        }
        if marks[at] == 2 {
            return Ok(());
        }
        marks[at] = 1;
        for dep in services[at].dependencies {
            if let Dependency::Managed(id) = dep {
                let j = services
                    .iter()
                    .position(|s| s.id == *id)
                    .ok_or(Refusal::UnknownDependency)?;
                visit(j, services, marks, order, n)?;
            }
        }
        marks[at] = 2;
        order[*n] = at;
        *n += 1;
        Ok(())
    }
    for i in 0..services.len() {
        visit(i, services, &mut marks, &mut order, &mut n)?;
    }
    let mut plan = Plan {
        steps: [None; MAX_SERVICES],
        count: n as u8,
    };
    for (dest, &idx) in order[..n].iter().enumerate() {
        let svc = services[idx];
        let image = held(inventory, svc.image)?;
        if image.kind != Kind::Image {
            return Err(Refusal::WrongKind);
        }
        if image.object != svc.image_id as u64 {
            return Err(Refusal::WrongImage);
        }
        if image.rights & READ == 0 {
            return Err(Refusal::MissingAuthority);
        }
        let mut step = Step {
            service_id: svc.id,
            image_slot: image.source_slot,
            grants: [None; MAX_GRANTS],
            count: svc.grants.len() as u8,
            restart_limit: svc.restart_limit,
            backoff_us: svc.backoff_us,
        };
        for (i, req) in svc.grants.iter().enumerate() {
            if req.child_slot as usize != i {
                return Err(Refusal::WrongChildSlot);
            }
            if req.rights == 0 || req.rights & !KNOWN_RIGHTS != 0 {
                return Err(Refusal::InvalidPolicy);
            }
            if svc.grants[..i].iter().any(|r| r.key == req.key) {
                return Err(Refusal::DuplicateGrant);
            }
            let cap = held(inventory, req.key)?;
            // An Endpoint's READ side is its server authority. Never
            // start two separately managed servers on one endpoint.
            if req.kind == Kind::Endpoint
                && req.rights & READ != 0
                && plan.steps[..dest].iter().flatten().any(|previous| {
                    previous.grants.iter().flatten().any(|prior| {
                        prior.kind == Kind::Endpoint
                            && prior.object == cap.object
                            && prior.rights & READ != 0
                    })
                })
            {
                return Err(Refusal::DuplicateServer);
            }
            if cap.kind != req.kind {
                return Err(Refusal::WrongKind);
            }
            if cap.rights & COPY == 0 {
                return Err(Refusal::NoDelegation);
            }
            if req.rights & !cap.rights != 0 {
                return Err(Refusal::RightsAmplification);
            }
            step.grants[i] = Some(Granted {
                key: req.key,
                kind: cap.kind,
                object: cap.object,
                source_slot: cap.source_slot,
                rights: req.rights,
                child_slot: req.child_slot,
            });
        }
        plan.steps[dest] = Some(step);
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    const IMAGE: Key = Key(1);
    const NETD: Key = Key(2);
    const STACK: Key = Key(3);
    const RNG: Key = Key(4);
    const BACKOFF: Key = Key(5);
    const GRANTS: [Request; 4] = [
        Request {
            key: NETD,
            kind: Kind::Endpoint,
            rights: WRITE,
            child_slot: 0,
        },
        Request {
            key: STACK,
            kind: Kind::Endpoint,
            rights: READ,
            child_slot: 1,
        },
        Request {
            key: BACKOFF,
            kind: Kind::Notification,
            rights: READ | WRITE,
            child_slot: 2,
        },
        Request {
            key: RNG,
            kind: Kind::Endpoint,
            rights: WRITE,
            child_slot: 3,
        },
    ];
    const CAPS: [Held; 5] = [
        Held {
            key: IMAGE,
            source_slot: 0,
            kind: Kind::Image,
            object: 17,
            rights: READ,
        },
        Held {
            key: NETD,
            source_slot: 1,
            kind: Kind::Endpoint,
            object: 7,
            rights: WRITE | COPY,
        },
        Held {
            key: STACK,
            source_slot: 2,
            kind: Kind::Endpoint,
            object: 9,
            rights: READ | WRITE | COPY,
        },
        Held {
            key: BACKOFF,
            source_slot: 3,
            kind: Kind::Notification,
            object: 12,
            rights: READ | WRITE | COPY,
        },
        Held {
            key: RNG,
            source_slot: 4,
            kind: Kind::Endpoint,
            object: 11,
            rights: WRITE | COPY,
        },
    ];
    fn svc<'a>(grants: &'a [Request], deps: &'a [Dependency]) -> Service<'a> {
        Service {
            id: 5,
            image: IMAGE,
            image_id: 17,
            grants,
            dependencies: deps,
            restart_limit: 3,
            backoff_us: 100_000,
        }
    }
    fn resolve_one(
        grants: &[Request],
        caps: &[Held],
        deps: &[Dependency],
        ext: &[External],
    ) -> Result<Plan, Refusal> {
        resolve(&[svc(grants, deps)], caps, ext)
    }
    #[test]
    fn attenuated_grants_and_actual_source_slots() {
        let plan = resolve_one(
            &GRANTS,
            &CAPS,
            &[Dependency::External(6)],
            &[External { id: 6, ready: true }],
        )
        .unwrap();
        assert_eq!(plan.count, 1);
        let step = plan.steps[0].unwrap();
        assert_eq!((step.image_slot, step.count), (0, 4));
        assert_eq!(
            step.grants[1].unwrap(),
            Granted {
                key: STACK,
                kind: Kind::Endpoint,
                object: 9,
                source_slot: 2,
                rights: READ,
                child_slot: 1
            }
        );
        assert_ne!(step.grants[1].unwrap().rights, CAPS[2].rights);
    }
    #[test]
    fn missing_or_false_authority_never_gets_a_plan() {
        assert_eq!(
            resolve_one(&GRANTS, &CAPS[..2], &[], &[]),
            Err(Refusal::MissingAuthority)
        );
        let mut caps = CAPS;
        caps[2].kind = Kind::Notification;
        assert_eq!(
            resolve_one(&GRANTS, &caps, &[], &[]),
            Err(Refusal::WrongKind)
        );
        caps = CAPS;
        caps[1].rights = WRITE;
        assert_eq!(
            resolve_one(&GRANTS, &caps, &[], &[]),
            Err(Refusal::NoDelegation)
        );
        caps = CAPS;
        caps[0].object = 6;
        assert_eq!(
            resolve_one(&GRANTS, &caps, &[], &[]),
            Err(Refusal::WrongImage)
        );
        let mut dup = CAPS.to_vec();
        dup.push(Held {
            source_slot: 5,
            ..CAPS[1]
        });
        assert_eq!(
            resolve_one(&GRANTS, &dup, &[], &[]),
            Err(Refusal::DuplicateInventory)
        );
        let mut aliased = CAPS;
        aliased[4].source_slot = 1;
        assert_eq!(
            resolve_one(&GRANTS, &aliased, &[], &[]),
            Err(Refusal::DuplicateSourceSlot)
        );
    }
    #[test]
    fn rights_may_never_be_broadened_or_quietly_clamped() {
        let mut bad = GRANTS;
        bad[0].rights = READ | WRITE;
        assert_eq!(
            resolve_one(&bad, &CAPS, &[], &[]),
            Err(Refusal::RightsAmplification)
        );
        bad = GRANTS;
        bad[1].child_slot = 3;
        assert_eq!(
            resolve_one(&bad, &CAPS, &[], &[]),
            Err(Refusal::WrongChildSlot)
        );
        bad = GRANTS;
        bad[1].key = NETD;
        assert_eq!(
            resolve_one(&bad, &CAPS, &[], &[]),
            Err(Refusal::DuplicateGrant)
        );
    }
    #[test]
    fn dependencies_must_be_present_ready_and_acyclic() {
        assert_eq!(
            resolve_one(&GRANTS, &CAPS, &[Dependency::External(6)], &[]),
            Err(Refusal::DependencyOffline)
        );
        assert_eq!(
            resolve_one(
                &GRANTS,
                &CAPS,
                &[Dependency::External(6)],
                &[External {
                    id: 6,
                    ready: false
                }]
            ),
            Err(Refusal::DependencyOffline)
        );
        let first = svc(&[], &[Dependency::Managed(8)]);
        let second = Service {
            id: 8,
            dependencies: &[Dependency::Managed(5)],
            ..first
        };
        assert_eq!(resolve(&[first, second], &CAPS, &[]), Err(Refusal::Cycle));
        let unknown = svc(&[], &[Dependency::Managed(99)]);
        assert_eq!(
            resolve(&[unknown], &CAPS, &[]),
            Err(Refusal::UnknownDependency)
        );
    }
    fn first_with_grants() -> Service<'static> {
        svc(&GRANTS, &[])
    }
    #[test]
    fn topological_order_and_duplicate_policy() {
        let first = svc(&[], &[]);
        let second = Service {
            id: 8,
            dependencies: &[Dependency::Managed(5)],
            ..first
        };
        let p = resolve(&[second, first], &CAPS, &[]).unwrap();
        assert_eq!(p.steps[0].unwrap().service_id, 5);
        assert_eq!(p.steps[1].unwrap().service_id, 8);
        assert_eq!(
            resolve(&[first, first], &CAPS, &[]),
            Err(Refusal::DuplicateService)
        );
        let zero = Service {
            restart_limit: 0,
            ..first
        };
        assert_eq!(resolve(&[zero], &CAPS, &[]), Err(Refusal::InvalidPolicy));
        let unbounded = Service {
            restart_limit: MAX_RESTARTS + 1,
            ..first
        };
        assert_eq!(
            resolve(&[unbounded], &CAPS, &[]),
            Err(Refusal::InvalidPolicy)
        );
        let again = Service {
            id: 8,
            grants: &GRANTS,
            ..first
        };
        assert_eq!(
            resolve(&[first_with_grants(), again], &CAPS, &[]),
            Err(Refusal::DuplicateServer)
        );
    }
}
