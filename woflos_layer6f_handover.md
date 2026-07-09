# woflOS Handover — Layer 6 COMPLETE (a→f all verified) → Layer 7 opens next

**Prepared:** July 9, 2026
**For:** Next Claude instance picking up woflOS
**From:** Claude (the session that verified L6e, added distinct MACs, and built+verified L6f HMAC attestation)
**Status:** 🟢 **Layer 6 is done.** All six sub-layers (a through f) are built, booted, and sealed with pushed tags. The distributed-native seam is now both real AND authenticated — a forged or tampered capability gets rejected, not silently accepted.

---

## 👋 QUICK ORIENTATION

You're helping **wofl** (CEO of RYO Modular, founder of whispr.dev, ~2.5 yrs Rust) build **woflOS** — a capability-based, **distributed-native** microkernel in Rust for RISC-V (S-mode under OpenSBI, `riscv64gc-unknown-none-elf`, `no_std`).

**Communication style:** casual, Welsh-inflected, calls you "fren," peer-level. Complete `cat > file << 'EOF'` blocks or guarded `python3` assert-then-replace patches, every block ending in an `echo`/`grep` GATE. The *why* behind every change, tradeoffs explicit, gotchas surfaced before they bite. Work real build/QEMU errors one at a time.

**⚠️ The `userPreferences` "elite OS architect" persona is REAL and deliberately maintained for woflOS.** Honor its substance: think through ISA/toolchain constraints before code, complete compilable artifacts, proactively flag UB/alignment/page-flag gotchas, explain tradeoffs. Warm register, high bar.

**This session ran long and hit real friction — none of it damaged anything, all of it got resolved.** Full list in "SCARS THIS SESSION" below so you don't have to relearn any of it the hard way. Short version: a stray second git clone drifted on the Windows side, a hardcoded slice length silently truncated the HMAC tag off the wire, and several multi-command bash blocks got mangled on paste. Every one was caught by an honest gate failing, not by luck.

---

## 🖥️ CANONICAL ENVIRONMENT — THE SURFACE (unchanged, P52 stays dead)

- **Machine:** Surface, WSL2 Ubuntu 22.04, **user `root`** (prompt `root@wofls-surface`). No `sudo` needed.
- **Project:** `~/code/wofl-os` (ext4). Crate at `~/code/wofl-os/v.0.3.0`, `.git` at repo root one level up.
- **⚠️ THIRD-COPY HAZARD IS NO LONGER HYPOTHETICAL — IT HAPPENED THIS SESSION.** A live clone existed at `/mnt/c/github/wofl-os` (Windows-native NTFS path, opened via VSCode). It had genuinely uncommitted work in it (the original blake3 dependency + smoke-test addition) that turned out to *already* be present in the canonical tree too — a wrong-file recon (`main.rs` at crate root vs `src/main.rs`) made it look absent when it wasn't. The C:\ copy has since been **deleted** (`rm -rf /mnt/c/github/wofl-os`) after confirming zero unique commits and zero unique uncommitted content. **If a `/mnt/c/...` or any non-`~/code/wofl-os` copy ever reappears, do NOT assume it's scenery — check `git log`, `git status -sb`, AND confirm you're grepping the right path before concluding anything is "missing."**
- **VSCode is now correctly attached via the WSL extension** — launch with `code .` from inside `~/code/wofl-os/v.0.3.0` in a WSL terminal, not by browsing to a Windows path. This was fixed this session; if a future session's VSCode is showing a `C:\...` path bar again, stop and re-attach before trusting anything it shows you.
- **Toolchain, QEMU version, case-sensitivity scar:** all unchanged from the L6e handover — nightly pinned, QEMU 6.2 needs `-global virtio-mmio.force-legacy=false`, watch for `Cargo.toml` casing issues.

### Build & Run
```bash
cd ~/code/wofl-os/v.0.3.0
./build.sh                                  # writes target/riscv64gc-unknown-none-elf/release/woflos (node 0 shape)
NODE_ID=1 ./build.sh                        # OVERWRITES THE SAME PATH with the node-1 shape — see scar #3 below
```

**⚠️ NEW LESSON THIS SESSION — NAMED STASHES, ALWAYS:** `build.sh` always writes to the same output path regardless of `NODE_ID`. Immediately after EVERY build, `cp` it to a distinct name — `/root/woflos-node0` and `/root/woflos-node1` — and boot ONLY from those named paths, never from the bare `target/...` path directly. This session lost real time to both kernels silently being the same binary because the shared path got overwritten between builds and a boot happened before the re-copy.

**Canonical two-node QEMU lines (now with distinct MACs, per-node named binaries):**
```bash
# Node 1 — run FIRST, wait for "node 1 up - listening" before starting node 0
qemu-system-riscv64 -machine virt -cpu rv64 -smp 1 -m 128M -nographic -no-reboot \
  -global virtio-mmio.force-legacy=false \
  -netdev socket,id=n0,mcast=230.0.0.1:5555 \
  -device virtio-net-device,netdev=n0,mac=52:54:00:00:00:01 \
  -kernel /root/woflos-node1 2>&1 | tee node1.txt

# Node 0 — run SECOND
qemu-system-riscv64 -machine virt -cpu rv64 -smp 1 -m 128M -nographic -no-reboot \
  -global virtio-mmio.force-legacy=false \
  -netdev socket,id=n0,mcast=230.0.0.1:5555 \
  -device virtio-net-device,netdev=n0,mac=52:54:00:00:00:00 \
  -kernel /root/woflos-node0 2>&1 | tee node0.txt
```
No `-bios`. Foreground + tee. Quit: Ctrl+A, X. Node 1 never self-exits (permanent wfi listen loop).

---

## 🟢 MILESTONE LADDER — Layer 6, NOW COMPLETE

| Tag | What it proves | Trust |
|---|---|---|
| `layer6a-virtio-probe` through `layer6d-first-rx` | unchanged from prior handover | ✅ booted |
| `layer6e-seam-closed` | **RE-SEALED this session** — was pushed pre-verification-boot last handover; verified two-node green THIS session, junk file (`node1.txtcd`) removed, tag re-pointed at a clean, rebased, junk-free commit | ✅ booted + sealed |
| **`layer6f-hmac-attestation`** | **NEW.** BLAKE3-keyed HMAC over the wire IPCMessage, verified in both directions (matching key delivers, wrong key rejects) | ✅ booted + sealed |

**Frame-ledger floor is STILL 7** — HMAC added no new driver-owned frames, only grew the wire payload size. Unchanged invariant.

**Wire size grew:** L6e frame was 14(eth)+4(WOFL)+360(IPCMessage) = 378 bytes payload (390 with 12B vnet header). **L6f adds a 32-byte BLAKE3 tag** → 410 bytes payload (422 with vnet header). Compile-time assert at `virtio.rs` (~line 490) confirms this still fits `RX_BUF_LEN=512` with 90 bytes to spare — margin is tighter than before L6f; if a future layer bumps `IPC_PAYLOAD_MAX` (currently 256, design target 4096), the assert will correctly fail the build rather than silently truncate DMA. Grow `RX_BUF_LEN`/buffer scheme together with any payload increase.

---

## 🔐 WHAT L6f BUILT (architecture)

**New file: `src/attest.rs`** (~30 lines). Holds:
- `HMAC_KEY: [u8; 32]` — a shared, symmetric, compile-time-baked constant. **LOUDLY COMMENTED AS A STEPPING STONE, NOT A REAL SECURITY BOUNDARY.** It authenticates "possesses the same open-source binary," not "is a trusted peer" — anyone with the repo has the key. Real per-node key provisioning via discovery/exchange is explicit L7 work. Grep `HMAC_KEY` to find the upgrade site.
- `tag(msg_bytes) -> [u8; 32]` — `blake3::keyed_hash(&HMAC_KEY, msg_bytes)`. TX-side.
- `verify(msg_bytes, received_tag) -> bool` — recomputes and does a **data-independent** (branchless XOR-accumulate) comparison, not early-exit-on-first-mismatch. Cheap timing-attack hygiene; on a single-hart toy kernel this is more principle than necessity, but it's the right habit for the primitive to have baked in from day one.

**Dependency:** `blake3 = { version = "1", default-features = false }` in `Cargo.toml`. Confirmed the portable (non-SIMD) backend compiles clean for `riscv64gc-unknown-none-elf` `no_std` — no missing intrinsics, no `alloc` demand issues. This was proven in isolation with a throwaway smoke-test function before being wired into real logic, and the smoke test was fully removed afterward — `main.rs` carries no trace of it now.

**What gets MAC'd:** the WHOLE 360-byte `IPCMessage` (wofl's explicit call — cap-alone was the cheaper alternative, whole-message was chosen because it also kills payload tampering and pairs naturally with the existing `nonce` field for future replay defence). NOT the `"WOFL"` magic (that's framing, not payload) and NOT the vnet/eth headers (network plumbing, not IPC content).

**Wire format, updated:**
```
[vnet hdr 12B][eth hdr 14B]["WOFL" 4B][IPCMessage 360B][BLAKE3 tag 32B]
```
Total 422B (was 390B pre-L6f).

**TX side — `net_send_ipc` in `virtio.rs`:** grew the payload buffer from `[u8; 4+MSG]` to `[u8; 4+MSG+TAG]`, computes `attest::tag(bytes)` over the IPCMessage bytes, appends it after the message.

**RX side — `deliver_remote` in `ipc.rs`:** verify is the **OUTERMOST gate**, checked BEFORE the `dst_cap.node_id != NODE_ID` party-line filter and BEFORE the checksum zero-recompute-restore dance. Order matters: HMAC authenticates the raw wire bytes as received; everything downstream treats the message as trusted structure only after that passes. Runs in IRQ context (invariant: SIE=0, no locks, no allocation, no blocking) — a single `blake3::keyed_hash` call plus a branchless compare, nothing that can stall. On mismatch: `[IPC] remote: HMAC verify FAILED - dropped ({} bytes)`, frame dropped silently to the sender (no reply), noisy locally.

**The real bug this session found (see SCARS below for the full story):** `rx_parse` in `virtio.rs` builds the `body` slice handed to `deliver_remote` with `core::slice::from_raw_parts(pl.add(4), MSG)` — hardcoded to the OLD 360-byte length. This silently truncated the 32-byte tag off before `deliver_remote` ever saw it, even though the frame on the wire was the correct new size. Fixed to `MSG + TAG`. The length guards at the call site (both the outer `if` and the inner magic-check) were also widened to require `MSG + TAG`, not just `MSG`.

---

## 🔧 NEW CRITICAL INVARIANTS (additive to L0–6e, all of which still hold)

1. **HMAC verify runs BEFORE the node-id filter, deliberately.** Every node does full crypto on every mcast frame including ones it will immediately drop as not-for-me. This is a CPU-vs-timing-leak tradeoff, chosen for correctness of the design even though it's more cycles than filter-first would cost. Don't "optimize" this ordering without understanding why it's there.
2. **The HMAC_KEY must be IDENTICAL across every node's build.** Unlike `NODE_ID` (which deliberately varies per node via `option_env!`), this is a single shared symmetric secret. It must NOT be parameterized per-build. If a future you is tempted to make it `option_env!`-driven like NODE_ID, stop — that would break every node's ability to verify every other node.
3. **Named binary stashes, not the shared `target/` path, for anything you intend to boot.** See the build-section warning above. This bit us for real this session.
4. **`pid == 7` on the LOCAL sender is NOT proof of remote delivery.** It only reflects `send_remote` returning `Ok` locally — there is no blocking wait on an actual remote reply in the current client.rs flow. This session watched pid 2 exit 7 in BOTH a fully-working run AND a run where the far node was silently dropping every frame with a truncated tag. **The only real end-to-end proof is the `REMOTE DELIVERED` line appearing on the RECEIVING node's own console.** Don't trust the sender's exit code as an IPC-arrived gate again.
5. All L0–L6e invariants (dma_fence, PLIC claim/complete pairing, SIE=0 sacred, RX-before-frames-can-arrive, checksum zero-recompute-restore, mcast party-line filter) still hold unchanged.

---

## ⚠️ SCARS THIS SESSION (read this before you repeat any of it)

1. **The C:\ copy drift.** A second git clone at `/mnt/c/github/wofl-os` (Windows-native, VSCode-opened) had uncommitted changes that LOOKED unique but were actually already present in canonical — a recon check grepped the wrong file (`v.0.3.0/main.rs` at crate root, which doesn't exist as a separate file from `src/main.rs`... actually the wrong PATH, not filename — the point is: **always confirm you're checking the exact same relative path in both locations before concluding something is "only in one place."** Resolved: work rescued was already redundant, C:\ copy deleted, VSCode re-pointed at WSL via `code .`.
2. **The `node1.txtcd` / L6e seal took THREE tries** because: (a) the remote had grown 4 unpushed handover-doc commits mid-session (harmless, disjoint paths, but the naive push got rejected as non-fast-forward), and (b) `client.bin` (a checked-in COMPILED ARTIFACT that changes on every rebuild even with identical source — worth flagging as a real design debt, see below) was sitting dirty and blocked the rebase until stashed. Lesson banked: **fetch before reconciling any push rejection, and always check `git status -sb` for dirty tracked build-artifacts before a rebase.**
3. **The shared `target/...` build output path** caused both QEMU panes to boot the SAME (node-1-shaped) binary without either of us noticing until the log showed `node 1 up - listening` printed from what was supposed to be the node-0 pane. Now always using named stashes (see build section).
4. **The `rx_parse` body-slice truncation bug** — the actual interesting bug of the session. A hardcoded `MSG`-length slice silently dropped the newly-added 32-byte tag before verification ever ran, causing `deliver_remote` to correctly-but-confusingly report "short frame (360 bytes, need 392)". Caught because the boot gate demanded seeing an ACTUAL `REMOTE DELIVERED` line, not just "no crash." Fixed by widening the slice AND both length guards to `MSG + TAG`.
5. **Multi-command bash blocks wrapped in `( set -e ... )` subshells repeatedly got mangled on paste** — `-bash: syntax error near unexpected token '('`, `-bash: !crate: event not found` (bash history-expansion eating a literal `!` inside pasted code). **Lesson: prefer flat, sequential, unwrapped commands over parenthesized subshells or heavily-chained `&&`/`||` blocks when handing off to this user's terminal.** Simpler blocks survived every time; clever ones didn't.
6. **`client.bin`** is a compiled userspace binary checked into git (not gitignored like `target/`). It changes byte-for-byte on every rebuild even when source is unchanged (confirmed: same size, different content — points to a non-determinism source, likely an embedded timestamp or build path). This will keep generating phantom dirty-tree state on every future build. **Flagged as real debt, not yet fixed** — either gitignore it and generate at build time, or pin the build to be reproducible. Low urgency, but it WILL bite the next rebase-under-pressure again if left alone.

---

## ✅ IMMEDIATE NEXT STEPS

**Layer 6 is DONE.** Per the prior handover's own words: *"After L6f, Layer 6 is COMPLETE and Layer 7 (discovery, Raft, migration — the thesis) opens."* That's now true.

1. **Layer 7 design conversation, fresh session, fresh context.** This handover exists specifically so that conversation can start clean rather than carrying this session's token weight. Likely opening topics per the L6e handover's own forward-pointer: node discovery (replacing the crude `option_env!("NODE_ID")` with something real), Raft or equivalent consensus, process/capability migration between nodes — "the thesis" the whole distributed-native architecture has been building toward.
2. **Low-priority cleanup debt, whenever convenient, NOT blocking L7:**
   - `client.bin` non-determinism (scar #6 above).
   - Distinct-MAC follow-through: broadcast dst is still used everywhere (`ff:ff:ff:ff:ff:ff`); unicast-by-MAC is now trivially possible since nodes have honest distinct addresses, but hasn't been implemented.
   - TX still polled (not interrupt-driven) — fine at current traffic levels.
   - The inherited L0–L6e pile: `IPC_PAYLOAD_MAX` 256→4096 (build-guarded), `Capability` struct duplicated in `user/src/wofl.rs` (hoist to a shared `wofl-abi` crate), one-page user stacks, `ENDPOINT_DEPTH=4`/`MAX_PROCS=4`, various dead-code warnings (`print_hex`, `PTE_G`, `enable_timer`, `create_test_user_context`), the three `static_mut_refs` warnings on `NIC` (needs-locks-at-SMP ledger, same single-hart justification as ENDPOINTS/TABLE/CURRENT).

None of the above blocks starting Layer 7. They're background noise, logged so nobody has to rediscover them.

---

## 🗣️ STYLE NOTES (delta from previous handover — all previous ones still stand)

- **Prefer flat sequential bash commands over parenthesized `( set -e ... )` subshells or heavily-chained blocks.** This session lost time to paste-mangling on the fancier constructs, every time. Simple survives; clever doesn't.
- **The real end-to-end gate for any future remote-IPC work is the RECEIVER's own console line, never the sender's exit code.** Say this explicitly if a future boot's gate criteria ever get fuzzy again.
- Recon-before-patch discipline (grep the actual current code, exact anchor lines, guarded python assert-then-replace) worked flawlessly again this session for every stage of L6f — kept it, keep it.
- Hold the seal until green is called, still true, still worked — L6f's commit didn't happen until both the positive AND negative boot were confirmed.
- If a second git-tree copy of ANYTHING ever surfaces again, treat it as a live hazard requiring `git log`/`git status -sb` verification, not scenery to ignore.

---

*End of handover. Layer 6 opened with a virtio probe and closed with two kernels proving they can tell a real capability from a forged one — matching key delivers, wrong key drops, same frame, same size, only the secret differs. That's the difference between "the wire works" and "the wire can be trusted." Six sub-layers, a real bug caught by an honest gate, and a project-drift near-miss resolved clean. Layer 7 — discovery, consensus, migration, the actual thesis — is next. Go careful, go clean, name every anchor before you touch it.* 🐺
