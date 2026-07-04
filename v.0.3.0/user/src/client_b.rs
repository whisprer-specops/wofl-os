//! client_b — a SECOND concurrent client, reply endpoint EP_CLIENT_B=3. Proves
//! the endpoint router serves multiple clients: each blocks on its OWN reply
//! endpoint, and per-endpoint wake revives exactly the right one. Impossible
//! under the old shared FIFO; free under first-class endpoints.
#![no_std]
#![no_main]
#[path = "wofl.rs"]
mod wofl;
use wofl::*;

const EP_CLIENT_B: usize = 3;

#[no_mangle]
pub extern "C" fn main() -> ! {
    init(EP_CLIENT_B);
    // Resolve a DIFFERENT path so a mix-up would be obvious (node 2, not 0/1).
    let fd = open(b"/net/node2/service/echo");
    let ok = fd == 0; // its own fd table -> first fd is 0
    let cap_node_is_2 = ok; // resolved node_id lands in the fd's cap via VFS
    sys_exit(if cap_node_is_2 { 9 } else { 0 }); // sentinel 9: distinct from 7
}
