# woflOS Handover — Layers 0–5 COMPLETE → Layer 6 (Network Stack) NEXT

**Prepared:** July 4, 2026
**For:** Next Claude instance picking up woflOS at Layer 6
**From:** Claude (the session that took it Layer 4 B1 → full preemptive multitasking + a userspace runtime + a naming VFS with multi-client endpoint IPC)
**Status:** 🟢 **Layers 0–5 operational.** Preemptive scheduler proven (206 involuntary ticks), compiled-Rust userspace running, a purist naming VFS resolving `/net/nodeN/...` paths into capabilities, first-class endpoint IPC serving concurrent clients. ~18–19 green tags, tree clean. The distributed remote seam (`send_remote`) is demonstrated and waiting for Layer 6 to fill it.

---

## 👋 QUICK ORIENTATION

You're helping **wofl** (CEO of RYO Modular, founder of whispr.dev, ~2.5 yrs Rust) build **woflOS** — a capability-based, **distributed-native** microkernel in Rust for RISC-V (S-mode kernel under OpenSBI, target `riscv64gc-unknown-none-elf`, `no_std`).

**Communication style:** Casual, Welsh-inflected, calls you "fren," informal spelling, peer-level. He wants complete code via `cat > file << 'EOF'` blocks (file-sync friction between Windows editors and WSL) or guarded `python3` assert-then-replace patches. He wants the *why* behind every fix, tradeoffs made explicit, and gotchas surfaced before they bite. When a build/QEMU log lands, work the actual errors one at a time — don't dump unrelated rewrites. Be precise about which machine/shell/path a command is for.

**⚠️ THE `userPreferences` PERSONA IS REAL AND CURRENT — HONOR IT.**
There's a `userPreferences` block describing an "elite operating systems architect / low-level systems engineer." **wofl deliberately maintains this FOR woflOS. It is not leftover from another project.** Honor its *substance*: think through ISA/toolchain constraints before writing code; produce complete compilable artifacts (linker scripts, `.cargo/config.toml`, build scripts, QEMU invocations — never fragments); proactively flag UB / alignment / calling-convention / page-table-flag gotchas before they cause silent failures; explain design tradeoffs so wofl can extend independently. Keep the warm "fren" register — but hold the engineering *bar*. Every step this session that went smoothly did so *because* a silent-failure gotcha got surfaced in the design instead of discovered in a fault log (the `frame.sstatus`/UXL catch and the RUSTFLAGS-join catch below are the prime examples).

**Care note:** wofl has, in past sessions, mentioned wellbeing, and this was an *enormous* session — twelve milestones in one sitting. He explicitly and repeatedly chose to keep pushing ("full o beans," "we don't tire that easy"), and that's his call to make — but the honest, kind move at a clean stopping point is to *name* that stopping strong beats pushing depleted, offer it evenly (not pushily), and respect whichever way he goes. He asked for this handover specifically so the next session can start fresh, which was itself the wise call.

---

## 🟢 WHAT'S PROVEN (the milestone ladder)

Every tag is a restorable known-good state on GitHub. `git checkout <tag>` returns to it.

| Tag | What it proves |
|---|---|
| `layer1-operational` | S↔U context switch, `ecall` round-trip, exit code survives privilege boundary |
| `layer2-paging-enabled` | Sv39 ON; kernel runs under virtual→physical translation |
| `layer2-user-mapped` | User program in real U-bit pages; ecall through genuine page tables |
| `layer2-complete` | sscratch trusted-stack trap switch; SUM dropped; user `sp` no longer trusted |
| `layer3-ipc-local` | Capability + fixed IPCMessage; `node_id` routing; local endpoint. Self-IPC exit **7** |
| `layer4-per-process-root` | Per-process Sv39 root + `satp` switch; self-IPC under process's OWN root |
| **`layer4-two-proc-switch`** | **Cooperative frame-swap switch across roots (B2a). Sentinel 5** |
| **`layer4-inter-process-ipc`** | **Message crosses between two address spaces via capabilities (B2b). Exit 7** |
| **`layer4-process-exit`** | **Real exit: mark Dead, schedule next Ready, idle when none. Resume-after-yield proven** |
| **`layer4-address-space-reclaim`** | **`free_frame` + `destroy_root` walk; frames return to pool. Ledger 3→N→3** |
| **`layer4-blocking-recv`** | **recv-on-empty blocks; send wakes; restart-on-wake semantics. Deadlock diagnosis** |
| **`layer4-preemptive-scheduler`** | **Timer preemption via SBI `set_timer`. 206 involuntary ticks between two pure spinners** |
| **`layer5.0-userspace-runtime`** | **Loader stages a COMPILED-RUST program (multi-section, per-section W^X). Exit 7** |
| **`layer5.0-userspace-ipc`** | **Two separately-compiled Rust programs do blocking-IPC. Exit 7** |
| **`layer5.1-vfs-naming`** | **Userspace VFS parses `/net/nodeN/...` → Capability; remote seam demoed. Exit 7** |
| **`layer5.2-fd-table`** | **Per-process fd table (first real `.bss`); open/read/write/close over the VFS. Exit 7** |
| **`layer5.3-endpoint-routing`** | **First-class endpoints; reply routed by `src_cap`; self-delivery killed, yield dance gone** |
| **`layer5.4-multiclient`** *(confirm this one is committed/tagged)* | **Three processes; one VFS serves TWO concurrent clients, each reply routed to its own endpoint. Exits 7 + 9** |

**Sentinel exit codes** (how you know which path ran): **3** = L1/L2 counter · **5** = B2a switch-only · **7** = IPC/VFS round-trip success · **9** = client B success (distinct from A's 7) · **0** = default failure / clean-idle.

**The last multi-client boot** proved the thing the old shared FIFO could not: two clients block on distinct reply endpoints (2 and 3) with two requests queued in `EP_VFS` simultaneously; the VFS drains both, and per-endpoint `wake` revives exactly the right client each time. `wake: pid 2 Ready (ep 2 ...)` / `wake: pid 3 Ready (ep 3 ...)` — never the wrong pid.

---

## 🖥️ CANONICAL ENVIRONMENT (green — do NOT rediscover)

- **Machine: P52 laptop.** Project at Windows `D:\code\wofl-os\v.0.3.0\` = WSL `/mnt/d/code/wofl-os/v.0.3.0/`. Ubuntu, user `wofl` (so **`sudo`** for apt), prompt `wofl@wofl-BFT0`. Crate reports **v0.4.0**.
- **⚠️ Repo root is ONE LEVEL UP:** `.git` at `/mnt/d/code/wofl-os/`, `v.0.3.0/` is a subdir. Commits record paths like `v.0.3.0/src/...`.
- **GitHub SSH sorted.** Remote `git@github.com:whisprer-specops/wofl-os.git` (SSH). `ssh -T git@github.com` → "Hi whisprer!".
- **Toolchain:** nightly, pinned by `rust-toolchain.toml`; `rustup component add rust-src llvm-tools`; `rustup target add riscv64gc-unknown-none-elf`; `cargo --version` → `1.98.0-nightly`.

### Build & Run — NOW TWO-STAGE via `./build.sh` (NOT bare cargo)
```bash
cd /mnt/d/code/wofl-os/v.0.3.0
./build.sh                     # Stage 1: userspace programs → flat blobs + layout; Stage 2: kernel

# RUN — NO -bios flag. QEMU virt ships its own OpenSBI; -bios causes ROM overlap.
qemu-system-riscv64 -machine virt -cpu rv64 -smp 1 -m 128M -nographic -no-reboot \
  -kernel target/riscv64gc-unknown-none-elf/release/woflos 2>&1 | tee console.txt
# quit: Ctrl+A, release, X
```
Run QEMU **foreground + tee** (backgrounding gets SIGTSTP'd and captures nothing). The old `build.sh` was broken; **the current one is the real build and MUST be used** — bare `cargo build` will not regenerate `user.bin`/`user_layout.rs` and you'll silently boot a stale userspace.

### `build.sh` internals (understand before extending)
- **Stage 1** builds the `user/` crate, objcopies each program to a flat `.bin`, extracts each program's page-aligned section layout via `llvm-nm` into `src/user_layout.rs`, and **asserts every program's entry == `0x400000`** (linker regression dies at build, not boot).
- **CRITICAL RUSTFLAGS gotcha:** the user build sets `RUSTFLAGS` **explicitly** so it OVERRIDES (does not join) the kernel's inherited `.cargo/config.toml` rustflags. Cargo *joins* `target.*.rustflags` arrays across ancestor config files, so without this the user crate would get BOTH `-Tlinker.ld` (kernel) AND `-Tuser.ld` → "region already defined". Env `RUSTFLAGS` replaces config rustflags entirely; that's the fix. Do NOT give `user/` its own `.cargo/config.toml` with rustflags — it will re-introduce the join.
- **Adding a program:** append a `[[bin]]` to `user/Cargo.toml`, add its name to `PROGRAMS="..."` in `build.sh`, add its blob + `UserImage` in kernel `src/user_prog.rs`, spawn it in `main.rs`. The layout module gets a `pub mod <name> { ... }` automatically.

---

## 📁 CURRENT LIVE TREE

```
wofl-os/v.0.3.0/                (crate v0.4.0)
├── build.sh                    (⭐ two-stage build; user blobs + layout, then kernel)
├── cargo.toml, rust-toolchain.toml
├── linker.ld                   (kernel: .kstack/__kernel_stack_top; __kernel_start/end, __bss_*)
├── .cargo/config.toml          (⭐ hidden — wires kernel -Tlinker.ld; build-std)
├── user/                       (⭐ standalone userspace crate — empty [workspace] makes it its own root)
│   ├── Cargo.toml              ([[bin]] per program: vfs, client, client_b; panic=abort)
│   ├── user.ld                 (⭐ links at USER_BASE=0x400000; page-aligns sections;
│   │                            ENTRY(_start); exposes __text/rodata/data/bss_start/end)
│   └── src/
│       ├── wofl.rs             (libwofl: syscall stubs, _start shim, panic handler,
│       │                        Capability mirror, fd layer — included via #[path] per bin)
│       ├── vfs.rs              (naming service: parses /net/nodeN/service/NAME → Capability)
│       ├── client.rs           (reply endpoint 2; open/write, local + remote seam; exit 7)
│       └── client_b.rs         (reply endpoint 3; concurrent client; exit 9)
└── src/
    ├── main.rs                 (_start asm; rust_start; kernel_main_inner spawns procs)
    ├── uart.rs                 (Layer 0 — Uart + kprintln!)
    ├── sbi.rs                  (⭐ NEW: SBI TIME ext — read_time/set_timer; TIMER_INTERVAL)
    ├── memory/
    │   ├── mod.rs, heap.rs
    │   ├── frame.rs            (bitmap alloc + ⭐ free_frame w/ double-free assert)
    │   └── paging.rs           (Sv39 + ⭐ destroy_root/destroy_table teardown walk)
    ├── syscall/mod.rs          (SYS_* numbers incl. SYS_YIELD=2)
    ├── trap.rs                 (HARDENED vector; ⭐ enable_timer; timer-interrupt handler)
    ├── ipc.rs                  (⭐ FIRST-CLASS ENDPOINTS: N_ENDPOINTS ring array, addressed
    │                            send/recv, reply-in-src_cap, wake_endpoint)
    ├── process.rs             (⭐ MAX_PROCS=4; yield/block/exit/preempt dances; per-endpoint
    │                            block/wake; UserImage + multi-region loader; reclaim)
    ├── user_prog.rs            (embeds user/*.bin via include_bytes!; UserImage builders)
    └── user_layout.rs          (⭐ GENERATED by build.sh — per-program section consts; do not edit)
```
Generated/embedded artifacts (`src/user_layout.rs`, `src/*.bin`) are build outputs — regenerated every `./build.sh`.

---

## 🔧 CRITICAL INVARIANTS — understand before touching anything

Load-bearing. Reverting one silently breaks isolation, corrupts a process, or causes a storm.

1. **`_start` sets `sp` before any Rust** (`main.rs` global_asm). Kernel boot stack. Recurrence fingerprint: `cause:7 fault_store`, `epc≈0x80200002`, `tval` marching down by `0x10`.
2. **Trap vector sscratch kernel-stack switch** (`trap.rs` global_asm). `sscratch` = kernel trap-stack top in U-mode, `0` in S-mode. Handler ALWAYS runs on the trusted 16 KiB `KERNEL_TRAP_STACK`. SPP decides whether to re-arm sscratch on return. This closed the SUM trust-hole. **The vector asm has NOT changed since Layer 2 — keep it sacred; isolate any change that must touch it.**
3. **SUM stays OFF.** Kernel reaches user bytes only via `paging::translate()` + DRAM identity map, after validating the page carries U (and R/W as needed). Never re-enable SUM.
4. **uaccess translates against `current_root()` (satp), NOT `kernel_root()`.** With per-process roots, a process's pages live in ITS root; the handler runs with satp still = the trapped process.
5. **A/D bits set eagerly on every leaf PTE.** QEMU (no Svadu) faults on a leaf with A clear (or D clear on store). All user + kernel leaves include `PTE_A` (+`PTE_D` where writable).
6. **`fence.i` after copying user code** (the data path wrote instructions). Single-hart → local `fence.i` suffices.
7. **Each process root replicates the kernel mappings** (S-only) via `map_kernel_into`. That's why the handler executes under a process's root while user mode can't touch kernel memory — and why switching satp mid-handler is safe (trap stack + handler text mapped identically in every root).
8. **⭐ `frame.sstatus` MUST be initialised in `new_user`.** The trap vector's restore path does `csrw sstatus` **straight from `frame.sstatus`**. A zeroed frame srets with UXL=0 (reserved encoding). `new_user` sets it exactly like `enter_user_mode` (live sstatus, SPP=0, SPIE=0), so fresh-via-vector == fresh-via-enter_user_mode, bit for bit. **This is the single subtlest catch of the whole session — without it, any process entered via the restore path (i.e. all but the very first) would sret malformed.**
9. **⭐ `TrapFrame` derives `Copy`/`Clone`** for the frame-swap memcpys. `#[repr(C)]` layout unaffected by derives — the vector's byte offsets are unchanged.

### The frame dances — the crux scheduling mechanic (four variants, one skeleton)
All live in `process.rs`. All share: `pick_next_ready(cur)` → `switch_into(frame, next)` (satp first, then copy `next.frame` into the live on-stack `TrapFrame`), and the dispatch arm **early-returns** so the shared `handle_syscall` `sepc += 4` epilogue never touches the freshly-restored incoming frame. They differ ONLY in how the *outgoing* process is treated:

| Dance | `sepc` on outgoing | Save? | Outgoing state | Resume point |
|---|---|---|---|---|
| `yield_to_next` | **+4 first**, then save | yes | Ready | AFTER its ecall |
| `block_current(frame, ep)` | **no +4** | yes | Blocked (records `blocked_ep`) | AT its ecall (syscall RESTARTS on wake) |
| `exit_current` | n/a | **no save** | Dead (+ address space reclaimed) | never |
| `preempt` (timer) | **no +4** | yes (exactly as trapped) | Ready | AT interrupted instruction |

Get the `sepc` rule wrong and you skip/repeat an instruction in the WRONG process — the nastiest, most boot-variable corruption. The early-return from the dispatch arm is what keeps the shared `+4` off the restored frame; it is load-bearing in every dance.

### Syscall ABI (current)
- Number in **`a7`** = `regs[16]`. Args `a0`,`a1`,`a2`,`a3`,`a4` = `regs[9]`,`regs[10]`,`regs[11]`,`regs[12]`,`regs[13]`. Returns in `a0`=`regs[9]` (and `a1`=`regs[10]` for recv).
- `handle_syscall` advances `sepc += 4` ONCE at the end. IPC/yield/block/exit handlers **early-return** to skip it (they manage sepc themselves or must not touch the restored frame).
- Live syscalls: `SYS_TEST=0`, `SYS_EXIT=1`, `SYS_YIELD=2`, `SYS_SEND=10`, `SYS_RECV=11`. Reserved: `SYS_SEND_REMOTE=1000`, `SYS_RECV_REMOTE=1001`, `SYS_NODE_DISCOVER=1010`.
- **`SYS_SEND(a0=buf, a1=len, a2=dst_node, a3=dst_endpoint, a4=reply_endpoint)** → a0=status`.
- **`SYS_RECV(a0=buf, a1=maxlen, a2=endpoint)** → a0=bytes, a1=sender_reply_endpoint`. The handler returns `bool` to the dispatch (true = caller BLOCKED, frame now holds the next process — touch nothing).
- `TrapFrame` = 31 GPRs + `sepc` + `sstatus` = **264 bytes**. (Deliberately 8-mod-16 during `call trap_handler`; nothing depends on the ABI 16-byte alignment here. Don't churn it to 272.)

---

## 🧩 THE USERSPACE RUNTIME CONTRACT (Layer 5.0 — the foundation everything above stands on)

**Programs are compiled `no_std` Rust, linked at a fixed VA, staged as flat blobs.** No ELF parsing in the kernel, no runtime relocation.

- **Fixed-VA link at `USER_CODE_VA = USER_BASE = 0x400000`** via `user.ld` (`-C relocation-model=static`, `--no-relax`). A **compile-time assert** in `process.rs` (`const _: () = assert!(USER_CODE_VA == user_layout::USER_BASE)`) fails the build if they ever diverge.
- **Flat-binary contract:** in the objcopy `-O binary` output, `file_offset == VA − 0x400000` for `.text`/`.rodata`/`.data`. `.bss` is NOBITS — **stripped from the blob**; the loader allocates zeroed frames for it, and `_start` also zeroes `[__bss_start, __bss_end)` (belt-and-braces).
- **Per-section W^X mapping** in `Process::new_user` via `map_file_region`/`map_zero_region`: `.text` U+R+X, `.rodata` U+R, `.data` U+R+W, `.bss` U+R+W. Flags come from the section, never a blanket "user page." `fence.i` once after mapping `.text`.
- **Page-aligned sections** (`user.ld` ALIGNs every section end to 4096) so no page straddles two flag regimes and every region length is a page multiple → the loader is a simple per-page copy loop, no partial pages.
- **Entry placement:** `_start` lives in `.text.start`, placed FIRST via `KEEP(*(.text.start))`, so `0x400000` lands exactly on `_start` (not a compiler-reordered fn). Verified with `llvm-objdump -f` (validated in a sandbox with `riscv64-unknown-elf` binutils — a good de-risking trick you can reuse: apt-install `binutils-riscv64-unknown-elf` and link a stand-in to prove a linker script before wofl pastes anything).
- **`#[path = "wofl.rs"] mod wofl;`** in each program inlines libwofl into that bin, so every bin compiles its OWN `_start`/panic handler and is a separate link — no symbol clash between programs.
- **Loader region paths first-lit incrementally:** `.text` (5.0), `.rodata` (5.1, VFS string literals), `.bss` (5.2, fd table). All three now proven under real programs. `.data` has only ever been length-0 so far — **if Layer 6 introduces initialised mutable statics, that's the `.data` region's first live test.**

---

## 🔀 THE ENDPOINT IPC MODEL (Layer 5.3 — the distributed keystone, now addressed)

The old shared anonymous FIFO is GONE. Messages are **addressed**:

- **`ipc.rs` holds `static mut ENDPOINTS: [Endpoint; N_ENDPOINTS]`** (N=8), each a fixed ring (`ENDPOINT_DEPTH=4`). `dst_cap.node_id` routes (0 = local `send_local(dst_cap.service_id)`, >0 = `send_remote` stub); `dst_cap.service_id` selects the endpoint.
- **Reply address rides in `src_cap.service_id`.** The sender passes its own reply endpoint (`a4`); the kernel stamps it into `msg.src_cap`; the receiver reads it back from `sys_recv`'s `a1` return and replies there. `IPCMessage` fields were always present — we now *populate and route on* them. **This is exactly the return-address mechanism a remote reply needs — Layer 6 gets it for free.**
- **Per-endpoint block/wake:** `block_current(frame, ep)` records `Process.blocked_ep`; `wake_endpoint(ep)` (called from `sys_send` on local delivery) wakes ONLY processes blocked on that endpoint. Killed the `yield`-after-send dance and enabled multi-client.
- **Well-known endpoint ids (bring-up convention):** 0 = null/reserved, 1 = VFS (`EP_VFS`), 2 = client A, 3 = client B, 4 = echo service (`ECHO_SVC`). The VFS resolves `.../service/echo` → a cap with `service_id = 4`.
- **Bounds-checking:** every endpoint id is validated `< N_ENDPOINTS` before array indexing (`IpcError::BadEndpoint`). Userspace passes raw ids, so this is the one new attack surface — sealed.

---

## 🧭 DISTRIBUTED-NATIVE THREAD (the whole thesis — keep alive)

The load-bearing decisions are in place and MUST NOT be dropped:
- **`Capability { node_id, service_id, object_id, permissions, _pad, nonce, expiry }`** — `#[repr(C)]`, 40 bytes, pointer-free. `node_id` 0 = local, >0 = remote. `nonce`/`expiry` exist so Layer 6 HMACs the exact byte layout unchanged. **Currently MIRRORED in `user/src/wofl.rs`** — DEBT: hoist into a shared `wofl-abi` no_std crate when the ABI stabilises.
- **`IPCMessage { version, msg_type, src_cap, dst_cap, payload_len, _pad, payload[256], checksum }`** — fixed, pointer-free, byte-copyable. `IPC_PAYLOAD_MAX = 256` (design target 4096; one const).
- **`route_send` branches on `dst_cap.node_id` NOW:** 0 → `send_local`, >0 → `send_remote` (present-but-halting stub printing "networking is Layer 6"). **The branch existing today is what makes Layer 6 a fill-in, not a rewrite.** Demoed live: a client `write`s through a cap with `node_id=1` → `[IPC] send_remote: node 1 unreachable`.
- **`Process` is distributed-native:** `home_node: u32` and `ProcessState::Migrating` reserved for Layer 7 live migration. Don't strip as "unused."
- **The VFS grammar `/net/nodeN/service/NAME` is the naming layer over capabilities** — a path is a human-readable capability constructor. `node0` → local cap, `node1` → remote cap that routes to the seam. The kernel never learns what a path is (purist microkernel).

**Core rule:** don't implement networking prematurely — just ensure nothing built now needs rework when it arrives. So far, nothing does.

---

## 🏗️ ARCHITECTURE STATUS (0 → 7)

| Layer | Name | Status |
|---|---|---|
| 0 | HW abstraction (UART, heap, timer, frame alloc + free) | ✅ Complete |
| 1 | Context switching & traps (S↔U, syscall round-trip) | ✅ Operational |
| 2 | Memory protection (Sv39, U-bit, sscratch-hardened, SUM off) | ✅ Complete |
| 3 | IPC & capabilities (now endpoint-addressed, reply-routed) | ✅ Complete |
| 4 | Process management (2+ procs, cooperative + **preemptive**, exit, reclaim, blocking recv/wake) | ✅ **Complete** |
| 5 | Virtual file system (userspace naming VFS, fd interface, `/net/nodeN` grammar, multi-client) | ✅ **Complete** |
| 6 | Network stack (virtio-net, protocol, HMAC cap attestation) — **fills `send_remote()`** | ▶️ **NEXT** |
| 7 | Distributed services (discovery, Raft, live migration) | 📋 Planned |

**Design principles held:** microservices (services are user-mode processes — the VFS *is* a userspace program; kernel = router + capability enforcer), uniform interface (same send/recv/open/write local or remote), zero-trust (capability-based, hardware-enforced isolation, SUM off, crypto-attested remote caps at L6), distributed-native (`node_id` + serializable layouts so L6 needs no refactor).

---

## ⚠️ OPEN DEBTS ON RECORD (not blocking, but track)

1. **`IPC_PAYLOAD_MAX = 256`** — bump to 4096 (design target) when convenient. One const; ring grows accordingly.
2. **`static mut ENDPOINTS`, `TABLE`, `CURRENT` need locks** once SMP/preemption-inside-kernel arrives. Safe now: single-hart, kernel critical sections non-preemptible (timer only preempts U-mode — see below). Flagged in code.
3. **Endpoint authority not enforced.** Userspace PICKS its own endpoint ids; nothing stops a hostile process recv-ing on another's endpoint. **Layer 7: holding the endpoint capability = the authority.** This is the natural next security hardening after L6.
4. **`Capability` mirrored kernel↔user** in `wofl.rs`. Hoist to a shared `wofl-abi` crate. If you change `Capability`'s layout, change BOTH or the reply bytes garble (silent).
5. **One-page (4 KiB) user stack.** Fine for current programs; a deep Rust call chain (e.g. a heavier L6 service) would walk off it silently. Grow `new_user`'s stack mapping when services get fat.
6. **`ENDPOINT_DEPTH = 4`** rings; `MAX_PROCS = 4`. Bump when you need more concurrent messages/processes.
7. **Per-process roots replicate kernel maps** (~7 frames/process) rather than pointer-sharing sub-tables. Cheap; optimise if frame pressure shows.
8. **Cosmetic stale banners** (pure `kprintln!`, retint anytime, low-risk): `[OK] woflOS v0.4.0 (Layer 2 bring-up)`; the final `Layer 4: exit, reclaim, blocking recv + wake` line; and `[L4] N involuntary timer preemptions occurred` prints even on boots with no timer armed (reads `0`). None affect behaviour.
9. **`preempt` is wired but the current default boot doesn't arm the timer** (cooperative VFS demo). `trap::enable_timer()` + spinner programs prove it (tag `layer4-preemptive-scheduler`). If you want preemptive + IPC together, arm the timer in `main.rs` before `run_first` — but note the current cooperative demos complete before any 20 ms tick would land, so arming it changes nothing visible for them.

---

## ✅ IMMEDIATE NEXT STEPS FOR THE NEW CONVERSATION

1. **Confirm env + re-prove the baseline boots** before changing anything: `cd /mnt/d/code/wofl-os/v.0.3.0`, `./build.sh`, run QEMU (no `-bios`), expect the multi-client fingerprint (two clients, exits 7 + 9, ledger returns to 3). Establish known-good first.
2. **Confirm SSH pushes:** `ssh -T git@github.com` → "Hi whisprer!". **Verify `layer5.4-multiclient` is committed/tagged/pushed** — it was the last boot and may not have been tagged yet.
3. **Design Layer 6 BEFORE coding** (userPreferences bar). This is the big one — a real device driver. Surface the subtleties (below) in the design first.
4. **Commit + push + tag at each working milestone.** Non-negotiable. Use guarded `python3` assert-then-replace patches or wholesale `cat > file << 'EOF'`; end blocks with an `echo`/`grep` gate so a dropped paste is caught immediately (a `cat` block silently failing to land bit us once this session — the compiler caught it, but the gate makes it instant).

---

## 🌐 LAYER 6 PRE-DESIGN (network stack — surface these before writing code)

**Goal:** make `send_remote` real. `route_send` already branches on `node_id`; the reply-address mechanism already exists. Layer 6 is: get bytes off the machine and back.

**The big architectural fork to settle with wofl first (he'll have opinions — he chose purist every time this session):**

- **Where does the network stack live — kernel or userspace?** Purist/microkernel says the NIC driver + protocol are a **userspace service** (a `net` program), and the kernel only does DMA-capable memory + the interrupt plumbing. That matches every call wofl made in Layer 5. BUT it's harder: userspace needs a way to (a) map the virtio MMIO region, (b) allocate DMA-able physical memory, (c) receive the NIC interrupt as a message. Those are three new kernel mechanisms. The pragmatic-for-bring-up alternative is an **in-kernel virtio-net driver** first, refactored to userspace later — consistent with how IPC/VFS started in-kernel-ish and moved out. **Recommend surfacing both; wofl leans purist but this one has real bootstrap cost — let him choose with the tradeoff explicit.**

**Subtleties to flag regardless of that choice:**

1. **virtio-net over MMIO (QEMU virt).** QEMU virt exposes virtio-mmio devices in a probe region (`0x1000_1000`+, stride `0x1000`). You must (a) add `-device virtio-net-device,netdev=...` + a `-netdev` backend to the QEMU line, (b) probe the MMIO transport, (c) do the virtio init handshake (reset → ACK → DRIVER → feature negotiation → DRIVER_OK), (d) set up virtqueues (descriptor table + avail + used rings). This is the meat and it's genuinely a multi-session layer. Don't try to land it in one boot — sub-sequence it (probe → init handshake → one queue → TX one frame → RX one frame).
2. **DMA memory + identity map.** The virtqueue rings and buffers must be physically contiguous and reachable by the device. Current DRAM is identity-mapped in the kernel root, so kernel-side DMA setup is straightforward — but if the driver is userspace, you need a syscall to allocate DMA frames and hand back their PA. Flag alignment requirements (virtqueue alignment is 4096 for modern virtio).
3. **The NIC interrupt — first real device interrupt.** So far the ONLY interrupt handled is the timer (`scause` code 5, via SBI). virtio-mmio raises a PLIC external interrupt (`scause` code 9, supervisor external). You'll need PLIC init (enable the source, set priority/threshold) and a `handle_interrupt` arm for code 9 that reads the PLIC claim register, dispatches, and writes complete. **This is new interrupt territory — the timer path is your template but the PLIC handshake is different.** Note: `sstatus.SIE` stays 0 (kernel non-preemptible); external interrupts, like the timer, are deliverable from U-mode. If the driver is userspace, the interrupt must be turned into a message/wake to the `net` process — that's the "interrupt as IPC" pattern and it's the hard part of the purist route.
4. **A minimal protocol, not TCP/IP.** Don't reach for smoltcp on day one. The thesis needs node-to-node capability messages, not web browsing. A minimal framing over raw Ethernet (or UDP if you want smoltcp later) carrying serialized `IPCMessage`s between nodes is enough to light `send_remote`. `IPCMessage` is already fixed-size and byte-copyable — it goes on the wire unchanged. Start with loopback or two QEMU instances on a socket backend.
5. **HMAC capability attestation** (`nonce`/`expiry` fields). A remote cap must be unforgeable — the receiving node HMACs the cap bytes with a shared/derived key and rejects bad ones. This is where `node_id > 0` caps become trustworthy. Design the byte layout as the HMAC input NOW (it's already stable). Probably a later sub-step (get bytes moving first, secure them second) — but flag it so the message format doesn't need reshaping.
6. **The `send_remote` → real path:** it currently returns `RemoteUnreachable`. The first Layer 6 win is: `send_remote(node_id, msg)` serializes `msg`, hands it to the NIC/net-service, and (for a local loopback test) it comes back in on the RX path, gets deserialized, and `send_local`'d to the destination endpoint on the receiving side. **First provable milestone: a message with `node_id=1` that actually reaches an endpoint via the wire instead of hitting the stub.** That's the seam finally closing.

**Sub-sequencing recommendation (keep the boot-between-each discipline):**
`L6a` virtio-mmio probe + init handshake (DRIVER_OK) → `L6b` one virtqueue set up → `L6c` TX a single hand-built frame, observe it on the QEMU backend → `L6d` RX a single frame via PLIC interrupt → `L6e` wire `send_remote` to TX an `IPCMessage`, RX path deserializes + `send_local`s it → `L6f` HMAC attestation. Each its own boot + tag.

---

## 🗣️ STYLE NOTES

- Call him "fren," casual, warm — but hold the systems rigor the `userPreferences` block asks for (real and current).
- Complete `cat > file << 'EOF'` blocks or guarded `python3` assert-then-replace patches; the *why* + the tradeoff behind each change. **End blocks with an `echo`/`grep` gate** so a dropped paste is caught instantly, not at the next confusing compile error.
- Work actual build/QEMU errors one at a time; read the real diagnostic. Most "disasters" this project were one-line fixes in scary costumes (a dropped `cat` paste, a config-rustflags join, a mis-calibrated timer test that printed zero ticks because the spinners finished inside one quantum).
- Decode faults from `code`/`stval`/`sepc`: code 12=instr page fault, 13=load page fault, 15=store page fault; interrupt code 5=timer, 9=supervisor external (PLIC — new at L6). `stval`=faulting addr, `sepc`=faulting instr.
- Commit + tag at every green. Two-disk drift is dead — keep it that way.
- **Watch the ledger.** `[L4] ... frames in use` returning to the pre-spawn baseline (3) after all processes exit is the leak check. It has held through every milestone; if it stops holding, teardown regressed.

---

*End of handover. Layers 0–5 stand complete, fren — the kernel preempts, isolates, reclaims, and runs compiled-Rust userspace programs that name services through a VFS and talk to each other over addressed, reply-routed capability endpoints, concurrently. The remote seam is built, demonstrated, and waiting. Layer 6 is where "distributed-native" stops being architecture and starts being packets on a wire. It's a real device driver — a proper multi-session layer — so open it fresh, sub-sequence it, and boot between each step like we did everything else. Go make node 1 answer.* 🐺
