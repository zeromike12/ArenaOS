//! Synchronous 64-byte IPC v1.1 client with checked three-word replies.
//! Unexpected reply caps are explicitly discarded at the syscall-backed
//! boundary before reporting refusal; a pure fake transport parses only.
use crate::{abi::{CAP_NONE, MSG_BYTES}, sys};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(i64),
    ReturnedCap,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u64,
    pub value: u64,
}
/// Preserve typed service status and refuse unexpected authority. The
/// caller owns `msg` before and after the exchange; no hidden allocation.
pub fn interpret(words: [u64; 3]) -> Result<Reply, Error> {
    if words[2] != CAP_NONE { return Err(Error::ReturnedCap); }
    Ok(Reply { status: words[0], value: words[1] })
}
/// An injectable transport for host wire tests. Production uses `Syscall`.
pub trait Transport {
    fn exchange(&self, endpoint: u64, w0: u64, w1: u64, attached: u64,
                msg: &mut [u8; MSG_BYTES]) -> Result<Reply, Error>;
}
#[derive(Clone, Copy)]
pub struct Syscall;
impl Transport for Syscall {
    fn exchange(&self, endpoint: u64, w0: u64, w1: u64, attached: u64,
                msg: &mut [u8; MSG_BYTES]) -> Result<Reply, Error> {
        let words = sys::ipc_call(endpoint, w0, w1, attached, msg)
            .map_err(Error::Transport)?;
        if words[2] != CAP_NONE {
            // SYS_IPC_CALL installed this reference *before* returning
            // its slot. Merely rejecting the third word leaks authority
            // and eventually fills the caller's fixed cap table.
            sys::discard_landed_cap(words[2]).map_err(Error::Transport)?;
        }
        interpret(words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_transport_and_capability_are_distinct() {
        assert_eq!(interpret([u64::MAX, 42, CAP_NONE]),
                   Ok(Reply { status: u64::MAX, value: 42 }));
        assert_eq!(interpret([0, 0, 0]), Err(Error::ReturnedCap));
        assert_ne!(Error::Transport(-3), Error::ReturnedCap);
    }
}
