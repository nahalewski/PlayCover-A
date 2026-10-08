# Experimental ARM64 runtime notes

## Current status

The local PlayCover-A fork runs freestanding ARM64 Mach-O executables and
small independently linked dylib fixtures. It also maps the acquired original
iOS ARM64 shared cache and decodes four slide-info-v2 regions. Original-cache
mapping and decoding have been tested on desktop and the first-generation
Pixel Fold. **Delta execution is not supported yet.** Mapping Apple libraries
and finding their symbols do not initialize those libraries or provide their
operating-system services.

The `a64` feature is enabled by default in this fork. Normal IPA and `.app`
launches route ARM64-only executables to the experimental runtime. Universal
apps containing ARM32 continue to use the existing ARM32 runtime. The ARM64
implementation remains separate from touchHLE's `Mem`, `Cpu`, Objective-C
runtime and framework implementations.

Local research fork only. touchHLE's CONTRIBUTING forbids AI-generated code
upstream; these changes are not intended for submission.

## ARM64 execution and independent dylibs

The Dynarmic A64 frontend and its separate wrapper support 64-bit guest
addresses, scalar and SIMD registers, PC/SP, thread pointers and full context
save/restore. Guest memory consists of separate regions with read/write/execute
permissions. Host loader writes are bounded and can cross adjacent regions;
unmapped holes remain inaccessible. Code writes invalidate the JIT cache.

Standalone and linked-image loaders map each Mach-O segment independently,
skip `__PAGEZERO`, and create an RX return trampoline plus an RW stack. The
ordinary image allocation limit is **256 MiB of actual mapped bytes**, rather
than the distance between the lowest and highest addresses. The sparse fixture
places data at `0x3_0000_0000` and text at `0x1_0000_0000`: its 8 GiB gap consumes
no allocation, and its freestanding test program returns 42. Overlaps,
overflowing ranges and invalid permissions are rejected.

The experimental linker supports:

- ARM64 `MH_EXECUTE` and independently loadable `MH_DYLIB` images, including
  ARM64 slices of universal binaries.
- Recursive dependencies, install identities, `@executable_path`,
  `@loader_path` and inherited `@rpath` search paths, with at most 128 images.
- Export tries, symbol-table fallback and bounded explicit/inherited dylib
  reexports. Resolver and TLS exports remain unsupported.
- Chained import formats 1/2/3, ordinary pointer formats 2/6
  (`DYLD_CHAINED_PTR_64` and `DYLD_CHAINED_PTR_64_OFFSET`), rebasing and binding,
  and positive dylib slides. Required missing symbols fail explicitly;
  legitimate missing weak references may resolve to zero.
- Classic dyld pointer rebasing and ordinary/lazy binding streams for older
  ARM64 images. Decoding is bounded, validates pointer slots and imported
  symbols, and stages writes before committing them. Lazy slots are resolved
  eagerly. Weak lookups accept a unique resolved address (including repeated
  exports of that same address) and return zero when absent. Conflicting
  definitions require unsupported coalescing and fail explicitly; strong-
  definition announcements remain unsupported. This is not full weak-symbol
  interposition support.
- Dependency-first initializer execution before main. Both 8-byte function
  pointer arrays and LLVM's 4-byte image-relative initializer offsets are
  supported. Targets must be mapped executable code. Initializers share a
  bounded instruction budget, and CPU context is restored after each call.
  Initializer dependency cycles are rejected.

The ordinary import fixture calls `_answer` in a relocated dylib and returns
42. The initializer fixture sets a global to 21, doubles it in a second
initializer and then returns 42. Tests also cover missing dependencies,
unresolved required symbols, weak references, malformed initializer tables,
invalid targets and initializer timeouts.

The new legacy fixtures use LLVM 18 with `-no_fixup_chains`. Their provider
contains a data pointer that must be rebased when the dylib slides. One client
uses a GOT import; the other uses a stub/lazy-pointer call. Both are designed
to return 42. The provider's `dyld_stub_binder` sentinel returns 99, so a result
of 42 also checks that eager lazy-slot binding bypasses the binder helper.
File and IPA launch tests cover both variants and a missing required symbol.
The generated GOT and lazy-call fixtures both returned 42 through file and
IPA launches on desktop. Integrated verification passed nine CPU tests and
61 loader/runtime tests. The Android build and device verification for
this legacy stage also passed on the Pixel 9 Pro Fold (`comet`): both generated
IPA fixtures returned 42 using the newly installed test APK. Device logs are
preserved in `pixel-fold-tests/9pro-legacy-bind.json` and
`pixel-fold-tests/9pro-legacy-lazy-bind.json` in the parent workspace. These
legacy results belong to the Pixel 9 Pro Fold; the earlier sparse/cache tests
described above belong to the first-generation Pixel Fold.

The coalescing-aware legacy callback supports weak_bind streams after all
images carry addressless strong-definition announcements collected globally.
Local direct definitions resolve in load order, preferring the first strong
definition over weak definitions. Missing symbols resolve to zero only for
explicit weak imports; required weak lookups fail. Coalescing through inherited
or explicit reexports and cache-backed dependencies remains unsupported and
fails explicitly. The conservative legacy API still rejects weak streams.

The bounded virtual Darwin shim supports exit, getpid, buffered stdout/stderr
write, zeroed anonymous private mmap, a stable single-thread identity,
monotonic virtual ticks with a 1/1 nanosecond timebase, and gettimeofday
anchored at process creation. BSD calls return errno in x0 with carry set;
Mach time traps return x0 without applying the BSD carry convention.
gettimeofday validates all requested guest output ranges before copyout.
Guest reads and writes check permissions and mapped bounds. Mach ports,
thread creation, filesystem services, mprotect, munmap and fixed/file-backed
mmap remain unsupported. These services do not initialize Apple frameworks.

LC_MAIN receives argc=1, a terminated argv containing the guest executable
path, an empty envp, and a terminated apple vector with executable_path.
No host environment or host filesystem path is exposed. Ordinary-image
initializers dispatch supported SVC calls through the same bounded process
shim, retaining context restoration and instruction limits. Cache preparation
continues to defer application and Apple initialization.

The runtime_services fixture checks PID, EBADF/carry, zeroed mmap memory,
virtual thread identity/timebase, monotonic time and gettimeofday copyout;
success exits 42. Regenerate it with tools/build_a64_runtime_fixture.sh.
No new device result is claimed here. Unsupported instructions halt with an
explicit error instead of aborting the host. General TLS, threads, Mach
services, Objective-C startup and ARM64e authenticated pointers remain
unsupported. The counter callback returns zero. A faulting JIT block can continue until its block boundary before
the preserved first memory fault is reported.

## Original Apple shared cache

The acquired firmware is `iPhone10,4`, iOS `16.7.16`, build `20H392`; provenance
and its verified SHA256 are in the parent workspace's
`ios-runtime/firmware/source.json`. The main cache is under
`ios-runtime/cache/System/Library/Caches/com.apple.dyld/`, including its required
subcaches. DriverKit's separate cache is not used for iOS framework imports.

`src/a64_cache.rs` validates the main/subcache identities and file ranges and
builds a mapping plan at original guest addresses. `src/a64_cache_map.rs`
maps the original files privately rather than copying or pretending that
extracted dylibs are independently loadable. Four slide-info-v2 data regions
are decoded in place. The desktop mapping experiment covered approximately
2.54 GiB of mapped cache bytes with approximately 329 MiB resident memory;
mapping and all four decodes also passed on the first-generation Pixel Fold.
These are cache acquisition/mapping results, not Apple runtime execution.

`src/a64_cache_symbols.rs` lazily reads image headers and exports from mapped
original-cache addresses. It translates export file offsets through the
image's `__LINKEDIT` mapping, checks install identities and resolves ordinary
exports and reexports. Command, trie and traversal limits are enforced, with
separate budgets for raw metadata and retained expanded names/index storage.

`src/a64_cache_imports.rs` performs a **read-only import availability audit**:
it compares app/embedded-image chained imports with independently linked
embedded libraries and cached Apple exports. The audit does not write app
binding slots, run Apple initializers, invoke symbol resolvers, enter
Objective-C/Swift startup or execute Delta. Missing or unsupported imports
remain reported explicitly; no success count establishes runtime compatibility.

The `ipsw`-extracted dylibs under `ios-runtime/extractedlibs` remain inventory
artifacts. Extraction can remove `MH_DYLIB_IN_CACHE` while leaving metadata
that depends on the shared cache. The `.shared-cache-exports` marker blocks
using this directory as an ordinary runtime root. Ordinary loading also
rejects cached/split-segment images and unsafe relocation without supported
metadata. The original-cache mapping experiment is a separate path from app
execution. Apple binaries are not bundled in the APK.

## Delta requirements and remaining work

The parent workspace's `Delta-runtime-requirements.json` records 46 required
and 30 weak direct library dependencies from Delta 1.7.1. Its IPA embeds
`Systems.framework` and `libswift_Concurrency.dylib`. The main executable has
4,041 chained imports; Systems has 861. Both use pointer format 2 and import
format 1. Dependencies include SwiftCore, SwiftUI, UIKit, Metal, Foundation,
Objective-C and libSystem.

Independent-image linking and cache symbol inspection have advanced beyond
the original two-syscall prototype. The next integration work still includes
connecting app bindings to a validated shared-cache runtime, Apple runtime
initialization and resolver/TLS semantics, Darwin/Mach and threading services,
Objective-C/Swift startup, and framework/graphics integration. A passing
synthetic fixture or a mapped cache does not supply these services.

The existing ARM32 HLE implementation continues to assume 32-bit addresses,
pointer-containing structures, register arguments and libc/framework scalar
types. Its Objective-C layouts and framework bindings therefore cannot simply
be called using the Apple ARM64 ABI. An HLE route would require that broader
ABI work; the original-cache route requires the operating-system/runtime
services expected by Apple's code.

Android Swift libraries are ELF binaries with different platform bindings.
They cannot substitute for the iOS Mach-O Swift overlays, UIKit, SwiftUI or
Metal referenced by Delta.

Pokemon Quest remains unplayable. The earlier MediaToolbox runtime-root
error in pixel-fold-tests/pokemon-quest-9pro.json is historical device
evidence. Current desktop original-cache preparation now reaches the genuine
_OSAtomicAdd32 export and reports an unsupported resolver. Empty-root export
tries and backward legacy pointer cursor movement have been fixed; cursor
subtraction is checked against underflow. No new phone result is
claimed here, and preparation never executes the game or Apple frameworks.

## Original iOS 11 cache and legacy C++ provider

The original iPhone 6 iOS 11.4.1 (15G77) firmware was acquired directly from
Apple and verified against firmware SHA256
969932ec35c7a3122936ec49e220657dfb3b1d2876e2517f4d3a0cb7a627738d.
The preserved original cache SHA256 is
b196c0c60837bb9708605e6ef53a068bc44f1c2872847eb1f1e006e1b9361f84.
It is at ios-runtime/legacy-cache/System/Library/Caches/com.apple.dyld/
dyld_shared_cache_arm64 in the parent workspace. The parser supports its
monolithic header, three original mappings, 1,318 images and global slide-v2
metadata. It is a complete primary cache; iOS 11 and iOS 16 mappings are
not mixed.

Its genuine /usr/lib/libstdc++.6.dylib export trie contains all 38 required
CydiaSubstrate symbols: 34 regular strong exports and four weak allocation
operator reexports. The old GCC C++ ABI was absent from the iOS 16 cache;
it is no longer an acquisition blocker for the iOS 11 cache. Current libc++
is an incompatible replacement. Binding and execution still require
validation. Source hashes and export evidence are in
ios-runtime/legacy-provider-provenance.json and
ios-runtime/legacy-libstdcxx-symbol-audit.json. Reproduce acquisition with
tools/obtain_legacy_ios_runtime.py.

The old filesystem proves the private IOSurface framework directory symlink
to ../Frameworks/IOSurface.framework. Its exact alias is restricted to cache
UUID 7336d75f-3014-33e7-843f-e1f3522fc52f and requires the canonical public
cached image. ios-runtime/legacy-iosurface-alias-proof.json records the
source OS image hash and inode; the adjacent filesystem listing is evidence.
The acquired iOS 16 cache has its independently proven alias scoped to UUID
32035564-853b-388b-a1ef-2ea354c196e1.

The --a64-cache-prepare=PATH_TO_MAIN_CACHE PATH_TO_IPA diagnostic maps the
original cache and ordinary app/embedded images, resolves supported imports,
and writes application binding slots. It reports deferred initializers and
retains an explicit execution gate. It does not invoke app main, initialize
Apple frameworks or establish game compatibility. Broader Apple initialization,
resolver/TLS behavior, Darwin/Mach services, Objective-C and graphics
integration remain required for a playable game.


## Isolated genuine Apple atomic resolver probe

The --a64-cache-resolver-test=PATH_TO_MAIN_CACHE diagnostic validates the
audited iOS 11 OSAtomicAdd32 resolver in isolation. The desktop probe executed
genuine Apple resolver and selected atomic-add code; adding 21 to a guest
value of 21 returned 42 and stored 42. This is actual bounded Apple code
execution, separate from a synthetic implementation or a full app launch.

The probe maps a read-only ARM64 commpage at 0xfffffc000 using the fixed
XNU 4570.71.2 ABI. Version is 3; the 32-bit capability word at offset 0x20
advertises baseline NEON/VFP/FMA and one virtual CPU. ARMv8.1 LSE atomics are
not advertised, selecting the baseline LL/SC implementation. Hardware and
approximate-clock fast paths remain disabled. The ARM signature area is
zero, matching XNU page initialization; no unsupported timestamps or fast
TLS behavior are advertised. This minimal commpage supports the audited
resolver experiment and is not a complete Darwin commpage environment.

Production app binding still rejects resolver exports. Pokemon Quest
therefore continues to stop at _OSAtomicAdd32 requiring an unsupported
resolver; the isolated success has not removed that guard or supplied Apple
runtime initialization. No gameplay or new phone result is claimed here.

## Validation and commands

Run `tools/verify_a64.sh` from the parent workspace under WSL. It verifies an
isolated source copy so Claude's build directory stays separate. It runs CPU,
loader/linker and cache metadata tests, freestanding self-tests and fixture
launch checks. Use the current command output for test counts; early results
with only two CPU/six loader tests predate the sparse and linked-image work.
APK builds and device checks are separate steps. The latest parent verification
passed 92 active tests: 12 CPU and 80 loader/runtime tests, with three ignored.
This result does not establish Apple runtime execution or gameplay.

```sh
cargo test -p touchHLE_dynarmic_wrapper --features a64
cargo test --features a64 --lib a64
cargo run --features a64 -- --a64-selftest
cargo run --features a64 -- --a64-run=tests/a64/hello_arm64.macho
cargo run --features a64 -- --a64-cache-info=PATH_TO_MAIN_CACHE
cargo run --features a64 -- --a64-cache-map-test=PATH_TO_MAIN_CACHE
cargo run --features a64 -- --a64-cache-import-test=PATH_TO_MAIN_CACHE PATH_TO_IPA
cargo run --features a64 -- --a64-cache-prepare=PATH_TO_MAIN_CACHE PATH_TO_IPA
cargo run --features a64 -- --a64-cache-resolver-test=PATH_TO_MAIN_CACHE
cargo run --features a64 -- --a64-run=tests/a64/runtime_services.macho
```

Use `--a64-runtime=PATH` only for independently loadable ordinary dylibs under
that filesystem root. Fixture generation scripts are in the parent workspace:
`tools/build_a64_import_fixture.sh` (LLVM 18 clang/Mach-O lld) and
`tools/build_a64_sparse_fixture.py`. The legacy fixtures and IPA bundles are
generated by `tools/build_a64_legacy_fixture.sh`.

Relevant implementation modules are `src/a64.rs`, `src/a64_linker.rs`,
`src/a64_fixups.rs`, `src/a64_exports.rs`, the three cache mapping/symbol/import
modules, `src/a64_cache.rs`, and `src/cpu/dynarmic_wrapper/a64.{cpp,rs}`.
`src/lib.rs` handles launch routing and diagnostic CLI options.

