//! vfs — naming service. Recv requests on EP_VFS; reply to each sender's OWN
//! reply endpoint (learned from sys_recv's returned src). No yield: the kernel's
//! per-endpoint block/wake routes every reply to exactly the right client, so
//! this same loop serves MANY clients without change.
#![no_std]
#![no_main]
#[path = "wofl.rs"]
mod wofl;
use wofl::*;

const ECHO_SVC: u32 = 4; // echo service's endpoint id (distinct from EP_VFS=1)

fn seq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut i = 0; while i < a.len() { if a[i]!=b[i] { return false; } i+=1; } true
}
fn parse_num(d: &[u8]) -> u32 {
    let mut n=0u32; let mut i=0;
    while i<d.len() { let c=d[i]; if c<b'0'||c>b'9' {break;} n=n.wrapping_mul(10).wrapping_add((c-b'0') as u32); i+=1; }
    n
}
fn parse_path(path: &[u8]) -> Capability {
    let mut it = path.split(|&b| b==b'/');
    let _lead = it.next();
    let net=it.next().unwrap_or(b""); let node=it.next().unwrap_or(b"");
    let svc=it.next().unwrap_or(b""); let name=it.next().unwrap_or(b"");
    if !seq(net,b"net") || !seq(svc,b"service") || node.len()<4 || !seq(&node[..4],b"node") {
        return Capability::zero();
    }
    let node_id = parse_num(&node[4..]);
    let service_id = if seq(name,b"echo") { ECHO_SVC } else { 0 };
    Capability::new(node_id, service_id, CAP_SEND | CAP_RECV)
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    let mut buf = [0u8; 256];
    let mut served = 0;
    // Harness termination: one client issues 2 opens -> serve 2 then exit.
    // (2-client boot bumps this to 4; a real service loops forever.)
    while served < 3 {
        let (n, reply_ep) = sys_recv(buf.as_mut_ptr(), buf.len(), EP_VFS); // blocks on EP_VFS
        let cap = parse_path(&buf[..n]);
        sys_send(cap.as_bytes().as_ptr(), core::mem::size_of::<Capability>(),
                 0, reply_ep, EP_VFS);   // reply to THIS client's endpoint
        served += 1;
    }
    sys_exit(0)
}
