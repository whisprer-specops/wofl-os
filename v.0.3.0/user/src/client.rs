//! client — reply endpoint EP_CLIENT=2. open() resolves via the VFS with NO
//! yield (endpoint block/wake routes the reply straight back). Proves addressed
//! IPC + the fd layer atop it: local write delivers, remote write hits Layer 6.
#![no_std]
#![no_main]
#[path = "wofl.rs"]
mod wofl;
use wofl::*;

const EP_CLIENT: usize = 2;

#[no_mangle]
pub extern "C" fn main() -> ! {
    init(EP_CLIENT);

    let fd_local  = open(b"/net/node0/service/echo"); // fd 0
    let fd_remote = open(b"/net/node1/service/echo"); // fd 1
    let fds_ok = fd_local == 0 && fd_remote == 1;

    let magic = 0x00C0_FFEEu64.to_le_bytes();
    let w_local  = write(fd_local,  &magic);
    let local_ok = (w_local as isize) >= 0;
    let w_remote = write(fd_remote, &magic);
    let remote_stub_fired = (w_remote as isize) < 0;

    close(fd_local); close(fd_remote);
    sys_exit(if fds_ok && local_ok && remote_stub_fired { 7 } else { 0 })
}
