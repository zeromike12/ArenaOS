//! Host contract tests for the actual no_std client module and ABI.
#[allow(dead_code)]
#[path = "../userspace/abi.rs"]
mod abi;
#[allow(dead_code)]
#[path = "../userspace/arena-lib/src/sys.rs"]
mod sys;
#[allow(dead_code)]
#[path = "../userspace/arena-lib/src/ipc.rs"]
mod ipc;
#[allow(dead_code)]
#[path = "../userspace/net.rs"]
mod net;
use abi::*;
use net::*;
use std::cell::RefCell;
use std::collections::VecDeque;

struct Exchange {
    op: u64,
    arg: u64,
    request: Vec<u8>,
    status: u64,
    value: u64,
    response: Vec<u8>,
}
struct Fake(RefCell<VecDeque<Exchange>>);
impl Transport for Fake {
    fn call(&self, ep: u64, op: u64, arg: u64, msg: &mut [u8; MSG_BYTES]) -> Result<(u64, u64)> {
        assert_eq!(ep, 7);
        let item = self.0.borrow_mut().pop_front().expect("unexpected IPC call");
        assert_eq!((op, arg), (item.op, item.arg));
        assert_eq!(&msg[..item.request.len()], item.request);
        msg[..item.response.len()].copy_from_slice(&item.response);
        Ok((item.status, item.value))
    }
}
fn ex(op: u64, arg: u64, request: &[u8], status: u64, value: u64, response: &[u8]) -> Exchange {
    Exchange { op, arg, request: request.to_vec(), status, value, response: response.to_vec() }
}
fn client(items: Vec<Exchange>) -> Client<Fake> {
    Client::with_transport(7, Fake(RefCell::new(items.into())))
}
impl Drop for Fake {
    fn drop(&mut self) { assert!(self.0.get_mut().is_empty(), "expected IPC call was skipped"); }
}

#[test]
fn address_wire_layout_dns_and_local_validation() {
    let c = client(vec![
        ex(ARP_OP_RESOLVE, 0x0202000a, &[], ARP_S_OK, 0x02020a005552, &[]),
        ex(ICMP_OP_PING, 0x0202000a, &[], ICMP_S_NO_REPLY, 0, &[]),
        ex(DNS_OP_LOOKUP, 11, b"example.com", ARP_S_OK, 0x9a170168, &[]),
    ]);
    assert_eq!(c.resolve([10, 0, 2, 2]), Ok([0x52,0x55,0,10,2,2]));
    assert_eq!(c.ping([10, 0, 2, 2]), Err(Error::Status(ICMP_S_NO_REPLY)));
    assert_eq!(c.dns_a(b"a..b.c"), Err(Error::Status(DNS_S_BAD_NAME)));
    assert_eq!(c.dns_a(b"example.com"), Ok([104,1,23,154]));
}
#[test]
fn udp_chunk_offsets_sender_identity_and_bearer_lifecycle() {
    let token = 0xdead_beef_1234_5678;
    let mut response = vec![0u8; MSG_BYTES];
    response[..4].copy_from_slice(&[10,0,2,3]);
    response[4..6].copy_from_slice(&53u16.to_be_bytes());
    response[6..8].copy_from_slice(&(UDP_INLINE as u16).to_be_bytes());
    for (i, b) in response[8..].iter_mut().enumerate() { *b = i as u8; }
    let c = client(vec![
        ex(UDP_OP_BIND, 5353, &[], ARP_S_OK, token, &[]),
        ex(UDP_OP_SEND, token, &[10,0,2,3,0,53,0,3,1,2,3], ARP_S_OK, 3, &[]),
        ex(UDP_OP_RECV, token, &123u64.to_le_bytes(), ARP_S_OK, 61, &response),
        ex(UDP_OP_RECV_CHUNK, token, &(56u16).to_be_bytes(), ARP_S_OK, 5, &[56,57,58,59,60]),
        ex(UDP_OP_CLOSE, token, &[], ARP_S_OK, 0, &[]),
        ex(UDP_OP_SEND, token, &[10,0,2,3,0,53,0,1,9], UDP_S_BAD_HANDLE, 0, &[]),
    ]);
    let peer = Address { ip: [10,0,2,3], port: 53 };
    let mut socket = c.bind_udp(5353).unwrap();
    assert_eq!(socket.send(peer, &[1,2,3]), Ok(3));
    assert_eq!(socket.send(peer, &[0; UDP_INLINE + 1]), Err(Error::TooLong));
    let mut data = [0; UDP_PAYLOAD_MAX];
    assert_eq!(socket.recv(123, &mut data), Ok(Datagram { from: peer, len: 61 }));
    assert_eq!(&data[56..61], &[56,57,58,59,60]);
    socket.close().unwrap();
    assert_eq!(socket.send(peer, &[9]), Err(Error::Closed));
    assert_eq!(c.adopt_udp(token).send(peer, &[9]), Err(Error::Status(UDP_S_BAD_HANDLE)));
}
#[test]
fn tcp_async_bearer_state_and_revocation() {
    let token = 0x9999_aaaa_bbbb_cccc;
    let c = client(vec![
        ex(TCP_OP_OPEN, 0x0202000a, &54321u16.to_be_bytes(), ARP_S_OK, token, &[]),
        ex(TCP_OP_POLL, token, &200u64.to_le_bytes(), ARP_S_OK, TCP_CONNECTING, &[]),
        ex(TCP_OP_POLL, token, &0u64.to_le_bytes(), ARP_S_OK, TCP_ESTABLISHED | (5 << 8), &[]),
        ex(TCP_OP_WRITE, token, b"\x03abc", ARP_S_OK, 3, &[]),
        ex(TCP_OP_READ, token, &[], ARP_S_OK, 5, b"hello"),
        ex(TCP_OP_CLOSE, token, &[], TCP_S_STATE, 0, &[]),
        ex(TCP_OP_CLOSE, token, &[], ARP_S_OK, 0, &[]),
        ex(TCP_OP_RELEASE, token, &[], ARP_S_OK, 0, &[]),
        ex(TCP_OP_POLL, token, &0u64.to_le_bytes(), TCP_S_BAD_HANDLE, 0, &[]),
    ]);
    let mut conn = c.tcp_open(Address {ip:[10,0,2,2], port:54321}).unwrap();
    assert_eq!(conn.poll(200), Ok((TcpState::Connecting, 0)));
    assert_eq!(conn.poll(0), Ok((TcpState::Established, 5)));
    assert_eq!(conn.write(&[0; MSG_BYTES]), Err(Error::TooLong));
    assert_eq!(conn.write(b"abc"), Ok(3));
    let mut bytes = [0; MSG_BYTES];
    assert_eq!(conn.read(&mut bytes), Ok(5));
    assert_eq!(&bytes[..5], b"hello");
    assert_eq!(conn.close(), Err(Error::Status(TCP_S_STATE)));
    conn.close().unwrap();
    conn.release().unwrap();
    assert_eq!(conn.poll(0), Err(Error::Closed));
    assert_eq!(c.adopt_tcp(token).poll(0), Err(Error::Status(TCP_S_BAD_HANDLE)));
}
#[test]
fn malformed_replies_are_not_silently_truncated() {
    let c = client(vec![ex(TCP_OP_POLL, 99, &0u64.to_le_bytes(), ARP_S_OK,
        TCP_ESTABLISHED | (401 << 8), &[])]);
    assert_eq!(c.adopt_tcp(99).poll(0), Err(Error::Protocol));
}
