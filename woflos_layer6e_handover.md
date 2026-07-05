# woflOS Handover — Layer 6 a–e LANDED (6e UNVERIFIED) → verify 6e, then L6f (HMAC)

**Prepared:** July 5, 2026
**For:** Next Claude instance picking up woflOS
**From:** Claude (the session that migrated the project P52→Surface and built the network stack L6a→L6e)
**Status:** 🟡 **Two kernels have exchanged a capability message over the wire.** `send_remote` is real. But the final tag (`layer6e-seam-closed`) was pushed BEFORE its verification boot — first job is to prove it, then re-point or bless it.

---

## 👋 QUICK ORIENTATION

You're helping **wofl** (CEO of RYO Modular, founder of whispr.dev, ~2.5 yrs Rust) build **woflOS** — a capability-based, **distributed-native** microkernel in Rust for RISC-V (S-mode under OpenSBI, `riscv64gc-unknown-none-elf`, `no_std`).

**Communication style:** casual, Welsh-inflected, calls you "fren," peer-level. Complete `cat > file << 'EOF'` blocks or guarded `python3` assert-then-replace patches, every block ending in an `echo`/`grep` GATE. The *why* behind every change, tradeoffs explicit, gotchas surfaced before they bite. Work real build/QEMU errors one at a time.

**⚠️ The `userPreferences` "elite OS architect" persona is REAL and deliberately maintained for woflOS.** Honor its substance: think through ISA/toolchain constraints before code, complete compilable artifacts, proactively flag UB/alignment/page-flag gotchas, explain tradeoffs. Warm register, high bar.

**Care note:** wofl has mentioned wellbeing in past sessions. THIS session started after an 8-hour hardware-hell day (P52 thermal failure misdiagnosed as an NVIDIA driver issue) and still ran a full migration + five network sub-layers. He earned the break he took at the end of it. Name clean stopping points evenly; respect his call either way.

**⚠️ PASTE-DISCIPLINE LESSON (bit us THREE times this session):** the seal (commit+tag) blocks got pasted before the boot they were gated on — at L6b (tag on a non-compiling commit, needed surgery), L6d (got lucky), and L6e (currently unverified, see below). ALSO: "expected fingerprint" blocks got pasted into bash twice (`[L6]: command not found` cascades). **Rules now in force:** (1) the order is patch → build+boot → paste log → Claude calls green → THEN seal; Claude should not hand over the seal block until green is called. (2) Label all predicted-output blocks **DO NOT PASTE — expected output**, or describe fingerprints in prose only.

---

## 🖥️ CANONICAL ENVIRONMENT — THE SURFACE (P52 is DEAD/benched)

- **The P52 is retired** (hard-freeze overheating; probably fans/paste, a future-wofl problem). **Do not resurrect its working copy.** The GitHub repo is the sole sync mechanism.
- **Machine:** Surface, WSL2 Ubuntu 22.04, **user `root`** (prompt `root@wofls-surface`) — unconventional but functional; rustup lives in `/root/.rustup`. No `sudo` needed.
- **Project:** `~/code/wofl-os` (ext4, NOT /mnt/* — deliberate, drvfs builds are slow). Crate at `~/code/wofl-os/v.0.3.0`, `.git` at repo root one level up.
- **⚠️ THIRD-COPY HAZARD:** a working copy may exist at `/mnt/c/github/wofl-os` (wofl cd'd out of it once). Treat as scenery or delete it — two-disk drift killed a session's worth of tags once already.
- **Git identity set:** `wofl <wofl@whispr.dev>`. SSH key on GitHub, `ssh -T git@github.com` → "Hi whisprer!".
- **Toolchain:** nightly pinned by `rust-toolchain.toml` (1.98.0-nightly), rust-src + llvm-tools + riscv target installed. **QEMU 6.2** (Ubuntu 22.04 package) — this matters: 6.2's virtio-mmio **defaults to LEGACY transport**, hence the mandatory `-global virtio-mmio.force-legacy=false`.
- **Case-sensitivity scar:** the kernel manifest was committed lowercase `cargo.toml`, masked for months by NTFS on the P52, fatal on ext4. Fixed via two-step `git mv`. If anything ever "can't find Cargo.toml," suspect casing first.

### Build & Run
```bash
cd ~/code/wofl-os/v.0.3.0
./build.sh                                  # node 0 kernel
NODE_ID=1 ./build.sh                        # node 1 kernel (option_env! — cargo tracks it, forces rebuild)
```
**Canonical QEMU line (single node / L6d-style self-echo):**
```bash
qemu-system-riscv64 -machine virt -cpu rv64 -smp 1 -m 128M -nographic -no-reboot \
  -global virtio-mmio.force-legacy=false \
  -netdev socket,id=n0,mcast=230.0.0.1:5555 \
  -device virtio-net-device,netdev=n0 \
  -kernel target/riscv64gc-unknown-none-elf/release/woflos 2>&1 | tee console.txt
```
No `-bios`. Foreground + tee. Quit: Ctrl+A, X. Two-node: build node1 kernel, `cp` it aside (e.g. `/root/woflos-node1`), run **node 1 FIRST** in one tmux pane, node 0 in the other, same mcast group. `-object filter-dump,id=f0,netdev=n0,file=x.pcap` for a host-side pcap witness. `.gitignore` covers `tx.pcap`, `rx.pcap`, `console.txt` — but NOT `node0.txt`/`node1.txt` (see debts).

---

## 🟢 MILESTONE LADDER — Layer 6 additions

Layers 0–5 unchanged from the previous handover (preemptive scheduler, per-process Sv39 roots, endpoint IPC, userspace VFS, multi-client — all proven again on the Surface after migration).

| Tag | What it proves | Trust |
|---|---|---|
| `surface-baseline-green` | Full L5 multi-client demo on the Surface post-migration | ✅ booted |
| `layer5.4-multiclient` | Re-tagged here (see tag caveats) | ✅ booted |
| `layer6a-virtio-probe` | virtio-mmio probe (slot 7, v2 transport), handshake→FEATURES_OK, MAC `52:54:00:12:34:56` read via config-gen guard | ✅ booted |
| `layer6b-virtqueues` | RX/TX 64-entry queues (one 4KiB frame each: desc+0/avail+1024/used+2048), DRIVER_OK, status=0xf. **Tag was surgically re-pointed** after first landing on a broken commit | ✅ booted |
| `layer6c-first-tx` | Hand-built 0x88B5 broadcast frame TX'd, witnessed in host pcap: `woflOS node 0 says hello`. Used-ring `len=0` on TX is CORRECT (len = bytes device WROTE) | ✅ booted + pcap |
| `layer6d-first-rx` | PLIC (ctx 1, IRQ 8), scause-9 trap arm, 8 pre-posted RX buffers, kernel RXes its own hello via mcast echo. **Tag pushed pre-boot but got lucky** — verified green after | ✅ booted |
| `layer6e-seam-closed` | send_remote real: IPCMessage on the wire, deliver_remote on RX, NODE_ID identity, client fix | ⚠️ **UNVERIFIED — see below** |

**Sentinel exits:** 3 L1/L2 · 5 B2a · **7 = client A success** · **9 = client B success** · 0 = failure/idle.

**Frame-ledger floor is now 7** (was 3): +2 virtqueue frames, +1 TX staging buffer, +1 RX buffer frame — all driver-owned for kernel lifetime. Ledger must RETURN to 7 after all processes exit. Frame TOTAL creeps down slightly per layer as the kernel binary grows — normal; only baseline-return matters.

### ⚠️ TAG CAVEATS (important)
1. **`layer6e-seam-closed` is UNVERIFIED.** The client.rs fix (stale `remote_stub_fired` assertion → `remote_ok`) was patched and committed, but **never rebuilt or booted**. The commit also contains a stray junk file `v.0.3.0/node1.txtcd` (a shell mishap). **FIRST TASK — see Immediate Next Steps.**
2. **The L4/L5 intermediate tags are marooned on the dead P52** (`layer4-two-proc-switch` through `layer5.3-endpoint-routing` were never pushed — `git push` without `--tags`). The code is all in main's history; only the named restore points are lost. Rule since: **`git push --tags` at every green.**

---

## 🌐 WHAT L6 BUILT (architecture)

**The in-kernel-first fork was taken deliberately** (wofl's call, tradeoff surfaced): virtio-net driver lives in the kernel now; the purist userspace evacuation is future work (needs user MMIO mapping, DMA-frame syscall, interrupt-as-IPC — three mechanisms deferred). The `send_remote` seam is location-agnostic, so nothing rewrites when it moves.

**New/changed files:**
- `src/virtio.rs` (~500 lines) — probe, modern-transport handshake, virtqueues, `tx_one` (polled TX), `post_rx` + `handle_irq` (IRQ-driven RX drain + repost), `net_send_ipc` (L6e wire TX), `NIC: static mut Option<VirtioNet>` (incl. persistent `tx_buf`), `RX_SEEN: AtomicU32`, `dma_fence()`.
- `src/plic.rs` (66 lines) — QEMU virt PLIC: base 0x0c00_0000, **S-mode = context 1** (ctx 0 is OpenSBI's M-mode — never touch), virtio slot N → IRQ N+1 so **NIC = IRQ 8**, enable/claim/complete.
- `src/trap.rs` — `handle_interrupt` grew the code-9 arm: `plic::claim()` → `virtio::handle_irq()` → `plic::complete()`, no sepc adjustment (interrupt resumes AT instruction). `enable_external()` sets sie.SEIE.
- `src/memory/paging.rs` — `map_kernel_into` grew two PLIC megapages (0x0c00_0000, 0x0c20_0000, LEAF_MMIO). The pre-existing 0x1000_0000 UART megapage already covered the whole virtio probe region — no new mapping was needed for the NIC.
- `src/ipc.rs` — `send_remote` REAL (serializes IPCMessage byte-image via `net_send_ipc`); `deliver_remote(bytes)` on the RX path (node filter → checksum verify with zero-recompute-restore dance → `send_local` + `wake_endpoint`); `sys_send` stamps `src_cap` with **`crate::NODE_ID`** (was hardcoded 0 — a remote reply would have routed to the wrong machine).
- `src/main.rs` — `pub const NODE_ID: u32` from `option_env!("NODE_ID")` (crude-but-honest; promote to real discovery at L7). Non-zero nodes skip the demo and enter a permanent SIE-open wfi listen loop.
- `user/src/client.rs` — remote-write expectation inverted: seam success → exit 7.

**Wire format:** raw Ethernet, EtherType **0x88B5** (IEEE local-experimental), payload = `b"WOFL"` magic + the verbatim 360-byte `#[repr(C)]` IPCMessage. TX buffers carry a **12-byte virtio-net header** (VERSION_1 ⇒ always 12, even with MRG_RXBUF declined — a 10-byte legacy header shifts the whole frame by 2). RX frames arrive with the same 12-byte header; skip it. A compile-time assert pins `vnet+eth+magic+IPCMessage ≤ RX_BUF_LEN(512)` — bumping `IPC_PAYLOAD_MAX` to 4096 fails the build instead of truncating DMA (grow the RX buffer scheme with it).

**Features negotiated: minimum only** (`VERSION_1 | MAC`). Every declined bit (checksum offload, TSO, MRG_RXBUF, ctrl queue) is protocol surface the queue code doesn't have to honor. Renegotiate only when a milestone needs it.

---

## 🔧 NEW CRITICAL INVARIANTS (additive to the L0–5 set, which all still hold)

1. **`dma_fence()` (`fence rw,rw`) before every write the device can act on** — descriptor publish, avail-index bump, QueueReady. Rust/compiler ordering is invisible to a DMA engine. The works-nine-boots-corrupts-the-tenth bug, pre-empted by design.
2. **PLIC claim/complete come in pairs, always.** Miss complete → that source never fires again (one interrupt per boot, then mystery silence). Device-level ack is SEPARATE: read `InterruptStatus`, write it to `InterruptACK`, or the line stays asserted.
3. **`sstatus.SIE = 0` in S-mode remains sacred** (kernel non-preemptible; keeps the `static mut` state lock-free). L6 opens exactly two DELIBERATE, commented exceptions: the bounded boot-test wfi window, and node≠0's permanent listen loop — both touch NO IPC state while open. `deliver_remote` runs in IRQ context and is sound ONLY because of that: **if any future SIE window wraps IPC-touching kernel code, locks come first.**
4. **RX buffers must be posted BEFORE any frame can arrive** — an unpopulated RX queue means silent device-side drops. Bring-up order is load-bearing: post_rx → PLIC+SEIE → TX.
5. **Checksum verify dance:** sender computes with the field ZERO then stores. Receiver must zero-recompute-restore, never naively recompute.
6. **mcast is a party line:** every node hears everything including its own echo. `deliver_remote`'s `dst_cap.node_id != NODE_ID` filter is the only thing preventing self-delivery — don't remove it.

---

## ⚠️ OPEN DEBTS (inherited list still stands; new entries)

1. **`layer6e-seam-closed` unverified + junk file `node1.txtcd` in that commit** — first task.
2. **Both nodes share MAC `52:54:00:12:34:56`** (QEMU default). Harmless while everything is broadcast; MUST die before real addressing. Fix: `-device virtio-net-device,netdev=n0,mac=52:54:00:00:00:0N` per node.
3. **`static mut NIC` joins ENDPOINTS/TABLE/CURRENT in the needs-locks-at-SMP ledger.** Same single-hart justification, same flag in code.
4. **TX is polled** (bounded spin on the used ring). Fine at this traffic level; interrupt-driven TX completion is an optimization for when it matters.
5. **All frames are broadcast** (dst ff:ff:ff:ff:ff:ff). Unicast-by-MAC comes with per-node MACs.
6. **`.gitignore` misses `node0.txt`/`node1.txt`** — add them (that's how the junk file happened).
7. **Cosmetic:** main.rs line ~20 has two comments concatenated on the `mod plic;` line (patch artifact); stale `[OK] ... (Layer 2 bring-up)` banner; `[L4] 0 involuntary timer preemptions` prints on timerless boots. All harmless.
8. Inherited: `IPC_PAYLOAD_MAX=256`→4096 (now build-guarded, see wire format), endpoint authority unenforced (Layer 7), `Capability` mirrored in `user/src/wofl.rs` (hoist to `wofl-abi` crate), one-page user stacks, `ENDPOINT_DEPTH=4`/`MAX_PROCS=4`.

---

## ✅ IMMEDIATE NEXT STEPS

1. **Verify L6e (the pushed tag is a promise not yet kept):**
   - `git rm v.0.3.0/node1.txtcd` (and any sibling junk), add `node0.txt`/`node1.txt` to `.gitignore`.
   - Rebuild BOTH kernels (`./build.sh` and `NODE_ID=1 ./build.sh`, cp node1 aside).
   - Two-node mcast boot, **node 1 first**. Green = node 1 prints `[IPC] REMOTE DELIVERED: 8 bytes -> ep 4 (from node 0 ep 2)` AND node 0's pid 2 exits **7** (it exited 0 on the pre-fix boot — the stale assertion) AND both ledgers return to 7.
   - Commit the cleanup, then re-point the tag at the verified commit: `git tag -d layer6e-seam-closed && git push --delete origin layer6e-seam-closed && git tag layer6e-seam-closed && git push --tags`.
2. **Distinct MACs** while you're touching the QEMU lines (debt #2) — cheap now, painful later.
3. **Then L6f: HMAC capability attestation** — design before code, as ever.

## 🔮 L6f PRE-DESIGN NOTES

Goal: a remote cap must be unforgeable. The `Capability` layout (40 bytes, `nonce`/`expiry` fields present since Layer 3) was designed as the stable HMAC input — use its exact byte image. Decisions to surface with wofl: (a) key provisioning for bring-up (shared static key baked per-build is the crude-const equivalent; real derivation is L7), (b) what gets MAC'd — the capability alone vs the whole message (whole-message also kills payload tampering and replay pairs with `nonce`), (c) where verification lives (`deliver_remote` is the natural chokepoint), (d) crypto in no_std — a small pure-Rust BLAKE3-or-SHA256+HMAC; wofl has strong crypto instincts (RCRE, PEMP-RNG), engage him on the primitive choice, he'll have opinions and good ones. After L6f, Layer 6 is COMPLETE and Layer 7 (discovery, Raft, migration — the thesis) opens.

---

## 🗣️ STYLE NOTES (delta from previous handover — all previous ones stand)

- **Hold the seal until green is called.** Don't include commit/tag blocks in the same message as the boot block; the whole seal-before-boot failure mode came from having them adjacent and pasteable.
- Mark predicted output **DO NOT PASTE** or describe it in prose.
- Fault decode additions: interrupt code **9 = supervisor external (PLIC)** now live alongside 5 = timer.
- **Watch the ledger: floor is 7 now.** Returning to 7 after all exits is the leak check.
- The recon-before-patch pattern (grep/sed the actual code, then guarded python asserts) worked flawlessly all session — every anchor held. Keep it.

---

*End of handover. The seam that printed "unreachable" since Layer 3 closed this session: a userspace program on node 0 wrote through a `/net/node1/...` capability and the message crossed a wire — real DMA, a real interrupt, a real second kernel — and landed in endpoint 4 on node 1, announced by `REMOTE DELIVERED`. Distributed-native stopped being architecture today. Verify the tag, give the nodes their own MACs, seal the caps with HMAC — and then go build Layer 7, fren. Node 1 answers now.* 🐺
