//! Bounded AFS1/fsd client. A name selects a file, not authority: the
//! caller must already hold an Endpoint/WRITE and lend its own DMA frame.
use crate::abi::*;
use crate::ipc::{self, Reply, Transport};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Input, Ipc(ipc::Error) }
impl From<ipc::Error> for Error {
    fn from(e: ipc::Error) -> Self { Self::Ipc(e) }
}
pub struct Client<T: Transport = ipc::Syscall> {
    endpoint: u64,
    transport: T,
}
impl Client<ipc::Syscall> {
    pub const fn new(endpoint: u64) -> Self { Self { endpoint, transport: ipc::Syscall } }
}
impl<T: Transport> Client<T> {
    pub const fn with_transport(endpoint: u64, transport: T) -> Self {
        Self { endpoint, transport }
    }
    /// Raw escape hatch for receiver-gated diagnostics; still refuses a
    /// returned cap. Never silently translates a service error to success.
    pub fn request(&self, op: u64, arg: u64, cap: u64,
                   msg: &mut [u8; MSG_BYTES]) -> Result<Reply, Error> {
        self.transport.exchange(self.endpoint, op, arg, cap, msg).map_err(Into::into)
    }
    fn named(&self, op: u64, name: &[u8]) -> Result<Reply, Error> {
        if name.is_empty() || name.len() >= FS_NAME_MAX || name.contains(&0) {
            return Err(Error::Input);
        }
        let mut msg = [0u8; MSG_BYTES];
        msg[..name.len()].copy_from_slice(name);
        self.request(op, 0, CAP_NONE, &mut msg)
    }
    pub fn create(&self, name: &[u8]) -> Result<Reply, Error> { self.named(FS_OP_CREATE, name) }
    pub fn open(&self, name: &[u8]) -> Result<Reply, Error> { self.named(FS_OP_OPEN, name) }
    pub fn unlink(&self, name: &[u8]) -> Result<Reply, Error> { self.named(FS_OP_UNLINK, name) }
    pub fn close(&self, handle: u64) -> Result<Reply, Error> {
        if handle > 255 { return Err(Error::Input); }
        self.request(FS_OP_CLOSE, handle, CAP_NONE, &mut [0; MSG_BYTES])
    }
    pub fn list(&self, cursor: u64, entry: &mut [u8; MSG_BYTES]) -> Result<Reply, Error> {
        if cursor > 32 { return Err(Error::Input); }
        entry.fill(0);
        self.request(FS_OP_LS, cursor, CAP_NONE, entry)
    }
    fn data(&self, op: u64, handle: u64, offset: u64, lent: u64,
            len: u64) -> Result<Reply, Error> {
        if handle > 255 || offset > (u64::MAX >> 8)
            || lent == CAP_NONE || len == 0 || len > FS_XFER_MAX {
            return Err(Error::Input);
        }
        let mut msg = [0u8; MSG_BYTES];
        msg[..8].copy_from_slice(&len.to_le_bytes());
        self.request(op, fs_rw_w1(handle, offset), lent, &mut msg)
    }
    pub fn read(&self, handle: u64, offset: u64, lent: u64, len: u64)
                -> Result<Reply, Error> {
        self.data(FS_OP_READ, handle, offset, lent, len)
    }
    pub fn write(&self, handle: u64, offset: u64, lent: u64, len: u64)
                 -> Result<Reply, Error> {
        self.data(FS_OP_WRITE, handle, offset, lent, len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    #[derive(Clone)]
    struct Expected { op:u64, arg:u64, cap:u64, input: [u8; MSG_BYTES], answer:Reply }
    struct Fake(RefCell<alloc::collections::VecDeque<Expected>>);
    extern crate alloc;
    impl Transport for Fake {
        fn exchange(&self, endpoint:u64, op:u64, arg:u64, cap:u64,
                    msg:&mut [u8;MSG_BYTES]) -> Result<Reply,ipc::Error> {
            assert_eq!(endpoint,7);
            let e=self.0.borrow_mut().pop_front().expect("unexpected fsd IPC");
            assert_eq!((op,arg,cap), (e.op,e.arg,e.cap));
            assert_eq!(&msg[..], &e.input[..]);
            Ok(e.answer)
        }
    }
    #[test]
    fn exact_wire_and_no_hidden_retry() {
        let mut name=[0;MSG_BYTES]; name[..9].copy_from_slice(b"arena.txt");
        let mut length=[0;MSG_BYTES]; length[..8].copy_from_slice(&512u64.to_le_bytes());
        let ok=Reply{status:FS_OK,value:12};
        let missing=Reply{status:FS_ERR_NOT_FOUND,value:0};
        let client=Client::with_transport(7,Fake(RefCell::new(alloc::collections::VecDeque::from([
            Expected{op:FS_OP_OPEN,arg:0,cap:CAP_NONE,input:name,answer:missing},
            Expected{op:FS_OP_CREATE,arg:0,cap:CAP_NONE,input:name,answer:ok},
            Expected{op:FS_OP_WRITE,arg:fs_rw_w1(12,512),cap:9,input:length,answer:Reply{status:FS_OK,value:512}},
            Expected{op:FS_OP_CLOSE,arg:12,cap:CAP_NONE,input:[0;MSG_BYTES],answer:Reply{status:FS_OK,value:0}},
        ]))));
        assert_eq!(client.open(b"arena.txt"),Ok(missing));
        assert_eq!(client.create(b"arena.txt"),Ok(ok));
        assert_eq!(client.write(12,512,9,512).unwrap().value,512);
        assert_eq!(client.close(12).unwrap().status,FS_OK);
        assert_eq!(client.open(b""),Err(Error::Input));
        assert_eq!(client.create(&[b'a'; FS_NAME_MAX]),Err(Error::Input));
        assert_eq!(client.open(b"a\0b"),Err(Error::Input));
        assert_eq!(client.read(12,0,CAP_NONE,512),Err(Error::Input));
        assert_eq!(client.write(12,0,9,FS_XFER_MAX+1),Err(Error::Input));
        assert_eq!(client.close(256),Err(Error::Input));
        assert!(client.transport.0.borrow().is_empty());
    }
}
