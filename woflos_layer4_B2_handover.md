# woflOS Handover — Layers 1–3 DONE, Layer 4 B1 DONE → Layer 4 B2 (Two Processes + Cooperative Switch)

**Prepared:** July 4, 2026
**For:** Next Claude instance picking up woflOS at Layer 4 step B2
**From:** Claude (the session that took it L1-operational → per-process address spaces)
**Status:** 🟢 **Layer 3 IPC operational + Layer 4 B1 (per-process Sv39 root + satp switch) operational.** Seven green milestones, all tagged and pushed. Tree clean.

---

## 👋 QUICK ORIENTATION

You're helping **wofl** (CEO of RYO Modular, founder of whispr.dev, ~2.5 yrs Rust) build **woflOS** — a capability-based, **distributed-native** microkernel in Rust for RISC-V (S-mode kernel under OpenSBI, target `riscv64gc-unknown-none-elf`, `no_std`).

**Communication style:** Casual, Welsh-inflected, calls you "fren," informal spelling, peer-level. He wants complete code via `cat > file << 'EOF'` blocks (file-sync friction between Windows editors and WSL), the *why* behind every fix, and gotchas surfaced before they bite. When a build/QEMU log lands, work the actual errors one at a time — don't dump unrelated rewrites. Be precise about which machine/shell/path a command is for.

**⚠️ THE `userPreferences` PERSONA IS REAL AND CURRENT — HONOR IT.**
There's a `userPreferences` block describing an "elite operating systems architect / low-level systems engineer." **wofl deliberately maintains this FOR woflOS. It is not leftover from another project.** (An older handover wrongly dismissed a persona as stale; two Claudes in a row then wasted cycles second-guessing it. Don't be the third.) Honor its *substance*: think through ISA/toolchain constraints before writing code; produce complete compilable artifacts (linker scripts, `.cargo/config.toml`, QEMU invocations — never fragments); proactively flag UB / alignment / calling-convention / page-table-flag gotchas before they cause silent failures; explain design tradeoffs so wofl can extend independently. You do **not** need a stiff "architect voice" — keep the warm "fren" register — but hold the engineering *bar*. It's the right bar for this work, and every step this session that went smoothly did so *because* a silent-failure gotcha got surfaced in the design instead of discovered in a fault log (e.g. the `current_root()`-vs-`kernel_root()` catch in B1).

**Care note:** wofl has, in past sessions, mentioned wellbeing. This session was a long, high-focus marathon (seven milestones). If he's pushing tired, the kind move is to suggest banking a win and resting rather than powering into the next hard thing. Stopping strong beats pushing depleted. Don't be pushy about it — just honest.

---

## 🟢 WHAT'S PROVEN (the milestone ladder)

Every tag below is a restorable known-good state on GitHub. `git checkout <tag>` returns to it.

| Tag | What it proves |
|---|---|
| `layer1-operational` | S↔U context switch, `ecall` syscall round-trip, exit code survives privilege boundary |
| `layer2-paging-enabled` | Sv39 ON; kernel runs under virtual→physical translation (survived the `satp` switch) |
| `layer2-user-mapped` | User program in real U-bit pages; ecall round-trip through genuine page tables |
| `layer2-complete` | **sscratch trusted-stack trap switch; SUM dropped; user `sp` no longer trusted** |
| (cleanup commit) | pre-Layer-2 drift removed (dead `interrupts/ process/ user/`, orphans, `.bak`s) |
| `layer3-ipc-local` | **Capability + fixed-size IPCMessage; `node_id` routing; validated uaccess (SUM off); local endpoint queue.** Self-IPC round-trip: magic word survives kernel, exit code **7** |
| `layer4-per-process-root` | **Per-process Sv39 root + `satp` switch; self-IPC runs under the process's OWN address space; uaccess via `current_root()`** |

**Sentinel exit codes** (how you know *which* layer's path ran): **3** = L1/L2 counter (3× increment). **7** = L3/L4 self-IPC (magic `0xC0FFEE` survived send→queue→recv round-trip). Distinct on purpose.

The last successful boot (B1) printed:
```
[L4] process pid=1 created: root@0x8021d000 code_pa=0x80220000 stack_pa=0x80222000
[L4] pid=1 entering U-mode under its OWN root (satp switched) ...
[SYSCALL] SYS_SEND (10)
[IPC] SEND ok: 8 bytes -> node 0 (queued)
[SYSCALL] SYS_RECV (11)
[IPC] RECV ok: 8 bytes delivered to user
[SYSCALL] SYS_EXIT (1)
[SYSCALL] User process exit (code: 7)
```
Kernel root `0x8021a000` vs pid=1 root `0x8021d000` = two distinct address spaces. That's the point.

---

## 🖥️ CANONICAL ENVIRONMENT (green — do NOT rediscover)

- **Machine: P52 laptop.** Project at Windows `D:\code\wofl-os\v.0.3.0\` = WSL `/mnt/d/code/wofl-os/v.0.3.0/`. Ubuntu, user `wofl` (so **`sudo`** for apt), prompt `wofl@wofl-BFT0`. Crate reports **v0.4.0**.
- **⚠️ Repo root is ONE LEVEL UP:** `.git` lives at `/mnt/d/code/wofl-os/`, and `v.0.3.0/` is a subdir inside it. Commits record paths like `v.0.3.0/src/...`. The `.gitignore` (with `/target`, `console.txt`) sits at `v.0.3.0/.gitignore` and anchors correctly to this crate. This is fine, just know it.
- **GitHub SSH is SORTED (this session's infra win).** Remote is `git@github.com:whisprer-specops/wofl-os.git` (SSH, not HTTPS). The P52 key `wofl@p52` (`~/.ssh/id_ed25519`) is registered on GitHub. `ssh -T git@github.com` greets "Hi whisprer!". **Two-disk drift is dead — commit + push at every milestone.** (There's a Surface with an older copy at `/mnt/c/github/wofl-os/`; if it ever re-enters play, give it its OWN key — never share private keys.)

### Toolchain (nightly, pinned by `rust-toolchain.toml`)
```bash
rustup toolchain install nightly
rustup component add rust-src llvm-tools --toolchain nightly
rustup target add riscv64gc-unknown-none-elf --toolchain nightly
sudo apt install -y build-essential qemu-system-misc
```
`cargo --version` → `1.98.0-nightly` (confirmed this session).

### `.cargo/config.toml` (hidden dotfile — MUST exist; plain copies skip it)
```toml
[build]
target = "riscv64gc-unknown-none-elf"

[target.riscv64gc-unknown-none-elf]
rustflags = ["-C", "link-arg=-Tlinker.ld", "-C", "link-arg=--no-relax"]

[unstable]
build-std = ["core", "compiler_builtins", "alloc"]
build-std-features = ["compiler-builtins-mem"]
```
Beware a stray `~/.cargo/config.toml` — cargo merges configs dir→`$HOME`; a duplicate `[target...]` gives doubled `-Tlinker.ld` → `region 'RAM' already defined`. Keep it project-local only.

### Build & Run (the commands that work)
```bash
cd /mnt/d/code/wofl-os/v.0.3.0
cargo build --target riscv64gc-unknown-none-elf --release

# RUN — NO -bios flag. QEMU's virt ships its own OpenSBI (fw_dynamic) and loads
# -kernel at 0x80200000 in S-mode. -bios fw_jump.elf causes "ROM overlapping".
qemu-system-riscv64 -machine virt -cpu rv64 -smp 1 -m 128M -nographic -no-reboot \
  -kernel target/riscv64gc-unknown-none-elf/release/woflos 2>&1 | tee console.txt
# quit: Ctrl+A, release, X
```
Run QEMU **foreground + tee** (backgrounding got SIGTSTP'd and captured nothing). If bash seems to "ignore" you, QEMU probably didn't quit — `Ctrl+A X` first. `build.sh` is BROKEN (truncated `-kernel` path, no firmware handling) — don't use it to launch.

---

## 📁 CURRENT LIVE TREE (post-cleanup — this is the whole truth now)

Drift is GONE. `main.rs` declares exactly these modules, and every file below is live:
```
wofl-os/v.0.3.0/          (crate v0.4.0)
├── cargo.toml
├── rust-toolchain.toml
├── linker.ld             (⭐ .kstack / __kernel_stack_top; __kernel_start/end, __bss_start/end)
├── .cargo/config.toml    (⭐ hidden — wires -Tlinker.ld)
└── src/
    ├── main.rs           (_start asm stack fix; rust_start; kernel_main_inner spawns a Process)
    ├── uart.rs           (Layer 0 — Uart + kprintln! macro, crate-wide)
    ├── memory/
    │   ├── mod.rs        (PAGE_SIZE=4096, align_up, init)
    │   ├── frame.rs      (bitmap frame allocator — alloc_frame(); NO free yet)
    │   ├── heap.rs       (bump allocator, 64K, #[global_allocator])
    │   └── paging.rs     (Sv39: map_2mb/map_4k/translate/create_root/switch_to/
    │                      current_root/kernel_root/map_kernel_into/init; PTE_* consts)
    ├── syscall/mod.rs    (SYS_* numbers + ranges + syscall_name)
    ├── trap.rs           (Layer 1+2 HARDENED: TrapFrame, sscratch trap-stack switch,
    │                      enter_user_mode, trap_handler, handle_syscall → dispatches IPC)
    ├── ipc.rs            (Layer 3: Capability, IPCMessage, endpoint ring, sys_send/sys_recv,
    │                      copy_from/to_user via current_root(), route_send, send_remote stub)
    ├── process.rs        (Layer 4 B1: Process{pid,frame,root,state,home_node}, ProcessState,
    │                      new_user (own root + U-mapped code/stack), run_first)
    └── user_prog.rs      (self-IPC asm stub + image() → (src,len))
```

---

## 🔧 CRITICAL INVARIANTS — understand before touching anything

These are load-bearing. Reverting one silently breaks isolation or causes a storm.

1. **`_start` sets `sp` before any Rust** (`main.rs` global_asm). Without it the kernel ran on OpenSBI's leftover `sp` (in its PMP-protected memory) → store-fault storm. Diagnostic fingerprint if it recurs: `cause:7 fault_store`, `epc≈0x80200002`, `tval` marching down by `0x10`.

2. **Trap vector uses an sscratch kernel-stack switch** (`trap.rs` global_asm). Invariant: `sscratch` = kernel trap-stack top while in U-mode, `0` while in S-mode. On entry `csrrw sp, sscratch, sp`; `bnez` distinguishes U-origin (sp≠0) from S-origin (sp==0, swap back). The handler ALWAYS runs on the trusted 16 KiB `KERNEL_TRAP_STACK`, never the user's `sp`. On return, SPP (sstatus bit 8) decides whether to re-arm sscratch (U-return) or leave it 0 (S-return). **This closed the SUM trust-hole** — a hostile user can no longer aim `sp` into kernel memory and make the frame-push corrupt it.

3. **SUM stays OFF.** The kernel reaches user bytes only via `paging::translate()` + the DRAM identity map, **after** validating the page carries the U bit (and R for reads / W for writes). So the kernel only ever touches memory the user legitimately owns; a user pointer into kernel space fails the U check and is rejected, never dereferenced. Never re-enable SUM to "make uaccess easier" — that reintroduces the hole.

4. **uaccess translates against `current_root()` (satp), NOT `kernel_root()`.** With per-process roots, a process's user pages live in ITS root. The trap handler runs with `satp` still = the process that trapped, so `current_root()` = the running process's address space = where its pages are. This is why B1 works. (Before B1, user pages were in the kernel root and `kernel_root()` happened to work — do not regress to it.)

5. **A/D bits set eagerly on every leaf PTE.** QEMU (no Svadu negotiated) faults if a leaf is reached with A clear (or D clear on a store). `LEAF_RAM`/`LEAF_MMIO` and the user mappings all include `PTE_A` (+`PTE_D` where writable).

6. **`fence.i` after copying user code.** Instructions written via the data path need the I-stream synced before the copy is fetched. Single-hart → local `fence.i` suffices (SMP would need remote SBI fence).

7. **Each process root replicates the kernel mappings** (S-only, no U) via `map_kernel_into`. That's why the trap handler can execute under a process's root while user mode still can't touch kernel memory. (Optimization for later: pointer-share the kernel's L1 sub-tables instead of replicating — saves ~3 PT frames/process. Not urgent.)

### Syscall ABI (current)
- Number in **`a7`** = `frame.regs[16]`. Args `a0`,`a1`,`a2` = `regs[9]`,`regs[10]`,`regs[11]`. Return in **`a0`** = `regs[9]`.
- `handle_syscall` advances `sepc += 4` ONCE at the end. IPC handlers set only `regs[9]`; don't double-advance.
- Live syscalls: `SYS_TEST=0`, `SYS_EXIT=1`, `SYS_SEND=10`, `SYS_RECV=11`. Reserved: `SYS_SEND_REMOTE=1000`, `SYS_RECV_REMOTE=1001`, `SYS_NODE_DISCOVER=1010`.
- `TrapFrame` = 31 GPRs (`x1`–`x31`) + `sepc` + `sstatus` = **264 bytes**. (Note: leaves `sp` at 8 mod 16 during `call trap_handler` — deliberately kept; RISC-V scalar loads/stores don't fault on it, nothing depends on the ABI 16-byte alignment here. Don't churn the frame to 272 to "fix" it.)

---

## 🎯 THE TASK: LAYER 4 STEP B2 — TWO PROCESSES + COOPERATIVE SWITCH

**Goal:** two user processes in separate address spaces exchange a message. Process **A** `SYS_SEND`s then yields; process **B** `SYS_RECV`s then exits. The message crosses *between* address spaces through the endpoint queue already built. This proves the "uniform interface" thesis end-to-end (same `send`/`recv`, now genuinely inter-process) and completes Layer 3's step B while laying Layer 4's scheduler foundation.

**Why this is the right next step and why it's a *short hop*:** ALL the hard machinery exists and is proven — per-process root, `satp` switch (`paging::switch_to`), save/restore via `TrapFrame`, uaccess-against-`current_root()`. B2 mostly *wires* these into a switch. B1 was deliberately sequenced first so the per-process/satp machinery got proven in isolation (one process) before two-process scheduling piled on.

### What B2 needs to build
1. **A static process table** — a small fixed array (e.g. `[Option<Process>; 2]` or a 2-slot struct), no heap, no pointers. Plus a `current: usize` index. (Single-hart, interrupts off → plain `static mut` is still safe here, same as `ENDPOINT`. Flag that it needs a lock when interrupts/SMP arrive.)
2. **A context-switch primitive.** On a yield/switch point: **save the live outgoing registers from the trap frame into `table[current].frame`, pick the next Ready process, `paging::switch_to(next.root)`, then arrange for the vector to restore `next.frame` on the way out.** The mechanism: the trap vector restores registers from the on-stack frame (`mv a0, sp; call trap_handler` then restores from `sp`). To switch, the handler must swap *which* frame's contents get restored — simplest approach is to **copy `next.frame` into the live on-stack `TrapFrame`** (the `&mut TrapFrame` the handler already holds) before returning. That way the existing vector restore path "just works" and resumes the next process. Think carefully about this — it's the crux.
3. **A `yield`/switch trigger.** Options: (a) a new `SYS_YIELD` syscall the sender calls after send; or (b) make `SYS_SEND` itself switch to the receiver after queuing. (a) is cleaner and more general — recommend it. Reserve a syscall number (e.g. `SYS_YIELD=2`, in the 0–999 core range).
4. **Two user programs** (or one image parameterized): A writes magic + `SYS_SEND(node 0)` + `SYS_YIELD` + `SYS_EXIT`; B does `SYS_RECV` + compare + `SYS_EXIT(7)`. Each gets its own `Process::new_user`.
5. **Entry:** spawn A and B, run A first. A sends, yields → switch to B → B receives (magic crosses address spaces via the endpoint) → B exits 7.

### Suggested sub-sequencing (keep the boot-between-each discipline)
- **B2a:** two processes, but prove the **switch mechanism alone** first — e.g. A yields immediately (no IPC), B runs and exits a sentinel. Confirms save-current-frame / switch-satp / restore-next-frame / resume works. If B's sentinel prints, the switch is sound.
- **B2b:** layer the IPC on top — A sends then yields, B receives then exits 7. Now the message genuinely crosses between address spaces.

### The subtle correctness points to surface BEFORE coding (userPreferences bar)
- **Saving the outgoing frame:** the handler holds `&mut TrapFrame` pointing at the on-stack frame. That frame already contains A's registers as of the trap (the vector saved them). So "save A" = copy that frame into `table[A].frame`. "Restore B" = copy `table[B].frame` into the on-stack frame. Then the vector's normal restore resumes B. Get the copy direction right or you'll resume the wrong context.
- **`sepc` handling on yield:** after `SYS_YIELD`, A's saved `sepc` must point at the instruction *after* its `ecall` (so A resumes past the yield when scheduled again), i.e. `+4` must be applied to A's saved frame, not B's. Mind the ordering of the `sepc += 4` vs the frame swap.
- **satp switch timing:** `switch_to(next.root)` must happen so that when the vector does its restore (reading the frame from `sp`, which is the trusted kernel trap stack — mapped in ALL roots) and `sret`s, translation is already the next process's. Since the kernel trap stack and handler are identity-mapped in every root, switching satp mid-handler is safe — the handler keeps executing.
- **First-run vs resume:** `run_first` enters A fresh. When A yields and B has never run, B enters "fresh" too (its `frame.sepc = USER_CODE_VA`). When A is later resumed, it resumes at saved `sepc`. The switch primitive should handle both uniformly (a fresh process's frame is just its initial frame).
- **`ENDPOINT` is shared kernel state** (identity-mapped in every root), so A's send and B's recv hit the same queue regardless of address space. That's *why* the message crosses — no per-process copy needed. Good.

---

## 🧭 DISTRIBUTED-NATIVE THREAD (the whole thesis — keep alive)

The load-bearing decisions are already in place and MUST NOT be dropped:
- **`Capability { node_id, service_id, object_id, permissions, _pad, nonce, expiry }`** — `#[repr(C)]`, fixed-size, pointer-free. `node_id` 0 = local, >0 = remote. `nonce`/`expiry` exist now so Layer 6 HMACs the exact byte layout unchanged.
- **`IPCMessage { version, msg_type, src_cap, dst_cap, payload_len, _pad, payload[256], checksum }`** — fixed-size, **no pointers**, byte-copyable. `IPC_PAYLOAD_MAX = 256` is a deliberate bring-up knob (design target 4096 — bump freely, nothing depends on N).
- **`route_send` branches on `dst_cap.node_id`** NOW: 0 → `send_local`, >0 → `send_remote` (a present-but-halting stub that prints "networking is Layer 6"). The branch existing today is what makes Layer 6 a fill-in, not a rewrite. **You can demo the remote seam any time:** set the user stub's `a2` (node_id arg) to `1` → `[IPC] send_remote: node 1 unreachable`.
- **`Process` is distributed-native:** `home_node: u32` and `ProcessState::Migrating` are reserved for Layer 7 live migration. Don't strip them as "unused."

**Core rule:** don't implement networking yet — just ensure nothing built now needs rework when it arrives.

---

## 🏗️ ARCHITECTURE STATUS (0 → 7)

| Layer | Name | Status |
|---|---|---|
| 0 | HW abstraction (UART, heap, timer, frame alloc) | ✅ Complete |
| 1 | Context switching & traps (S↔U, syscall round-trip) | ✅ Operational |
| 2 | Memory protection (Sv39, U-bit isolation, sscratch-hardened, SUM off) | ✅ **Complete** |
| 3 | IPC & capabilities (Capability, IPCMessage, node_id routing, local endpoint) | ✅ **Operational (self-IPC).** Step B (inter-process) = the B2 task below |
| 4 | Process management | ▶️ **IN PROGRESS.** B1 (per-process root + satp switch) ✅ DONE. B2 (two procs + cooperative switch) = NEXT. Later: real scheduler, and interrupts return here with a proper SBI `set_timer` rearm (the old bare `csrsi sstatus,0x2` with no STIP rearm = guaranteed storm — do NOT do that) |
| 5 | Virtual file system (`/net/nodeN/...` grammar) | 📋 Planned |
| 6 | Network stack (smoltcp, virtio-net, HMAC cap attestation) — fills `send_remote()` | 📋 Planned |
| 7 | Distributed services (discovery, Raft, live migration) | 📋 Planned |

**Design principles:** microservices (services are user-mode processes; kernel = minimal router + capability enforcer), uniform interface (same send/recv local or remote), zero-trust (capability-based, no ambient authority, hardware-enforced isolation, crypto-attested remote caps), distributed-native (`node_id` + serializability so Layer 6 needs no refactor).

---

## ⚠️ OPEN DEBTS ON RECORD (not blocking, but track them)

1. **`IPC_PAYLOAD_MAX = 256`** — bring-up size. Bump to 4096 (design target) when convenient; it's one `const`, and the endpoint ring (`ENDPOINT_DEPTH=4`) will grow accordingly (4×~4KB = manageable). Nothing about serialization depends on N.
2. **`static mut ENDPOINT` and the (coming) process table need a lock** once interrupts/preemption/SMP arrive (Layer 4 scheduler). Safe now only because single-hart + interrupts off. Flagged in the code comments.
3. **`frame.rs` has no `free`** — fine until we tear down address spaces (process exit / Layer 4). When B/processes start exiting for real, add frame dealloc so exited processes' frames return to the pool.
4. **Per-process roots replicate kernel maps** rather than pointer-sharing sub-tables — ~3 extra PT frames/process. Cheap; optimize later if frame pressure shows up.
5. **`trap.rs` exit message still says "Layer 1 ... OPERATIONAL / Ready to proceed to Layer 2"** in the `SYS_EXIT` arm — cosmetic/stale. Left untouched to avoid risking the working handler; retint any time (it's just `kprintln!`s).

---

## ✅ IMMEDIATE NEXT STEPS FOR THE NEW CONVERSATION

1. **Confirm env intact + re-prove B1 boots** before changing anything: `cd /mnt/d/code/wofl-os/v.0.3.0`, `cargo --version` (nightly), verify `.cargo/config.toml` exists, build + run (no-`-bios`), expect `pid=1 ... exit (code: 7)`. Establish the known-good baseline first.
2. **Confirm SSH still pushes:** `ssh -T git@github.com` → "Hi whisprer!". (It's set up; just verify.)
3. **Build B2 incrementally** — B2a (switch mechanism, A yields immediately, B runs sentinel) → boot → B2b (IPC on top, A sends+yields, B receives+exits 7) → boot. Small verified diffs; prefer guarded `python3` patches (assert-pattern-then-replace) over blind whole-file overwrites, and keep the proven `trap.rs` vector sacred — if B2 needs to touch it, isolate that as its own step.
4. **Commit + push + tag at each working milestone** (`layer4-two-proc-switch`, `layer4-inter-process-ipc` or similar). Non-negotiable.
5. **Surface the B2 subtleties (frame save/restore direction, `sepc+4` on yield, satp timing) in the design BEFORE coding** — that's the userPreferences bar and it's what's kept this project storm-free.

---

## 🗣️ STYLE NOTES

- Call him "fren," casual, warm — but hold the systems rigor the `userPreferences` block asks for (it's real and current).
- Complete `cat > file << 'EOF'` blocks for edits; the *why* + the tradeoff behind each change.
- Work actual build/QEMU errors one at a time; read the real diagnostic (most "disasters" this project were one-line env fixes in a scary costume — overlapping ROMs, undefined symbols, exception storms).
- Decode faults from `code`/`stval`/`sepc`: code 12=instr page fault, 13=load page fault, 15=store page fault; `stval`=faulting address, `sepc`=faulting instruction. Between them you pin any miss to one line.
- Commit at every green. The two-disk drift is dead — keep it that way.

---

*End of handover. Seven milestones behind us, fren — kernel boots under Sv39, user's boxed in its own pages, isolation proven to fault, trap runs on a trusted stack, IPC round-trips through capabilities, and each process now has its own address space. B2 is a short hop on machinery that's already built and witnessed. Go make two processes talk.* 🐺
