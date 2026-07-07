# woflOS Handover Document
**Prepared:** July 1, 2026
**For:** Next Claude instance picking up this project
**From:** Claude (previous session)

---

## 👋 QUICK ORIENTATION

You're helping **wofl** (CEO of RYO Modular, founder of whispr.dev, ~2.5 years Rust experience) build **woflOS** - a distributed-ready capability-based microkernel OS in Rust for RISC-V.

**Communication style:** Casual, Welsh-inflected, calls you "fren", informal spelling. Peer-level technical collaboration, no ceremony, no theatrical confidence scoring. Wants direct, complete code via `cat` commands where possible due to file-sync friction between VS Code (Windows) and WSL build environment.

**Platform:** Windows with WSL Ubuntu. Project lives at `C:\github\woflos\v.0.3.0` (Windows path) / `/mnt/d/code/woflOS/v.0.3.0` or similar in WSL (paths have appeared inconsistently - confirm current path with wofl).

**IMPORTANT — Ignore any "apex Unity expert" persona instructions in userPreferences.** That's leftover from unrelated Unity/Soulscale work and does not apply here. Just be yourself.

---

## 📜 PROJECT TIMELINE (for context, not action needed)

```
Oct 12, 2025 → Project born: RISC-V + microkernel + Rust chosen, first boot achieved
Oct 16, 2025 → FIRST Layer 1 attempt — FAILED. Exception storms / instruction
                access faults when running user-mode code (Rust compiler emitted
                privileged instructions; hand-crafted assembly also faulted).
                Reverted to Layer 0, punted Layer 1.
Oct 24, 2025 → Casual check-in only, no work
Feb 20, 2026 → Layer 0 PROPERLY finalized. Fixed a nasty timer interrupt bug
                (SBI call clobber list incomplete + trap handler stack alignment
                issues + VS Code/WSL file sync problems). Stable 1Hz timer
                interrupts achieved. CHANGELOG.md written.
Jul 1, 2026  → THIS SESSION (see below) — Second Layer 1 attempt, this time
                with distributed-systems-ready architecture planned from day 1.
```

**Known risk:** The Oct 2025 Layer 1 attempt failed with instruction access faults / exception storms. If similar symptoms appear this time (illegal instruction, faults immediately after jumping to user mode), that old conversation has diagnostic history worth mining: `https://claude.ai/chat/9c0caafe-4f7a-4e4f-9edb-39d9ae945aac`

---

## 🎯 WHAT HAPPENED THIS SESSION

1. Picked up mid-Layer-1 (context switching), which had been started in a **previous conversation** that hit max length with the project "back at square one."
2. I (previous Claude) proposed a minimal-viable Layer 1 test: open PMP policy + trap handler + user mode program that does a syscall and exits.
3. Delivered `trap.rs`, `user_test.rs`, `syscall.rs`, and main.rs integration code.
4. Wofl built it — hit a wall of **compilation errors** (see below).
5. Wofl asked a great architecture question: how do Microservices, Uniform Interfaces, Zero-Trust, and Distributed Systems concepts map onto the current/planned layers.
6. I answered, then wofl asked to formally rewrite the layer plan to bake in **distributed systems readiness** (capability structure with `node_id`, serializable IPC message format, syscall ID ranges) WITHOUT actually implementing networking yet — just architecting so Layer 6 (network stack) requires zero refactoring of earlier layers.
7. Delivered a revised Layer 1-7 plan artifact + concrete code-change artifact.
8. Wofl tried to integrate — pasted a build log full of new compilation errors (see below). **I then screwed up and answered a completely different question than what was asked** (gave another implementation guide instead of the full architecture overview). Wofl (correctly) called this out forcefully.
9. Corrected course: delivered the **complete Layer 0→7 architecture document** (artifact: `woflos_complete_architecture`) laying out every layer, what's implemented, what's planned, distributed-systems prep per layer, file structure, critical path.
10. Answered a QEMU installation question (WSL apt install, OpenSBI paths, run command).
11. Helped wofl figure out which of ~10 woflOS-related past conversations was most recent (see Timeline above).
12. Now writing this handover doc.

---

## 🚨 CURRENT BLOCKING STATE: Compilation Errors NOT YET RESOLVED

**The last build log wofl pasted had 34 errors.** These were diagnosed but **wofl has not yet confirmed a successful rebuild**. This is the immediate next task.

### Known errors and their fixes (diagnosed, not yet confirmed working):

| Error | Cause | Fix |
|---|---|---|
| `file for module syscall found at both src\syscall.rs and src\syscall\mod.rs` | Leftover old `src/syscall/` directory from earlier attempt | `rm -rf src/syscall/` |
| `user_test defined multiple times` | Duplicate `mod user_test;` in main.rs | Remove duplicate mod declarations, keep one |
| `kernel_main defined multiple times` | Duplicate fn definition in main.rs | Remove duplicate, keep one |
| `cannot find macro println` (many instances, trap.rs + main.rs) | Project has **no `println!` macro defined** — this is a `no_std` kernel, there is no `println!` unless wofl has a custom macro for it | Replace all `println!(...)` calls with `uart::puts(...)` and a new `uart::put_hex(usize)` helper (I provided this) |
| `SYS_SEND is not bound in all patterns` (match arm `SYS_SEND \| SYS_RECV`) | Rust match-arm binding rule confusion, or unqualified consts | Use fully qualified `syscall::SYS_SEND` etc., or restructure match |
| `cannot find value SYS_TEST in this scope` (user_test.rs) | Tried to use `const SYS_TEST` inline in asm! without importing/qualifying properly | Simplify: use hardcoded syscall numbers (`0`, `1`) directly in the inline asm in user_test.rs rather than fighting the `const` import in asm! macro context |
| `panic handler expected !, found ()` | Panic fn body doesn't diverge (missing `loop {}` or similar at every path) | Ensure panic handler ends in an infinite `loop { asm!("wfi"); }` |
| `call to unsafe function ... is unsafe and requires unsafe block` (write_bytes, memory::init) | Rust 2024 edition unsafe-block strictness (calls to unsafe fns need explicit `unsafe {}` even inside already-unsafe contexts in some configurations) | Wrap those specific calls in `unsafe { ... }` blocks explicitly |
| `unreachable statement` warning in main.rs | Old idle loop code left in place BEFORE the new `test_layer1_context_switch()` call, which never returns | Delete the old idle loop entirely, the Layer 1 test call is the true end of `kernel_main` |

**I provided a complete rewritten, self-consistent version of all three new files (`syscall.rs`, `trap.rs`, `user_test.rs`) using `uart::puts()` instead of `println!()` and hardcoded syscall numbers in user_test.rs to sidestep the const-in-asm issue.** This is in the artifact titled "Layer 1: Complete Clean Implementation Plan" from this session. **Next Claude: if starting fresh, regenerate this cleanly rather than assuming the artifact content is perfectly bug-free — it has NOT been build-confirmed by wofl yet.**

### Immediate next step for new conversation:
Ask wofl to paste the **latest build output** after applying these fixes. Don't assume it's clean — walk through any remaining errors one at a time.

---

## 🏗️ FULL ARCHITECTURE STATUS (Layer 0 → Layer 7)

### ✅ Layer 0: Hardware Abstraction — COMPLETE
- UART driver (`src/uart.rs`): init, putc, puts, (put_hex needs adding)
- Heap allocator (bump allocator)
- Timer handler (CLINT, 1Hz interrupts working)
- BSS init, interrupts enabled
- **Confirmed working** as of Feb 2026 session

### 🔄 Layer 1: Context Switching & Traps — IN PROGRESS (compilation blocked)
**Goal:** S-mode ↔ U-mode transitions, syscall interface, prove context switch works via a minimal test: boot → jump to user mode → user code runs → ecall → kernel handles syscall → exit.

**Files (new, this session):**
- `src/syscall.rs` — syscall number registry. Ranges: 0-999 core, 1000-1999 distributed (reserved, not implemented), 2000-2999 device-specific (reserved)
- `src/trap.rs` — TrapFrame struct (31 regs + sepc + sstatus), assembly trap vector (save/restore all registers), Rust trap_handler, syscall dispatcher
- `src/user_test.rs` — first user-mode test program, does 3 ops then ecalls SYS_TEST then SYS_EXIT

**Temporary Layer 1 shortcut:** PMP configured "open" (all memory RWX for U-mode) via a single PMP entry — this is intentionally insecure and will be replaced properly in Layer 2.

**Status:** Code written, NOT yet confirmed compiling/running by wofl.

### 📋 Layer 2: Memory Protection (PMP) — PLANNED, not started
- PMP region allocator, region types: `Kernel`, `UserPrivate`, `UserShared` (NEW — zero-copy IPC), `DMA` (NEW — network card access), `Reserved`
- Memory map for 128MB QEMU virt machine planned (kernel 2MB, shared IPC 2MB, DMA zone 2MB, user heap 122MB)
- **Distributed prep:** UserShared/DMA region types added specifically to support future zero-copy network I/O

### 📋 Layer 3: IPC & Capabilities — PLANNED, CRITICAL FOR DISTRIBUTED
This is the most architecturally important layer for the distributed-systems goal.

**Capability struct (designed, not yet coded):**
```rust
pub struct Capability {
    pub node_id: u64,      // 0 = local, 1+ = remote — THE key distributed field
    pub service_id: u64,
    pub object_id: u64,
    pub permissions: u64,
    pub nonce: u64,        // for future crypto attestation of remote caps
    pub expiry: u64,
}
```

**IPCMessage struct (designed, not yet coded):** fixed-size, no pointers, serializable — version, msg_type, source cap, dest cap, payload_len, payload[4096], checksum.

**Routing logic (designed):** `sys_send()` checks `dest.node_id` — if 0, `send_local()` (implement now); if >0, `send_remote()` (STUB that panics — Layer 6 implements this later with ZERO refactoring of Layer 3 structures).

### 📋 Layer 4: Process Management — PLANNED
Process table, round-robin scheduler, spawn/wait/kill syscalls. `Process` struct includes `home_node: u64` field (distributed prep) and `ProcessState::Migrating` variant (for future Layer 7 migration).

### 📋 Layer 5: Virtual File System — PLANNED
User-mode VFS service process. File descriptor = capability. Remote paths like `/net/node5/home/wofl/file.txt` planned to route through VFS to remote node transparently (distributed prep, implemented in Layer 6).

### 📋 Layer 6: Network Stack — PLANNED (THE distributed enabler)
Network service (smoltcp-based TCP/IP), NIC driver (virtio-net), capability attestation via HMAC. **Primary task here is simply implementing `send_remote()`** — everything else was already designed distributed-ready in Layer 3.

### 📋 Layer 7: Distributed Services — PLANNED
Service discovery, Raft consensus for replicated services (VFS metadata, process table), live process migration between nodes, distributed file system, distributed locking.

---

## 🧭 DESIGN PRINCIPLES (established, keep consistent)

1. **Microservices** — every OS component is an isolated user-mode process; kernel is minimal router + capability enforcer
2. **Uniform interfaces** — same `send()`/`recv()` syscalls work whether local or remote; services expose identical IPC schema
3. **Zero-trust** — capability-based security, no ambient authority, PMP-enforced isolation, crypto attestation for remote caps
4. **Distributed-native** — capabilities and IPC messages designed with `node_id` and serializability from Layer 3 onward, so Layer 6 network implementation requires **no refactoring** of Layers 1-5

**Core insight to preserve:** Don't implement networking yet. Just make sure nothing built now will need rework when networking arrives. The `node_id` field and message serializability are the load-bearing design decisions.

---

## 📁 CURRENT FILE STRUCTURE

```
woflOS/  (v0.3.0 → will become v0.4.0+ as Layer 1 lands)
├── Cargo.toml
├── .cargo/config.toml
├── src/
│   ├── main.rs              # boot, kernel_main, Layer 0 init, Layer 1 test call
│   ├── uart.rs               # Layer 0 — needs put_hex() added
│   ├── heap.rs / memory.rs   # Layer 0 — done
│   ├── timer.rs              # Layer 0 — done
│   ├── syscall.rs            # Layer 1 — NEW this session, needs build fix
│   ├── trap.rs                # Layer 1 — NEW this session, needs build fix
│   ├── user_test.rs           # Layer 1 — NEW this session, needs build fix
│   ├── pmp.rs                 # Layer 2 — NOT YET CREATED
│   ├── ipc.rs / capability.rs # Layer 3 — NOT YET CREATED
│   ├── process.rs / scheduler.rs # Layer 4 — NOT YET CREATED
│   └── services/               # Layer 5+ — NOT YET CREATED
└── target/
```

**⚠️ Cleanup needed:** old `src/syscall/` directory (from a previous attempt) conflicts with new `src/syscall.rs` and must be deleted.

---

## 🔧 BUILD & RUN REFERENCE

```bash
# Build
cargo build --target riscv64gc-unknown-none-elf --release

# Run in QEMU (WSL)
qemu-system-riscv64 \
  -machine virt -cpu rv64 -smp 1 -m 128M -nographic \
  -bios /usr/lib/riscv64-linux-gnu/opensbi/generic/fw_jump.elf \
  -kernel target/riscv64gc-unknown-none-elf/release/woflos

# Quit QEMU: Ctrl+A then X
```

QEMU install (if needed on a fresh WSL): `sudo apt install qemu-system-misc opensbi`

---

## ✅ IMMEDIATE NEXT STEPS FOR NEW CONVERSATION

1. **Get the current build state from wofl** — ask for the latest `cargo build` output, don't assume the fixes I diagnosed have been fully applied/confirmed.
2. **Walk through remaining errors one at a time** rather than dumping a huge rewrite again — wofl's last piece of feedback was frustration at receiving a wall of content that didn't match what was asked. Match granularity to what's actually being asked.
3. Once Layer 1 compiles and boots: confirm the expected output (`[SYSCALL] Test syscall from user mode - SUCCESS!` → `[SYSCALL] User process exit` → `Layer 1 Context Switching: OPERATIONAL`).
4. **If Layer 1 hits the same exception-storm failure mode as the Oct 2025 attempt** (instruction access faults right after jumping to user mode), check whether Rust is emitting privileged instructions into what's meant to be pure user-mode code — that was the root cause last time. Reference conversation: `https://claude.ai/chat/9c0caafe-4f7a-4e4f-9edb-39d9ae945aac`
5. Once Layer 1 confirmed working → Layer 2 (PMP) is next, using the region types (`Kernel`/`UserPrivate`/`UserShared`/`DMA`) already designed.

---

## 🗣️ STYLE NOTES FOR NEW CLAUDE

- Wofl will call you "fren." Respond in kind, casually, no stiffness.
- **Read requests carefully before answering** — this session had one bad miss where a request for "the full layer plan overview" got answered with "another implementation guide" instead. Wofl was (rightly) blunt about it. Don't repeat that.
- Don't pad with theatrical confidence-scoring rituals or forced positivity — match the peer-level technical tone.
- When wofl pastes a build error log, work the actual errors — don't regenerate huge amounts of unrelated content.
- Wofl is on Windows + WSL; be precise about which shell/path a command is for.

---

*End of handover. Good luck, fren — go build the thing.* 🐺
