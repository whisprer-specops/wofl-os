//! Layer 7a: identity & keyed discovery.
//!
//! Supersedes L6f's single shared HMAC key. Each node holds a static X25519
//! keypair; pairs derive a symmetric session key by Diffie-Hellman. Every
//! remote IPCMessage is now MAC'd under the PER-PEER session key, not one
//! global secret - a forged frame from an un-provisioned node has no valid
//! key to tag with and is dropped at the outermost gate (unchanged ordering
//! from L6f: verify BEFORE the node-id filter).
//!
//! Primitive: x25519-dalek (portable serial backend, proven no_std on
//! riscv64gc) for DH; blake3::derive_key for domain-separated key stretching;
//! blake3::keyed_hash for the MAC (same construction L6f used, new key).
//!
//! ============================================================
//! !!!!! STEPPING-STONE SECRETS - STILL NOT SHIP-GRADE !!!!!
//! ============================================================
//! The per-node SECRET SEEDS below are baked into every binary, exactly like
//! L6f's HMAC_KEY was. So the CRYPTO is now real per-pair DH, but the
//! PROVISIONING is not: anyone with the repo can derive every node's secret.
//! This is a deliberate, bounded step - it buys real pairwise keys and an
//! allowlist NOW, and defers true provisioning (env/file-injected PRIVATE
//! secrets + a PUBLIC-key-only allowlist, keys LEARNED via discovery rather
//! than baked) to later L7 work, where key exchange and node discovery are
//! the same problem. See PROVISIONING UPGRADE below. Grep NODE_SECRET_SEEDS.
//! ============================================================
//!
//! PROVISIONING UPGRADE (the real fix, later):
//!   1. Each node generates a private secret OUTSIDE the repo (env/file).
//!   2. Its PUBLIC key (safe to publish) goes into a baked allowlist, OR is
//!      learned dynamically during discovery + pinned.
//!   3. NODE_SECRET_SEEDS disappears; my_secret() reads the private secret
//!      from its out-of-repo source; peer_expected_pubkey() reads a table of
//!      PUBLIC keys. No secret ever lives in the binary.
//! The env override below (NODE_SECRET) is a first taste of (1): it lets a
//! node present a DIFFERENT identity than its baked seed - used to prove the
//! allowlist actually rejects an unrecognised key (the L7a negative test).

use x25519_dalek::{StaticSecret, PublicKey};
use core::sync::atomic::{AtomicBool, Ordering};

pub const TAG_LEN: usize = 32;
pub const KEY_LEN: usize = 32;

/// Max node slots. Peer table + seeds are sized to this; NODE_ID indexes it.
pub const MAX_NODES: usize = 4;

/// How many nodes actually form the cluster THIS build expects. Node 0's boot
/// handshake waits for (CLUSTER_SIZE - 1) peers before running its demo. Bump
/// to 3 at L7d (Raft wants an odd voter set). Becomes discovery's job later.
pub const CLUSTER_SIZE: usize = 2;

/// HELLO frame subtypes (payload byte 0). REQUEST solicits + announces; REPLY
/// answers a REQUEST and is TERMINAL (never triggers another frame) so the
/// exchange can't ping-pong. Wire: [subtype 1B][pubkey 32B] @ ETHERTYPE_HELLO.
pub const HELLO_REQUEST: u8 = 0;
pub const HELLO_REPLY: u8 = 1;

/// STEPPING-STONE per-node secret seeds (see loud banner above). derive_key
/// stretches each into a valid 32-byte X25519 scalar (the curve clamps
/// internally), so we never parse hex in const/no_std.
const NODE_SECRET_SEEDS: [&[u8]; MAX_NODES] = [
    b"woflOS-L7a-node0-secret-seed-do-not-ship",
    b"woflOS-L7a-node1-secret-seed-do-not-ship",
    b"woflOS-L7a-node2-secret-seed-do-not-ship",
    b"woflOS-L7a-node3-secret-seed-do-not-ship",
];

/// Domain-separation contexts for blake3::derive_key. Distinct strings ensure
/// the identity scalar and the session key can't collide even on equal input.
const CTX_IDENTITY: &str = "woflOS-L7a-identity-v1";
const CTX_SESSION: &str = "woflOS-L7a-session-v1";

/// Derived pairwise session keys, indexed by PEER node_id. `None` until the
/// HELLO handshake with that peer completes.
///
/// SAFETY (single-hart static mut, ledger entry shared with NIC/ENDPOINTS):
/// written only from on_hello (IRQ context, SIE-gated) and read from
/// tag_for/verify_from. The one S-mode path that runs with SIE open - the
/// boot/listen loop - touches this ONLY through on_hello, which completes each
/// write before returning. No interleave without a second hart; add a lock
/// FIRST if SMP ever lands. Accessed via addr_of! to avoid static_mut_refs.
static mut SESSION_KEYS: [Option<[u8; KEY_LEN]>; MAX_NODES] = [None; MAX_NODES];

/// Lock-free readiness flags mirroring SESSION_KEYS - lets the boot handshake
/// window poll "is peer N established?" without touching the static mut.
static SESSION_READY: [AtomicBool; MAX_NODES] = [
    AtomicBool::new(false), AtomicBool::new(false),
    AtomicBool::new(false), AtomicBool::new(false),
];

/// This node's static secret. Env override (NODE_SECRET) replaces the baked
/// seed for THIS node only - used by the negative test to present an identity
/// no peer's allowlist recognises. Recomputed on demand (cheap, called a
/// handful of times at boot); no cached static, no extra lint surface.
fn my_secret() -> StaticSecret {
    let seed: &[u8] = match option_env!("NODE_SECRET") {
        Some(s) => s.as_bytes(),
        None => NODE_SECRET_SEEDS[crate::NODE_ID as usize],
    };
    StaticSecret::from(blake3::derive_key(CTX_IDENTITY, seed))
}

/// The pubkey every node AGREES a given node_id should present - derived from
/// that node's BAKED seed (NEVER the env override). This IS the allowlist: an
/// arriving HELLO whose pubkey != this is rejected. Under upgraded
/// provisioning this becomes a lookup into a baked table of PUBLIC keys.
fn peer_expected_pubkey(node_id: usize) -> PublicKey {
    let scalar = blake3::derive_key(CTX_IDENTITY, NODE_SECRET_SEEDS[node_id]);
    PublicKey::from(&StaticSecret::from(scalar))
}

/// Our public key bytes, to place in outgoing HELLO frames.
pub fn my_pubkey_bytes() -> [u8; 32] {
    *PublicKey::from(&my_secret()).as_bytes()
}

/// Log our node id + public-key fingerprint at boot.
pub fn print_identity() {
    let pk = my_pubkey_bytes();
    crate::kprintln!("[L7a] node {} identity: pubkey fp {:02x}{:02x}{:02x}{:02x}..",
        crate::NODE_ID, pk[0], pk[1], pk[2], pk[3]);
}

/// Peers we expect to establish with (cluster minus ourself).
pub fn expected_peer_count() -> usize {
    CLUSTER_SIZE.saturating_sub(1)
}

/// Peers whose session key is currently established.
pub fn established_peer_count() -> usize {
    let me = crate::NODE_ID as usize;
    let mut n = 0;
    let mut i = 0;
    while i < MAX_NODES {
        if i != me && SESSION_READY[i].load(Ordering::SeqCst) { n += 1; }
        i += 1;
    }
    n
}

/// Handle an inbound HELLO. `subtype` is the frame's byte 0; `src_node` comes
/// from the source MAC's last byte; `pubkey` is the presented 32-byte key.
///
/// Allowlist gate: presented pubkey must equal peer_expected_pubkey(src_node),
/// binding claimed identity (node_id) to its key. On pass, derive the pairwise
/// session key by DH - idempotent, static keys derive the same value every
/// time. Returns TRUE iff the caller should transmit a HELLO_REPLY (only for
/// allowlisted REQUESTs; REPLYs are terminal). Runs in IRQ context: one
/// scalar-mult + one derive_key, no alloc, no blocking - heavier than a
/// keyed_hash but bounded and once-per-peer.
pub fn on_hello(subtype: u8, src_node: usize, pubkey: &[u8; 32]) -> bool {
    let me = crate::NODE_ID as usize;
    if src_node >= MAX_NODES || src_node == me {
        return false; // bogus id, or our own mcast echo
    }
    if peer_expected_pubkey(src_node).as_bytes() != pubkey {
        crate::kprintln!("[L7a] HELLO from node {} REJECTED - pubkey not in allowlist", src_node);
        return false;
    }
    if !SESSION_READY[src_node].load(Ordering::SeqCst) {
        let their_pub = PublicKey::from(*pubkey);
        let shared = my_secret().diffie_hellman(&their_pub);
        let key = blake3::derive_key(CTX_SESSION, shared.as_bytes());
        unsafe { (*core::ptr::addr_of_mut!(SESSION_KEYS))[src_node] = Some(key); }
        SESSION_READY[src_node].store(true, Ordering::SeqCst);
        crate::kprintln!("[L7a] session ESTABLISHED with node {} - key fp {:02x}{:02x}{:02x}{:02x}..",
            src_node, key[0], key[1], key[2], key[3]);
    }
    subtype == HELLO_REQUEST
}

/// TX-side MAC: tag `msg_bytes` under the session key for `dst_node`. `None`
/// if no session with that peer yet - caller must not put the frame on the wire.
pub fn tag_for(dst_node: usize, msg_bytes: &[u8]) -> Option<[u8; TAG_LEN]> {
    if dst_node >= MAX_NODES { return None; }
    let key = unsafe { (*core::ptr::addr_of!(SESSION_KEYS))[dst_node] }?;
    Some(*blake3::keyed_hash(&key, msg_bytes).as_bytes())
}

/// RX-side MAC check: verify `received` over `msg_bytes` under the session key
/// for `src_node`. False if no session (can't be from a known peer) OR the tag
/// mismatches. Data-independent compare - no early return on first bad byte.
pub fn verify_from(src_node: usize, msg_bytes: &[u8], received: &[u8; TAG_LEN]) -> bool {
    if src_node >= MAX_NODES { return false; }
    let key = match unsafe { (*core::ptr::addr_of!(SESSION_KEYS))[src_node] } {
        Some(k) => k,
        None => return false,
    };
    let expected = *blake3::keyed_hash(&key, msg_bytes).as_bytes();
    let mut diff: u8 = 0;
    let mut i = 0;
    while i < TAG_LEN {
        diff |= expected[i] ^ received[i];
        i += 1;
    }
    diff == 0
}
