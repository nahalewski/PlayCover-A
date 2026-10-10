# ARM64 non-UIKit system-framework plan (Coromon 1.4.1 first)

Scope: the framework layer *under* UIKit for the experimental ARM64 runtime
(`src/a64*.rs`). UIKit/QuartzCore views, `UIApplication`, `CALayer` and
`CADisplayLink` belong to the separate UIKit work (`src/a64_uikit*.rs`).
Initializer execution of cached dependency images belongs to the Initializer
work (`a64_cache_*`, `a64_execution_session`, `a64_dyld_*`). This document
owns `src/a64_frameworks*.rs`.

Nothing here claims Coromon boots. As of 2026-10-10 the session diagnostic
runs the genuine libSystem initializer, and dependency initializers are being
brought up. The framework layer is ahead of that boundary. It is built and
tested natively so it is ready when app code first calls into it.

## 1. What Coromon actually imports

Source: `tools/a64_framework_needs.py Payload/Coromon.app/Coromon`. It parses
the chained-fixup import table per library ordinal, plus `__objc_methname`
and `__cstring`. Coromon is Corona/Solar2D build 3722. Firebase and the Corona
plugins are linked statically. Bundled: `MetalANGLE.framework`, which is
replaced separately, and the `libswift*` dylibs.

| Framework | Imports | What is used |
| --- | --- | --- |
| OpenAL | 57 | Full AL 1.1 core used by ALmixer: `alc{Open,Close}Device`, `alc{Create,Destroy,MakeCurrent,Process,Suspend}Context`, `alcGetCurrentContext`, `alcGetContextsDevice`, `alcGetError`, gen/delete/is buffers and sources, `alBufferData`, the source/listener setters and getters, play/pause/stop/rewind, queue/unqueue, `alGetString`, `alGetEnumValue`, `alGetProcAddress`, `alIsExtensionPresent`, the global state getters, doppler and speed of sound |
| AudioToolbox | 14 | `AudioSession{Initialize,SetActive,SetActiveWithFlags,GetProperty,SetProperty}`; `AudioFileOpenWithCallbacks`/`AudioFileGetProperty`/`AudioFileClose`; `ExtAudioFile{WrapAudioFileID,SetProperty,Read,Seek,Dispose}` (ALmixer's CoreAudio decoder); `AudioServicesPlaySystemSound` |
| AVFoundation | 7 | `AVPlayer`, `AVPlayerLayer`, `AVPlayerItemDidPlayToEndTimeNotification`, gravity constants (video playback only). No `AVAudioPlayer`/`AVAudioSession` |
| CoreMedia | 3 | `CMTimeGetSeconds`, `CMTimeMakeWithSeconds`, `kCMTimeZero` (with AVPlayer) |
| CoreGraphics | 41 | Bitmap contexts (`CGBitmapContextCreate`, `CGBitmapContextCreateImage`, `CGContextDrawImage`), color spaces, gstate/CTM, paths and rounded rects, `CGImageCreate`/`GetWidth`/`GetHeight`, data providers, `CGFontCreateWithDataProvider`, rect helpers |
| ImageIO | 2 | `CGImageSourceCreateWithURL`, `CGImageSourceCreateImageAtIndex` (Rtt_AppleBitmap image loading) |
| CoreText | 1 | `CTFontManagerRegisterGraphicsFont` (custom TTF registration) |
| UIKit text | n/a | Text is rasterised by `-[NSString drawAtPoint:withAttributes:]` and `boundingRectWithSize:...` into a gray CGBitmapContext (`UIGraphicsPushContext` path). This is owned by UIKit; it needs CoreText/CoreGraphics underneath |
| CFNetwork | 31 | `NSURLSession`/`NSURLConnection`/`NSMutableURLRequest`, cookies, auth challenges, `CFHTTPMessage*` (Corona network lib, Firebase/GTMSessionFetcher, WebSockets plugin), SSL stream keys |
| SystemConfiguration | 7 | `SCNetworkReachability*` (Reachability, GULReachability) |
| Security | 38 | `SecItem{Add,CopyMatching,Update,Delete}` (keychain: Firebase installation ID, Corona plugins), `SecRandomCopyBytes`, `SecTrust*`/`SecCertificate*`/`SecPolicy*` (TLS pinning) |
| StoreKit | 40 | `SKPaymentQueue`, `SKProductsRequest`, `SKMutablePayment`, `SKReceiptRefreshRequest`, `SKStoreReviewController`, `SKAdNetwork`, weak StoreKit 2 Swift symbols |
| GameKit | 13 | `GKLocalPlayer`, `GKLeaderboard`, `GKAchievement`, `GKScore`, `GKGameCenterViewController`, turn-based and matchmaker classes (gameNetwork plugin) |
| QuartzCore | 2 | `CACurrentMediaTime`, `CADisplayLink` (UIKit work) |
| Others | small | MessageUI (2 classes), WebKit (`WKWebView`), SafariServices, LocalAuthentication (`LAContext`), CoreMotion (`CMMotionManager`), CoreLocation (`CLLocation`), CoreTelephony, DeviceCheck (`DCDevice`), GameController (weak), weak CloudKit/Social/AuthenticationServices/SwiftUI |
| libz, libsqlite3, libc++ | 6/70/92 | Pure user-space code |
| CoreFoundation, Foundation | 74/70 | Already partly owned (`a64_cf*`, `a64_foundation*`) |

Linked, but no symbol imports (only load-time dependencies): GLKit, MapKit,
MediaPlayer, MobileCoreServices.

### Order of first use at startup (from the Solar2D sources)

`UIApplicationMain` creates the app delegate and then the `CoronaView`.
`Runtime::LoadApplication` reads `config.lua` and then **retains
`PlatformOpenALPlayer`** (`librtt/Rtt_Runtime.cpp:1314`). This calls
`IPhoneAudioSessionManager::SetProperty(PreferredHardwareSampleRate)` and
`SetAudioSessionActive(true)`, which are AudioToolbox `AudioSession*`. It
then calls `ALmixer_Init`: `alcOpenDevice(NULL)`, `alcCreateContext`,
`alcMakeContextCurrent`, then `alGenSources(32)`, and starts a streaming
thread. All of this happens **before `main.lua` runs**. After that,
`main.lua` loads images (ImageIO and CoreGraphics), rasterises text (UIKit
and CoreText) and loads sounds (AudioFile/ExtAudioFile and `alBufferData`).
Reachability, Firebase keychain and network calls come from plugins and SDK
initialisation, mostly asynchronously.

## 2. Classification

Rule: a genuine cache implementation is preferred whenever its kernel, Mach
and XPC dependencies can be served honestly. Where the service is
daemon-backed, the emulator provides an owned implementation of the
**function contract**. When the real device could also legitimately report
"unavailable", owned code reports that honestly and never fabricates success:
no fake purchases, servers, authentication or receipts.

| Class | Frameworks | Why |
| --- | --- | --- |
| **Genuine from cache** | libz, libsqlite3 (with real file I/O), libc++, CoreFoundation/Foundation except the owned routes, CMTime arithmetic | Pure user-space. Needs only the libSystem/Mach services already being served |
| **Genuine first, owned only on evidence** | CoreGraphics bitmap contexts and paths, ImageIO PNG/JPEG decode, CoreText font registration and layout | CPU-only in principle. Risks: CG/ImageIO initializers touching IOSurface/IOKit, `CGImageSourceCreateWithURL` needing file I/O through CFURL, and CoreText font registration contacting `fontservicesd` (XPC). Decide per call after the Initializer work reaches them. If owned, reuse the 32-bit host rasteriser (`src/frameworks/core_graphics/*`, `core_text.rs`) |
| **Owned, required** | OpenAL; AudioToolbox `AudioSession*`, `AudioFile*`, `ExtAudioFile*`, `AudioServicesPlaySystemSound`; SystemConfiguration reachability; Security keychain `SecItem*` (persisted per app, like `src/frameworks/security.rs`); `SecRandomCopyBytes` (host entropy) | Real implementations need mediaserverd/audio HAL, configd, securityd and the keychain XPC services |
| **Owned, honest "unavailable"** | GameKit (`GKLocalPlayer` stays unauthenticated; the authenticate handler gets a GameKit "not supported/cancelled" error); StoreKit 64-bit (`canMakePayments` NO, and product requests fail with a real `SKErrorDomain` error); MessageUI (`canSendMail`/`canSendText` NO); LocalAuthentication (`canEvaluatePolicy` NO with a biometry-not-available error); DeviceCheck (`isSupported` NO); CoreMotion (accelerometer/gyro unavailable until host sensors are wired); CoreLocation/CoreTelephony (no carrier, services disabled); weak CloudKit/Social/AuthenticationServices (the app checks the class or account state); WebKit/SafariServices (no web view; the Corona web popup reports failure) | Real iOS has the same "no account/no capability" states. The game must already handle them |
| **Networking: owned host-backed** | CFNetwork `NSURLSession`/`NSURLConnection`/`CFHTTPMessage` | Real CFNetwork needs nsurlsessiond/networkd. The 32-bit host HTTP client (`src/frameworks/foundation/http_client.rs`) is the model. Before it exists, requests fail with `NSURLErrorNotConnectedToInternet`, which the game must tolerate offline. That is not fabricated content |
| **Separate track** | MetalANGLE GLES (replaced with host GLES forwarding), UIKit/QuartzCore views, AVPlayer video | MetalANGLE: `gl*` imports bind to the bundled framework today. The design stub is to route its exported `gl*`/`MGL*` symbols to host GLES through the same family dispatcher (section 3). Video: an owned AVPlayer reports the item failed, or plays through a host decoder later |

StoreKit note: the 32-bit store-purchase/version-spoofing code (`store_kit*`,
`GameSettings.java`, `live_settings.rs`) is out of scope and must not be
edited. Any 64-bit StoreKit behaviour beyond "unavailable" must be
coordinated with whoever owns that feature.

**Coverage limit:** routing only rebinds imports of the *app and its
embedded images*. Calls made from inside cached dylibs still reach genuine
code. Examples: UIKit's `-[UIImage imageWithContentsOfFile:]` reaching
ImageIO, and Foundation's `NSURLSession` reaching CFNetwork internals. Owning
those requires the owned ObjC class route (`a64_objc_namespace.rs`), not
import routing. This matters for UIKit's image and text paths.

## 3. How owned functions are registered

Existing mechanism (`a64_bridge.rs`, `a64_host_services.rs`, `a64_linker.rs`):

- `GuestBridge::register_service(cpu, name, closure)` writes a 16-byte stub
  `movz x16,#token; svc #0x7d; ret` into the single 4 KiB bridge code page.
  `MAX_SERVICES = 128`, and roughly 85 slots are used by
  CF/ObjC/Foundation/dyld services.
- `SelectedServices.bindings[(provider install name, "_symbol")] = stub`.
  `CacheContext::definition` resolves the **genuine** cache export first.
  `SelectedServices::route` then substitutes the stub, only for a strong,
  aligned, executable original export. Owned code can therefore never invent a
  provider or a symbol that the real cache lacks.
- Handlers receive a `ServiceFrame`:
  - `integer(0..7)`, `vector(0..7)` and `stack_u64` give the arguments.
  - `read`/`write` access guest memory. Each call has a 1 MiB budget.
  - `request_guest_call` makes guest callbacks.
  - The handler returns `ReturnValues` (x0/x1 and v0..v3).

Coromon alone needs about 160 owned C functions in OpenAL, AudioToolbox,
Security, SystemConfiguration and CoreGraphics, so one bridge slot per
symbol cannot work. **Family dispatch** (`src/a64_frameworks.rs`) solves
this:

- Each framework family registers **one** bridge service:
  `_touchHLE_a64_frameworks_<family>_dispatch`.
- `a64_frameworks` maps its own RX trampoline page. Each routed symbol gets an
  8-byte trampoline, `movz x17,#index; b <family stub>`.
  - x17 (IP1) is call-clobbered by the AAPCS64/Apple ABI, as x16 is.
  - LR is preserved, so the family stub's `ret` returns straight to the app.
  - The bridge's `pc == stub + 8` check still holds.
- The bridge hook is one accessor, `ServiceFrame::dispatch_index()`, which
  reads x17.
- One 4 KiB page holds 512 trampolines. A family costs one bridge slot.
- The family also owns an RW arena page for guest-visible opaque handles
  (`ALCdevice*`, `ALCcontext*`) and stable C strings (`alGetString`). Handles
  are never host pointers.
- `SelectedServices::enable_frameworks` places the pages after `scratch_end`,
  inserts each `(provider, symbol) -> trampoline` binding, and adds the page to
  `instruction_ranges()`.

ABI reminders:

- A `float` argument is the low 32 bits of `vector(n)`, counted separately
  from the integer registers. A float return goes in `vectors[0]` low bits.
- Struct-by-value CG types (CGRect is 4 doubles) use v0..v3 in and out.
- Large struct returns use x8 (`indirect_result`), so x8 must never be used
  as the dispatch index.

## 4. Reusing the 32-bit code

The 32-bit `src/frameworks/*` modules use `Environment`, `Mem`, 32-bit
`GuestUSize` and touchHLE's own ObjC runtime. None of these exist in the
ARM64 session. What can be reused is the **host layer** below them:

- `src/audio/openal.rs` / `touchHLE_openal_soft_wrapper`: openal-soft FFI.
  This is used directly by `a64_frameworks_openal.rs`.
- `src/audio.rs` (symphonia/IMA4 decoding): for owned AudioFile/ExtAudioFile.
- `src/frameworks/core_graphics/*`, `core_text.rs`, `src/image.rs`: host
  rasterisation and decoding if CG/ImageIO must be owned.
- `src/frameworks/foundation/http_client.rs`: the host HTTP backend.
- `src/frameworks/security.rs`: keychain persistence format.

Guest pointer handling must be rewritten. ARM64 pointers are 64-bit, and
guest memory is a set of separate regions accessed only through
`ServiceFrame::read/write` (or `A64Cpu::read_guest_into/write_guest_into`).
There is no flat `Mem`.

## 5. Milestones (each testable on the desktop)

1. **M1 (done): family dispatcher plus OpenAL core.**
   - Trampoline/arena pages, `ServiceFrame::dispatch_index`, and
     `SelectedServices::enable_frameworks`.
   - All 57 OpenAL functions Coromon imports, backed by OpenAL Soft
     (`src/a64_frameworks_openal.rs`).
   - 7 native tests, including a replay of ALmixer_Init and of buffer
     playback/streaming through real guest trampolines, using the OpenAL Soft
     null backend.
2. **M2 (done): routing wired into every selected-service session.**
   - `SelectedServices::install` enables the default families. It costs one
     bridge slot and 16 KiB, which `Selection::mapped_bytes` counts.
   - Desktop session harness evidence (2026-10-10):
     - Coromon's routed import records rose from 28 to 85. All 57 OpenAL
       imports bound over genuine strong iOS 16.7.16 exports.
     - The libSystem-initializer boundary is unchanged for Coromon and for
       Terraria.
   - No owned OpenAL call has run inside the app yet: the session has not
     reached app code.
3. **M3: AudioToolbox owned.**
   - `AudioSession*`: properties that openal-soft really supports. Set
     sample rate is recorded, and get returns the actual mixer rate.
   - `AudioFileOpenWithCallbacks` reads through guest callbacks
     (`request_guest_call`). `ExtAudioFile*` decodes via symphonia into the
     client format.
   - Test: decode a WAV/OGG fixture through guest read callbacks.
4. **M4: CoreGraphics/ImageIO/CoreText decision.**
   - Run the genuine `CGBitmapContextCreate`/`CGContextDrawImage`/
     `CGImageSource*` once the initializers pass.
   - Only if they need unavailable daemons, own them with the 32-bit host
     rasteriser.
   - Test: decode a bundled PNG into an RGBA bitmap and compare pixels.
5. **M5: SystemConfiguration, Security keychain and SecRandom owned. GameKit,
   StoreKit, MessageUI, LocalAuthentication and DeviceCheck honest-unavailable
   ObjC classes** via `a64_objc_namespace`.
6. **M6: CFNetwork** offline-honest errors first, then the host HTTP backend.
7. **M7: MetalANGLE GLES forwarding** through a `gles` family.

## 6. Limits that are known now

- `alBufferData` larger than the 1 MiB per-call service budget is rejected
  explicitly, with an error naming the size. ALmixer streams music in small
  chunks; large predecoded sounds need a bridge bulk-copy API.
- Calls from guest threads other than the main thread go through the same
  bridge. Scheduling belongs to the thread scheduler.
- The owned `alSourceUnqueueBuffers` reads `alGetError()` before it unqueues.
  It needs a clean read so that the guest array is written only on success,
  and an error is raised again on failure. As a side effect, an AL error the
  guest had not read yet from an earlier call is discarded. ALmixer checks
  errors after each call, so this should not affect it.
- Legacy iOS 11 check (Infinity Blade II, `session-image-info`): preparation
  and the boundary are identical with and without the framework hook. IBII
  routes no OpenAL imports through the cache.
