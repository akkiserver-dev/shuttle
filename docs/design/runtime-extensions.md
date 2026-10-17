# Design: runtime extensions for the Weave binding

Status: **design, not implemented** (2026-10-15). API names marked *(proposed)* need
the owner's approval before implementation; names marked *(approved)* were approved
on 2026-10-17.

## Scope

Shuttle provides a Weave-integrated, persistable Wasmtime runtime and its Java
binding, in the same role that JNLua plays for OpenComputers. Everything at the level
of "a computer" (firmware ABI, devices, ticks, power, BIOS/OS boot, the SHTL save
format) belongs to the host mod; the prototype in `native/shuttle-core` is going to
move to the Retro Computing mod.

Two features of that prototype are generic runtime features and belong in Shuttle's
Java binding (`shuttle-java`, package `jp.akkiserver.shuttle`):

1. **Weave extensions**: yielding imports, unwinding from inside a host call, and
   per-instance memory limits.
2. **Nested instances**: the primitives that a host needs to run one guest from
   inside another guest's host call, with a shared execution budget (what
   `native/shuttle-core` uses for its `co_*` imports).

The existing Weave layer (`WeaveModule`, `WeaveInstance`, `WeaveCaller`,
`shuttle-java/src/main/rust/wasmtime-binding/src/weave.rs`) is extended; nothing is
removed.

## 1. Weave version

`shuttle-java` pins Weave `0f26893`, which lacks all three extensions. They exist in
`677cce6` (branch `shuttle-host-hooks` of `akkiserver-dev/weave`), which `native/`
already uses:

| Weave change | API |
| --- | --- |
| `0286544` yielding imports, per-instance memory limits | `TransformOptions::yielding_imports`, `weave_wasmtime::request_unwind`, `WeaveInstance::set_memory_limit`, `Ctx::set_memory_limit` |
| `677cce6` change the limit from inside a host call | `Ctx::set_memory_limit` usable in host functions |

The Weave-side names in the table are kept as they are (approved). `Cargo.toml` pins
Weave by commit (`rev`), so the branch name `shuttle-host-hooks` does not matter to
Shuttle and stays.

Changes:

- `wasmtime-binding/Cargo.toml`: all four Weave crates to rev
  `677cce64cf9728485849af21f7bf36f801175d4f`; wasmtime to 46.0.3 (same as `native/`).
- `weave.rs` (`module_create`): the `TransformOptions` literal gains
  `..Default::default()`, otherwise it stops compiling once the new field exists.

## 2. Yielding imports

**Problem.** `WeaveCaller.requestUnwind()` sets the poller to `UnwindNext`: the host
call returns normally, and the guest stops at its next instrumented poll. A host
function cannot stop the guest *at the call* and produce its result later (for
example, a disk read that completes on the server thread).

**Weave's mechanism.** A module transformed with
`TransformOptions::yielding_imports = [(module, name), ...]` treats calls to those
imports as unwind points. Inside such a host call,
`weave_wasmtime::request_unwind(caller)` makes the guest unwind right at the call
site. On `resume`, the guest calls the **same import again** with the same
arguments, and that second call's return value is used.

**Java API** *(approved)*:

```java
WeaveModule module = WeaveModule.load(engine, wasm,
        WeaveModule.Options.builder()
                .pollPeriod(64)
                .stackPages(4)
                .yieldingImport("dev", "read")      // may be repeated
                .build());

builder.define("dev", "read", "ii>i", (caller, args) -> {
    Result r = pending.get(key(args));
    if (r == null) {                    // first call: start the work, stop the guest
        submitToServerThread(args);
        caller.suspend();
        return new long[] {0};          // ignored: the guest unwinds
    }
    return new long[] {r.value()};      // re-call after resume: the real result
});
```

- `WeaveModule.Options` (builder) with `pollPeriod`, `stackPages`, `yieldingImport(module,
  name)`. The existing `load(engine, wasm)` and `load(engine, wasm, pollPeriod,
  stackPages)` stay and map to it.
- `WeaveCaller.suspend()` calls `request_unwind`. `call`/`resume` then return
  `isUnwound()`.
- Calling `suspend()` from an import that is not declared as yielding throws. Weave
  does not check this itself: `request_unwind` only refuses calls during module
  initialization, and otherwise just sets the unwind state. The binding therefore
  records, per defined host function, whether its import is in the module's
  yielding list, and `suspend()` checks that flag.
- Because the import is called again after `resume`, and possibly after a
  `snapshot` + `restore` in a later session, the host keeps the request state on its
  side and makes the second call idempotent.
- `requestUnwind()` keeps its current meaning (stop at the next poll).

## 3. Memory limits

Without a limit, a guest can grow its memory until the host runs out.

**Java API** *(approved)*:

| Method | Maps to | Notes |
| --- | --- | --- |
| `WeaveInstance.setMemoryLimit(long bytes)` | `WeaveInstance::set_memory_limit(Some(n))`, or `None` for a negative value | any negative value (conventionally `-1`) means unlimited; there is no separate `clear` method |
| `WeaveInstance.memorySize()` | `mem_view().size(0)` | current size of memory 0, for accounting |
| `WeaveCaller.setMemoryLimit(long bytes)` | `Ctx::set_memory_limit` (same rule) | inside a host call, e.g. to shrink a parent's limit after creating a child |

- A `memory.grow` past the limit returns -1 to the guest; it does not trap.
- The limit is not part of a Weave snapshot. After `restore`, the host sets it again.

## 4. Nested instances

`native/shuttle-core` runs a child program from inside the kernel's `co_resume` host
call (`native/crates/shuttle-core/src/imports.rs`, `instantiate_child` and
`co_resume`). The runtime part of that is small:

1. **Re-entrancy.** A host function may `call` or `resume` another `WeaveInstance`
   on the same thread. Each instance has its own Store, so this already works in
   Rust. The Java binding has to allow it (no global lock, the outer `WeaveCaller`
   stays valid) and a test has to cover it.
2. **Shared execution budget.** The child's polls come out of the parent's
   remaining budget. The host needs:
   - `WeaveCaller.pollBudget()` / `WeaveCaller.setPollBudget(long)` *(approved)*: the
     parent's `Poller::UnwindAfter(n)`, read and written inside the host call.
   - `WeaveInstance.pollBudget()` *(approved)*: after `resume`, the child's leftover
     budget (`take_poller`), so the host can compute how many polls it used.
   - The existing `unwindAfter(n)` keeps its name.

   The host gives the child `min(requested, parent remaining)` with
   `unwindAfter(...)`, resumes it, and subtracts the polls the child used from the
   parent:

   ```java
   long remaining = caller.pollBudget();
   long given = Math.min(requested, remaining);
   child.unwindAfter(given);
   WeaveResult r = child.resume();
   long used = given - child.pollBudget();
   caller.setPollBudget(remaining - used);
   ```

3. **Snapshots** stay per instance: `snapshot()` on the parent and on each child, plus
   `WeaveModule.hash()` to identify the module. Putting them into one save file is
   the host's job.

Not part of Shuttle: the `co_*` import names and their ABI, child handles, the
call/yield record format, and the policy for how much memory a kernel may use.

## Out of scope for now

- Worker threads and stack sizes (`max_wasm_stack`), epoch-based time limits.
- A versioned envelope around snapshots (today the bytes are gzip of Weave's encoding).
- Moving `native/` to the Retro Computing mod repository.

## Tests for the implementation

Extend `shuttle-java/src/test/java/jp/akkiserver/shuttle/WeaveTest.java`:

- Yielding import: `suspend()` stops the guest at the call; after `resume()` the
  import is called again and its value is used; the same across `snapshot()` +
  `restore()`; `suspend()` from a non-yielding import throws.
- Memory limit: `memory.grow` past the limit returns -1 and the guest keeps running;
  the limit set from inside a host call takes effect; `memorySize()` matches.
- Nested instances: a host function resumes a child; the child's polls are taken
  from the parent's budget; parent and child snapshots restore and continue.

Run `mvn test` on Windows and on WSL2.
