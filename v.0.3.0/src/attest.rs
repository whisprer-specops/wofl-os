//! Layer 6f: HMAC capability attestation.
//!
//! Every remote IPCMessage carries a 32-byte tag computed over its byte image
//! under a shared symmetric key. Verified in `deliver_remote` before any
//! delivery, right beside the existing `dst_cap.node_id != NODE_ID` filter.
//!
//! Primitive: BLAKE3 in keyed-hash mode — the crate's native MAC construction,
//! not a bolted-on HMAC. Chosen for `no_std` + pure-Rust portability on
//! riscv64gc; swap-site is this module, one place.
//!
//! ============================================================
//! !!!!! STEPPING-STONE KEY — DO NOT SHIP AS-IS !!!!!
//! ============================================================
//! This constant is baked into every kernel binary. It authenticates
//! "possesses the same open-source binary," NOT "is a trusted peer."
//! Anyone with the repo has the key. This is deliberate at L6f — real
//! key provisioning (per-node keys, exchange during discovery) is L7
//! territory. Grep `HMAC_KEY` to find the upgrade site later.
//! ============================================================

pub const HMAC_KEY: [u8; 32] = *b"woflOS-L6f-stepping-stone-key!!!";
// exactly 32 bytes — the array type checks it at compile time

pub const TAG_LEN: usize = 32;

/// Compute the tag over a message. TX-side use.
pub fn tag(msg_bytes: &[u8]) -> [u8; TAG_LEN] {
    *blake3::keyed_hash(&HMAC_KEY, msg_bytes).as_bytes()
}

/// Verify a received tag against a message. RX-side use, runs in IRQ context
/// (SIE=0 — no locks, no allocation, no blocking). Compare is data-independent
/// (no early return on first mismatched byte) — timing-attack hygiene, cheap.
pub fn verify(msg_bytes: &[u8], received: &[u8; TAG_LEN]) -> bool {
    let expected = tag(msg_bytes);
    let mut diff: u8 = 0;
    let mut i = 0;
    while i < TAG_LEN {
        diff |= expected[i] ^ received[i];
        i += 1;
    }
    diff == 0
}
