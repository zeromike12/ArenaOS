//! ArenaOS-native networking API (M7.7, ADR-0036).
//!
//! Include this module beside `abi` in a no_std userspace program. `Client`
//! holds only the stack endpoint; UDP/TCP authority is an rngd-backed bearer
//! issued by netstackd. A bearer is intentionally transferable via `handle`
//! and `adopt_*`: possession, not pid or identity, grants authority. The
//! server validates every operation. No implicit Drop IPC: close/release are
//! explicit and return errors. Calls may block for bounded ARP, DNS and
//! ICMP work; TCP OPEN sends SYN then POLL drives progress; UDP RECV waits
//! for the caller's deadline. No POSIX sockets, DHCP or passive TCP.
use crate::abi::*;
use crate::ipc::Transport as _;

pub const UDP_PAYLOAD_MAX: usize = 512 - 14 - 20 - 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Transport(i64),
    Status(u64),
    Protocol,
    TooLong,
    Closed,
}
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    pub ip: [u8; 4],
    pub port: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Datagram {
    pub from: Address,
    pub len: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Connecting,
    Established,
    Closing,
    Closed,
    Failed,
}

fn ip_word(ip: [u8; 4]) -> u64 {
    u32::from_le_bytes(ip) as u64
}
fn word_ip(word: u64) -> Result<[u8; 4]> {
    if word >> 32 != 0 {
        return Err(Error::Protocol);
    }
    Ok((word as u32).to_le_bytes())
}
fn state(word: u64) -> Result<(TcpState, usize)> {
    let value = match word & 0xff {
        TCP_CONNECTING => TcpState::Connecting,
        TCP_ESTABLISHED => TcpState::Established,
        TCP_CLOSING => TcpState::Closing,
        TCP_CLOSED => TcpState::Closed,
        TCP_FAILED => TcpState::Failed,
        _ => return Err(Error::Protocol),
    };
    let available = (word >> 8) as usize;
    if available > 400 {
        return Err(Error::Protocol);
    }
    Ok((value, available))
}

/// IPC transport boundary, injectable for host tests; production uses Syscall.
/// It does not carry a caller identity or add authority to any handle.
pub trait Transport {
    fn call(&self, ep: u64, op: u64, arg: u64, msg: &mut [u8; MSG_BYTES]) -> Result<(u64, u64)>;
}
pub struct Syscall;
impl Transport for Syscall {
    fn call(&self, ep: u64, op: u64, arg: u64, msg: &mut [u8; MSG_BYTES]) -> Result<(u64, u64)> {
        let reply = crate::ipc::Syscall.exchange(ep, arg, op, CAP_NONE, msg)
            .map_err(|e| match e {
                crate::ipc::Error::Transport(r) => Error::Transport(r),
                crate::ipc::Error::ReturnedCap => Error::Protocol,
            })?;
        Ok((reply.status, reply.value))
    }
}

pub struct Client<T: Transport = Syscall> {
    endpoint: u64,
    transport: T,
}
impl Client<Syscall> {
    pub const fn new(endpoint: u64) -> Self {
        Self {
            endpoint,
            transport: Syscall,
        }
    }
}
impl<T: Transport> Client<T> {
    #[allow(dead_code)] // Public API for independent callers; the guest uses Syscall.
    pub const fn with_transport(endpoint: u64, transport: T) -> Self {
        Self {
            endpoint,
            transport,
        }
    }
    fn request(&self, op: u64, arg: u64, msg: &mut [u8; MSG_BYTES]) -> Result<u64> {
        let (status, value) = self.transport.call(self.endpoint, op, arg, msg)?;
        if status == ARP_S_OK {
            Ok(value)
        } else {
            Err(Error::Status(status))
        }
    }
    pub fn resolve(&self, ip: [u8; 4]) -> Result<[u8; 6]> {
        let packed = self.request(ARP_OP_RESOLVE, ip_word(ip), &mut [0; MSG_BYTES])?;
        if packed >> 48 != 0 {
            return Err(Error::Protocol);
        }
        let bytes = packed.to_le_bytes();
        Ok(bytes[..6].try_into().unwrap())
    }
    pub fn ping(&self, ip: [u8; 4]) -> Result<u64> {
        self.request(ICMP_OP_PING, ip_word(ip), &mut [0; MSG_BYTES])
    }
    /// Bounded ASCII dotted A lookup; malformed names never reach the wire.
    pub fn dns_a(&self, name: &[u8]) -> Result<[u8; 4]> {
        if name.is_empty()
            || name.len() > 32
            || !name.is_ascii()
            || name[0] == b'.'
            || name[name.len() - 1] == b'.'
            || name.split(|b| *b == b'.').any(|label| {
                label.is_empty()
                    || !label
                        .iter()
                        .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
            })
        {
            return Err(Error::Status(DNS_S_BAD_NAME));
        }
        let mut msg = [0; MSG_BYTES];
        msg[..name.len()].copy_from_slice(name);
        word_ip(self.request(DNS_OP_LOOKUP, name.len() as u64, &mut msg)?)
    }
    pub fn bind_udp(&self, port: u16) -> Result<UdpSocket<'_, T>> {
        let handle = self.request(UDP_OP_BIND, port as u64, &mut [0; MSG_BYTES])?;
        if handle == 0 {
            return Err(Error::Protocol);
        }
        Ok(self.adopt_udp(handle))
    }
    /// Explicit delegation: the bearer may have been passed by another client.
    /// The stack still rejects forged or revoked bearers on every operation.
    pub fn adopt_udp(&self, handle: u64) -> UdpSocket<'_, T> {
        UdpSocket {
            client: self,
            handle,
        }
    }
    pub fn tcp_open(&self, peer: Address) -> Result<TcpConnection<'_, T>> {
        let mut msg = [0; MSG_BYTES];
        msg[..2].copy_from_slice(&peer.port.to_be_bytes());
        let handle = self.request(TCP_OP_OPEN, ip_word(peer.ip), &mut msg)?;
        if handle == 0 {
            return Err(Error::Protocol);
        }
        Ok(self.adopt_tcp(handle))
    }
    pub fn adopt_tcp(&self, handle: u64) -> TcpConnection<'_, T> {
        TcpConnection {
            client: self,
            handle,
        }
    }
}

pub struct UdpSocket<'a, T: Transport> {
    client: &'a Client<T>,
    handle: u64,
}
impl<T: Transport> UdpSocket<'_, T> {
    pub fn handle(&self) -> u64 {
        self.handle
    }
    pub fn send(&self, peer: Address, payload: &[u8]) -> Result<usize> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        if payload.len() > UDP_INLINE {
            return Err(Error::TooLong);
        }
        let mut msg = [0; MSG_BYTES];
        msg[..4].copy_from_slice(&peer.ip);
        msg[4..6].copy_from_slice(&peer.port.to_be_bytes());
        msg[6..8].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        msg[8..8 + payload.len()].copy_from_slice(payload);
        let sent = self.client.request(UDP_OP_SEND, self.handle, &mut msg)? as usize;
        if sent != payload.len() {
            return Err(Error::Protocol);
        }
        Ok(sent)
    }
    /// Drains a whole datagram, including all authorized IPC continuations.
    /// `out` is exactly the stack's maximum bounded IPv4/UDP payload.
    pub fn recv(&self, timeout_us: u64, out: &mut [u8; UDP_PAYLOAD_MAX]) -> Result<Datagram> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        let mut msg = [0; MSG_BYTES];
        msg[..8].copy_from_slice(&timeout_us.to_le_bytes());
        let total = self.client.request(UDP_OP_RECV, self.handle, &mut msg)? as usize;
        let first = u16::from_be_bytes([msg[6], msg[7]]) as usize;
        if total > out.len() || first != total.min(UDP_INLINE) {
            return Err(Error::Protocol);
        }
        let from = Address {
            ip: msg[..4].try_into().unwrap(),
            port: u16::from_be_bytes([msg[4], msg[5]]),
        };
        out[..first].copy_from_slice(&msg[8..8 + first]);
        let mut off = first;
        while off < total {
            let mut chunk = [0; MSG_BYTES];
            chunk[..2].copy_from_slice(&(off as u16).to_be_bytes());
            let count =
                self.client
                    .request(UDP_OP_RECV_CHUNK, self.handle, &mut chunk)? as usize;
            if count == 0 || count > MSG_BYTES || count > total - off {
                return Err(Error::Protocol);
            }
            out[off..off + count].copy_from_slice(&chunk[..count]);
            off += count;
        }
        Ok(Datagram { from, len: total })
    }
    pub fn close(&mut self) -> Result<()> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        self.client
            .request(UDP_OP_CLOSE, self.handle, &mut [0; MSG_BYTES])?;
        self.handle = 0;
        Ok(())
    }
}

pub struct TcpConnection<'a, T: Transport> {
    client: &'a Client<T>,
    handle: u64,
}
impl<T: Transport> TcpConnection<'_, T> {
    pub fn handle(&self) -> u64 {
        self.handle
    }
    pub fn poll(&self, timeout_us: u64) -> Result<(TcpState, usize)> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        let mut msg = [0; MSG_BYTES];
        msg[..8].copy_from_slice(&timeout_us.to_le_bytes());
        state(self.client.request(TCP_OP_POLL, self.handle, &mut msg)?)
    }
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        if data.is_empty() || data.len() >= MSG_BYTES {
            return Err(Error::TooLong);
        }
        let mut msg = [0; MSG_BYTES];
        msg[0] = data.len() as u8;
        msg[1..1 + data.len()].copy_from_slice(data);
        let sent = self.client.request(TCP_OP_WRITE, self.handle, &mut msg)? as usize;
        if sent != data.len() {
            return Err(Error::Protocol);
        }
        Ok(sent)
    }
    pub fn read(&self, out: &mut [u8; MSG_BYTES]) -> Result<usize> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        // A failed IPC may have partially changed its request buffer.
        let mut msg = [0; MSG_BYTES];
        let n = self.client.request(TCP_OP_READ, self.handle, &mut msg)? as usize;
        if n > MSG_BYTES {
            return Err(Error::Protocol);
        }
        out[..n].copy_from_slice(&msg[..n]);
        Ok(n)
    }
    pub fn close(&self) -> Result<()> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        self.client
            .request(TCP_OP_CLOSE, self.handle, &mut [0; MSG_BYTES])?;
        Ok(())
    }
    pub fn release(&mut self) -> Result<()> {
        if self.handle == 0 {
            return Err(Error::Closed);
        }
        self.client
            .request(TCP_OP_RELEASE, self.handle, &mut [0; MSG_BYTES])?;
        self.handle = 0;
        Ok(())
    }
}
