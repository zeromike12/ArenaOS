//! Function references are attenuated copies of the exact fresh session
//! backing. DESTROY is explicitly delegated as the service-function marker;
//! ordinary graphical READ|WRITE|COPY cannot amplify to this grant. The
//! receiver also checks its provisioned resource scope, generation and held
//! original Process liveness. Scope is authority metadata, never app identity.
pub const FILE_READ: u8 = 1;
pub const FILE_WRITE: u8 = 2;
pub const PREFERENCES: u8 = 4;
pub const LAUNCH: u8 = 8;
pub const READ: u64 = 1;
pub const WRITE: u64 = 2;
pub const COPY: u64 = 4;
pub const DESTROY: u64 = 8;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    List,
    Read,
    Put,
    Delete,
    Configure,
    Launch,
}
pub fn permits(scope: u8, rights: u64, operation: Operation) -> bool {
    let (resource, needed) = match operation {
        Operation::List | Operation::Read => (FILE_READ, READ | COPY | DESTROY),
        Operation::Put | Operation::Delete => (FILE_WRITE, WRITE | COPY | DESTROY),
        Operation::Configure => (PREFERENCES, WRITE | COPY | DESTROY),
        Operation::Launch => (LAUNCH, COPY | DESTROY),
    };
    scope & resource != 0 && rights & needed == needed
}
pub fn public_name(name: &[u8; 32]) -> bool {
    let n = name.iter().position(|b| *b == 0).unwrap_or(32);
    n > 5
        && n < 32
        && &name[..5] == b"user-"
        && name[..n]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_' | b'.'))
        && name[n..].iter().all(|b| *b == 0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphics_never_amplifies_and_cross_function_scope_refuses() {
        for scope in 0..16 {
            for op in [
                Operation::List,
                Operation::Read,
                Operation::Put,
                Operation::Delete,
                Operation::Configure,
                Operation::Launch,
            ] {
                assert!(!permits(scope, READ | WRITE | COPY, op));
            }
        }
        assert!(permits(FILE_READ, READ | COPY | DESTROY, Operation::Read));
        assert!(!permits(FILE_READ, READ | COPY | DESTROY, Operation::Put));
        assert!(!permits(FILE_READ | FILE_WRITE, 15, Operation::Configure));
        assert!(!permits(FILE_READ | FILE_WRITE, 15, Operation::Launch));
        assert!(permits(
            PREFERENCES,
            WRITE | COPY | DESTROY,
            Operation::Configure
        ));
        assert!(!permits(PREFERENCES, 15, Operation::Read));
        for rights in 0..16 {
            assert_eq!(
                permits(FILE_WRITE, rights, Operation::Put),
                rights & 14 == 14
            );
        }
    }
    #[test]
    fn explicit_resource_namespace_cannot_write_trust_records() {
        for text in ["user-note", "user-new.txt", "user-a_b"] {
            let mut name = [0; 32];
            name[..text.len()].copy_from_slice(text.as_bytes());
            assert!(public_name(&name));
        }
        for text in [
            "ui10-prefs",
            "n8-test-01",
            "user-",
            "user-a/b",
            "user-a b",
            "user-../p",
        ] {
            let mut name = [0; 32];
            name[..text.len()].copy_from_slice(text.as_bytes());
            assert!(!public_name(&name));
        }
    }
}
