# 32-bit game status (PlayCover-A / touchHLE fork)

Maintained by Claude (32-bit side). Codex owns ARM64 status in `WHERE_WE_ARE.md` / `NEEDS_TO_DO.md`.
"Verified" means I saw it in a device screenshot. Last updated 2026-10-07.

Devices: Samsung Tab S11 (USB), Pixel 9 Pro Fold (wireless), Pixel Fold gen 1 (wireless).

## Working

| Game | Verified on | Notes |
|---|---|---|
| Sonic the Hedgehog 1 (1.2.6) | Tab S11, Ben on Pixel | Green Hill renders; sound confirmed by Ben |
| Sonic 2 (1.2.2) | Tab S11, Ben on Pixel | Title + gameplay OK; minor tile glitches (black patch, a few wrong tiles) |
| Pocket God 1.39 | Tab S11 | Menu + island gameplay |
| Rock Band 1.1.38 | Tab S11 (Ben), screenshot of menu | Portrait-only app; menu upright |
| Resident Evil 4 HD | Ben (Tab S11) | Basic Training and touch movement |
| Angry Birds (HD 1.5.0) | Pixel Fold, Ben on Tab S11 | Title screen + gameplay |
| Flappy Bird 1.2 | Pixel Fold 2026-10-06; Tab S11 title screen 2026-10-07 | Needs `--ignore-unknown-selectors` (compat on). Tab S11 startup crash fixed (missing AVAudioSession constants + setValue:forUndefinedKey:); gameplay not re-checked after fix |

## Patches per game (root causes)

**Sonic 1 / 2**
- `.cxx_construct` was never called by `alloc`: `DrawLayersInfo` stayed zeroed, so background layers were disabled (ghosting/black world). Added `call_cxx_construct` in `allocWithZone:`.
- `CGImageCreate` over a guest-memory data provider copied the pixels once; now re-reads on `CGImageCreateCopy` (video buffer was frozen).
- Opaque-layer blending fix in `composition.rs` (black squares around D-pad / A / Start).
- Live streaming of `alBufferDataStatic` ring buffers with `AL_BYTE_OFFSET` polling in `openal.rs` (sound).
- `HostIMP` arity extended to 7 params; `NSString getBytes:...`, `NSArray getObjects:range:`, `NSSet minusSet:`.
- GL error assertion in the compositor downgraded to a warning (crash on resume).

**Pocket God**
- Added Security.framework stubs (`SecItem*` returning `errSecNotAvailable`, kSec* constants).
- `NSString getBytes:maxLength:usedLength:encoding:options:range:remainingRange:`, `dataUsingEncoding:` (UTF-16, Mac Roman), `NSPropertyListSerialization dataFromPropertyList` (error ptr, XML), `NSArray getObjects:range:`.
- `NSXMLParser`: errors reported via `parser:parseErrorOccurred:` instead of panicking; lossy UTF-8.
- Still logs GL error 0x500 repeatedly (non-fatal).

**Flappy Bird 2026-10-07 crash fix:** guest code dereferenced NULL non-lazy symbols (`AVAudioSession*` interruption keys and ports, `UIApplicationSignificantTimeChangeNotification`, `NSGregorianCalendar`, `NSUnderlyingErrorKey`, StoreKit/iAd constants): now exported as NSString constants in `av_audio_session.rs`. `setValue:forUndefinedKey:` no longer panics when `--ignore-unknown-selectors` is on (AdMob KVC on an NSArray).

**Flappy Bird / Angry Birds** (earlier work): OpenGL ES 2.0 passthrough, `--ignore-unknown-selectors`, libgcc auto-load, CCCrypt AES, NSCalendar, many stubs.

**App-wide (PlayCover-A)**
- Rotation: follows the window within the app's supported orientations; overlay menu now has a manual orientation picker (Material 3 style translucent overlay: Automatic / Portrait / Landscape left / Landscape right / Upside down; verified on Tab S11 that picking Landscape turns the window and content, shown sideways for a portrait-only game as expected).
- Background/foreground: event polling stays on (black screen on resume fixed), picture-in-picture when leaving the app.
- `dispatch_group_*`, `dispatch_async_f`, mixer AudioUnit (`aumx`) stub, bundle-relative file fallback in `fs.rs`.

## Needs testing (copied to Tab S11, not yet run)

Amateur Surgeon 1.0.1, Angry Birds 8.0.3, Angry Birds Rio HD 1.1.0, C&C Red Alert 1.7.0, Call of Duty WaW Zombies HD 1.1.0, Call of Duty Zombies 1.3.0, Candy Crush Saga 1.19.0, Civilization Revolution 2.1.2, Dead Space 1.0.3, Devil May Cry 4 Refrain 1.01, Final Fantasy VII G-Bike 1.1.0.
(Pokemon Quest is 64-bit: Codex's side.)

From Desktop\ipa, test one at a time: Simpsons Tapped Out 4.28 (needs dead EA servers), Infinity Blade II/III, Oceanhorn 3.01, Bully AE, Final Fantasy VII.

Fixed by static analysis only, never run on a device: Bejeweled 2 (resource path + mixer unit; was crashing in Sexy engine ctor on a NULL global), Plants vs. Zombies HD (null-page read after `initWithBytes:length:encoding:`), Bloons TD 5 (`dispatch_group`).

## Known issues
- Sonic 2: occasional wrong tiles / black patch.
- Pocket God and Rock Band rotation direction not confirmed by Ben after the landscape flip.
- Samsung ignores adb rotation locks, so rotation needs a physical turn.


## 2026-10-07 regression run (Tab S11, 20 s each, see REGRESSION.md)

Survived 20 s with no panic ("runs", not proof it plays): Angry Birds Rio HD, C&C Red Alert, Call of Duty WaW Zombies HD, Dead Space, Devil May Cry 4 Refrain, Flappy Bird, LEGO Harry Potter 1-4, Modern Combat 5, Pocket God, Rock Band, Secret of Mana, Sonic 1, Sonic 2, Spore Origins. Left the foreground without a panic: Call of Duty Zombies 1.3.0, Prince of Persia Warrior (file was truncated at the time, recopied afterwards), Tony Hawk's Pro Skater 2.

First panics found (next fixes, in rough order of how cheap they look):
- Amateur Surgeon: `AudioSessionSetProperty` unimplemented property 'uifx'
- Candy Crush: `strerror(6)` not implemented
- Civilization Revolution: `ns_string.rs:111` asserts ASCII-only bytes
- Mega Man II: `UIImage` "unknown image type"
- PAC-MAN Remix: `ns_run_loop.rs:105` mode assertion
- Playboy: `NSFileManager createDirectoryAtPath` attributes != nil
- Sonic CD: `NSDictionary` assertion left != right; Sonic 4 Ep. I: `NSString` encoding 0x80000001
- Zenonia 1/2: CFStringEncoding 0x422; Worms 2: unimplemented host method; Sonic 20th: `UIView` insertion index
- N.O.V.A. 2 HD: unimplemented GLES call; NFSU: `audio_file.rs:305`
- MVC2: pthread mutex lock-recursion; Scribblenauts Remix: nib archive decoder
- Null-page reads: Angry Birds 8.0.3, FFVII G-Bike, Minecraft PE, Mirror's Edge, PvZ 2, Street Fighter IV

## Tools and launcher features added 2026-10-07 (unverified items marked)
- `tools/regress.ps1`: per-IPA launch/liveness/panic check, writes REGRESSION.md and pushes `compat.json` to the device.
- Launcher status badge per tile (Works / Partial / Broken marked by hand via "Compatibility details" > Mark; Runs / Crashes from compat.json and from crashes the launcher sees; Untested). Verified on the Tab S11.
- Launcher crash card after a game panics (name, panic, registers, Copy report). Verified on the Tab S11 (Amateur Surgeon).
- Import check: the launcher asks the native library which functions/constants/classes it implements (`SymbolIndex`, JNI `Java_org_touchhle_android_SymbolIndex_exportedSymbols` in `dyld/dylib_list.rs`) and compares them with each 32-bit IPA's undefined symbols ("N imports missing"; cached in `scan_cache.json`). Verified on the Tab S11. The counts include CF/CG C functions that look like constants; "serious" = missing C functions + data symbols.
- Mixer audio unit: `AudioUnitRender` is now exported and a mixer mixes its input-bus render callbacks (16-bit interleaved mono/stereo only). **Built, not tested on a device** (needs Bejeweled 2 or another aumx game).
- Sound: AIFF (PCM) files are rewrapped as WAV (Flappy Bird sfx_point.aif); non-looping `alBufferDataStatic` sources now play the whole buffer once instead of 3 short chunks. Sonic still sets AL_LOOPING=1 so its ring-buffer path is unchanged. **Neither sound change heard by me.**
- Icon picking prefers exact names from the plist (fixes blank icons for Red Alert and Dead Space, where `icon_background.png` won by file size).

## 2026-10-07 later: Amateur Surgeon touch fix and next targets
- **Amateur Surgeon** now takes touch input (verified on the Tab S11: New Game > "Are you sure?" > intro scene). Cause: the game copies its live-touch dictionary with `-[NSMutableDictionary initWithDictionary:copyItems:]`, which we did not implement, so it got an empty dictionary and ignored every touch. Added to both host dictionary classes. Earlier fixes in the same session: unknown AudioSession properties accepted (it crashed on 'uifx').
- Latest full regression (before the dictionary fix): 17 run, 21 crash, 2 leave the foreground. Rows in REGRESSION.md now list "[unimplemented calls: ...]" (functions touchHLE ignores and returns 0 for).
- Next cheap targets from that run: `printf` `%ll` length modifier (Candy Crush), `class_addMethod` / `object_getClass` (Worms 2, Street Fighter IV), C++ `__ZNSt3__...` libc++ symbols (Minecraft PE, Mirror's Edge, PvZ 2: they link libc++, which touchHLE does not bundle), `__Unwind_SjLj_*` (Angry Birds 8.0.3), thread-exit routines (Prince of Persia), `ns_string.rs:902` encoding 0x80000001 (Sonic 4), file manager `createDirectory` attributes (Playboy), `UIImage` formats (Mega Man II).
- Also fixed this session: `localeconv`, `kCFNull`, `OBJC_IVAR_$_NSObject.isa`, `dlopen` of unknown libraries returns NULL, GLES2 PVRTC decode, Korean text encodings, UIGraphics image contexts + PNG/JPEG export.

## libc++ for 32-bit games (investigation 2026-10-07)
- Games compiled with libc++ (PvZ 2, Mirror's Edge, Minecraft PE, probably Angry Birds 8.0.3) import `/usr/lib/libc++.1.dylib`, which touchHLE does not bundle (it ships libstdc++ 4.0, libgcc, zlib, sqlite, libxml2). Their crash is a null read after an unresolved libc++ symbol (e.g. `std::__1::basic_string` members, `std::logic_error`).
- Minecraft PE 1.1.7 is an iOS 9+ Swift app (Photos, WebKit, libswift*.dylib): libc++ alone will not run it.
- Route found: touchHLE's own open SDK `touchHLE/common-3.0-sdk` (release v0.3.7, clang + cctools `ld`) can cross-build armv7 Mach-O dylibs, the way touchHLE's zlib/sqlite/libxml2 dylibs are built. LLVM libc++/libc++abi source: `llvm/llvm-project` (Apache-2.0 with LLVM exception).
- Status: `tools/build_libcxx.sh` (WSL, work dir `~/libcxx-build`) configures LLVM 18.1.8 libc++ + libc++abi for armv7 and compiles 46 of 68 objects. Remaining errors come from the 2009-era SDK headers not matching LLVM 18 (xlocale `_time.h`, `CLOCK_REALTIME`/`clock_gettime`, missing `aligned_alloc`, Apple `_bounds.h`). Next step if continued: use an era-matched LLVM 3.x-6 libc++ (matches the C++ ABI iOS 5-7 apps were built against) instead of 18, then link the dylib as `/usr/lib/libc++.1.dylib` with the libc++abi symbols folded in, ship it in `touchHLE_dylibs`, and map it in `src/fs.rs` like libstdc++.

## libc++ integrated (2026-10-07, built; last fix not yet verified on a device)
- `touchHLE_dylibs/libc++.1.dylib`: LLVM 3.4.2 libc++ built for armv7 with touchHLE's common-3.0 SDK (`tools/build_old_libcxx.sh`, `tools/link_old_libcxx.sh`; base 0x37800000; 26 sources, 223 of the 251 `std::` symbols Minecraft imports; the rest, mainly `std::exception` classes, come from the bundled libstdc++). Mapped at `/usr/lib/libc++.1.dylib` in `fs.rs`; apps that link libc++ also get libstdc++ and libgcc loaded (`environment.rs`).
- First test (PvZ 2, Mirror's Edge): no more null reads from missing C++ symbols; C++ static constructors from the new libc++ ran, then both aborted because `std::random_device` opens `/dev/urandom`, which did not exist.
- Since added: `/dev/urandom` and `/dev/random` as readable guest devices (`GuestFile::RandomDevice` in `fs.rs`) and `mbtowc_l`. Built and installed on both devices, but not re-verified: the Tab S11 went to sleep behind its lock screen during the test run, so the later runs showed false "exited" results.
- Angry Birds 8.0.3 (iOS 8 min) additionally hits `dlsym` with a real handle (`dlfcn.rs:42`).

## libc++ games, later 2026-10-07
- PvZ 2 now gets past all C++ static initialisation, locale setup and early app startup (libc++ needs `newlocale`/`*_l` functions, `strftime` `%A %B %r ...`, `/dev/urandom`, 64-bit `OSAtomic*`, `__udivmodsi4`): it ran 20 s without a panic, black screen. Further gaps found and fixed on the way: `NSApplicationSupportDirectory`, `UIEdgeInsetsZero`, `kCMTimeZero`, AV gravity and CFPreferences-domain constants, tolerant CFPreferences (other app ids, `CopyKeyList`, `CopyValue`/`SetValue`), `NSFoundationVersionNumber`, `kern.proc.pid` sysctl (debugger check).
- Latest crash for PvZ 2 before the tablet locked again: none after the sysctl fix was built (untested). Mirror's Edge showed the same early path; both still need a device re-run.
- The Tab S11 screen timeout (30 min) locks it behind the PIN screen during long sessions: unlock it before asking for device tests.

## Latest libc++-era fixes (2026-10-07, evening)
- Sonic CD / Mirror's Edge: now past `objc_lookUpClass`, `objc_copyClassList`, missing `Digits` class (`NSClassFromString` returns nil in compat mode), static `NSOperationQueue` retain, class-object `autorelease` return type and `+superclass`, `reallocf`, `sel_getUid`. Last verified stop before the final fixes: a null read after `reallocf`/`sel_getUid` were unimplemented (retest pending, tablet locked).
- PvZ 2: stops in Crashlytics (`-[CLSWebClientTransaction startConnection]` asserts its NSURLRequest is non-nil) after: CFPreferences, sysctl, `CFGetTypeID`, exclusive stores (`strexb`), nil-tolerant dictionary/string setters, plist UnsignedLongLong.
- CPU: `MemoryWriteExclusive8/16/64` implemented (a native abort in `Dynarmic` before).
- The Tab S11 relocks after its 30-minute timeout; device tests need it unlocked.

## Regression 2026-10-07 night (40 IPAs, Tab S11, 20 s each): 16 run, 22 crash, 2 leave the foreground
New this session: blocks runtime (`objc/blocks.rs`), ARC weak references, libc++ + its libc support, `CFGetTypeID`, `objc_lookUpClass`/`copyClassList`, tolerant `objc_getClass`/`NSClassFromString` in compat mode, real `NSURLRequest` with network off, nib geometry tag relaxed, lossy UTF-8, NSOperationQueue/class retain fixes.
- Moved on from their old crash: Civilization Revolution and Amateur Surgeon (run), PvZ 2 and Mirror's Edge (libc++ startup passes; both now fault at a null singleton in the engine's own C++: `std::string` copy from address 0x10, caller 0x102783 in PvZ2), Sonic CD (null read near 0xa85ba earlier, blocks added since), Minecraft PE (iOS 9+ Swift app: now 3600 log lines in, next stop `sysctlbyname machdep.cpu.vendor`), Scribblenauts Remix (now at missing `UIToolbar`).
- Next cheap targets: Candy Crush printf `%ll`, Worms 2 `class_addMethod`, Minecraft `sysctlbyname machdep.cpu.*`, Scribblenauts `UIToolbar`, Prince of Persia thread-exit routines, Playboy `createDirectory` attributes, Zenonia 1 allocator page size, Zenonia 2 `MC_knlGetResource` (missing resource).

## Update 2026-10-07 (UIToolbar / NSFileManager)
- Playboy 1.1.45: NSFileManager default manager is now immortal (retain/release/autorelease no-ops), createFileAtPath/createDirectory accept attributes + fill NSError. Regress: runs (no screenshot yet, needs visual check).
- Scribblenauts Remix 7.8: UIToolbar added (ui_toolbar.rs), attributesOfItemAtPath no longer asserts, fs modified()/size() handle directories. Now stops at a null-page read (mem.rs:357, 0x0), 505 log lines.
- LEGO Harry Potter / Secret of Mana Pixel Fold screen issues: not yet investigated (need symptom).

## Update 2026-10-07 (Pixel 9 Pro Fold screen issues)
- Root cause (LEGO Harry Potter 1-4): Info.plist lists all four orientations, so Android device-rotation following turned the near-square unfolded Fold (2076x2152) to Portrait; the landscape-only game was then clipped into a portrait viewport. Fix (environment.rs): if the first listed orientation is landscape, only follow landscape rotations. Verified on Pixel 9 Pro Fold by screenshot: full landscape title menu (lego_fold_fixed.png).
- Secret of Mana: NOT fixed. Info.plist has no orientation keys (so touchHLE assumes portrait), but the game draws 480x320 landscape art into a 320x480 renderbuffer (title/copyright cropped). `--landscape-left` makes it worse (art rotated 90 deg). Needs a look at what viewport/size the game expects (som_fold.png, som_fold_fixed2.png).
- Diagnostics added: "composite: viewport ..., drawable ..." and "Rotating device A -> B" log lines; "Supported interface orientations ... -> mask" at startup.

## Update 2026-10-07 (Secret of Mana landscape fix, gen 1 test)
- Secret of Mana root cause: Info.plist has NO orientation keys, but the game calls `[UIApplication setStatusBarOrientation:]` to go landscape and then draws a fixed 480x320 scene (glViewport 480x320, ortho 0..480 x 320..0). Two bugs: (1) on Android the sensor-following override replaced the app's own request with Portrait (fixed: `note_app_requested_orientation` in window.rs); (2) touchHLE never resized the game's views, so the EAGL renderbuffer stayed 320x480 (fixed: `adapt_root_views_to_landscape` in ui_application.rs gives the window's root views landscape bounds + rotation transform and calls layoutSubviews, only for apps with no orientation keys in Info.plist). Verified by screenshot on Pixel 9 Pro Fold: title, name-entry screen, then the intro cutscene maps render full-screen and upright.
- OPEN: in-game UI text is missing (name-entry button labels, dialog box text). Pre-rendered title art and the pixel story text do render. Game builds an ASCII width table via `-[NSString sizeWithFont:]` (UIFont 12) but never calls any UIKit draw method; its text strings come from systxt_en.bin / scrtxt_en.bin; glyph textures are all RGBA from game-decoded gif/gix. Suspect the game's own glyph/script decoding path.
- Pixel Fold gen 1 (192.168.0.51:35429): both Secret of Mana and LEGO Harry Potter run with correct landscape viewports (2208x1472 centred). A system overlay dialog (package `android`, APPLICATION_OVERLAY, DIM_BEHIND) holds window focus on that device so no screenshot was taken (focus rule).
- adb pitfalls on gen 1: the app must create touchHLE_apps itself (a shell-created dir makes the app fail with "Permission denied"); pushed IPAs need `chmod 666`.
- Temporary diagnostics still in the source (remove when done): glTexImage2D log, sizeWithFont/initWithBytes sample logs.

## Update 2026-10-07 (Secret of Mana text FIXED, verified on Pixel Fold gen 1)
- Root cause: the game renders every glyph by putting one character in a `UILabel` and calling `-[CALayer renderInContext:]` into a CGBitmapContext, then caches the pixels in `Documents/sk2_fontcache_en_tex_0..3`. touchHLE had no `renderInContext:` (silently ignored), so the cache was written all-zero and then reused forever (blank text everywhere in menus/dialogs, while pre-rendered art and pixel story text were fine).
- Fix: implemented `-[CALayer renderInContext:]` (ca_layer.rs `render_layer_in_context`: background colour, delegate `drawLayer:inContext:`, sublayers). First launch after deleting the stale cache regenerates the atlases (~10+ s; Android may show an ANR dialog during that one-time pass); later launches are fast.
- NOTE: an old blank cache must be deleted once: `touchHLE_sandbox/com.square-enix.sk2/Documents/sk2_fontcache_en*`.
- Verified by screenshot on Pixel Fold gen 1: main menu (Options / New Game / Load Game / Manual / Website, Back button, scrolling hint text) and intro cutscene narration text ("In time, Mana was used to create the ultimate weapon...").
- Diagnostics from the investigation were removed.

## Update 2026-10-07 (LEGO Harry Potter touch FIXED, verified on Pixel Fold gen 1)
- Symptom: "Touch the screen to begin" never advanced. Root cause: the game keeps an UNRETAINED pointer to the `UIEvent` passed to `touchesBegan:` and polls `[event allTouches]` every frame (without checking touch phases). touchHLE allocated a fresh UIEvent per touch and freed it, so the game read a dead object. Fix (ui_event.rs / ui_touch.rs): one persistent shared UIEvent (like iOS), updated in place, and after `touchesEnded` its `allTouches` is reset to only the touches still down.
- Second bug found right behind it: after the tap the game starts an MPMoviePlayer intro and converts points between layers that are no longer in the same tree; `transform_for_conversion` (ca_layer.rs) panicked "have no common ancestor". It now logs a warning and uses an identity conversion.
- Verified by screenshot on gen 1: title -> main menu (Play/Options/Credits/More From WB) -> Play -> save-slot screen (Game 1 - 0% / Empty). Taps made before the title finishes loading (~30 s) are ignored by the game itself.
- Pixel 9 Pro Fold was not connected at the end of this session (adb offline), so the final build is installed on the gen 1 and the tablet only.

## Update 2026-10-08 (LEGO Harry Potter: intro movie)
- Gen 1 run: title -> Play -> Game 1 -> Load Game now starts the intro movie `y1_mainintro.m4v` through `MPMoviePlayerController`, which was only partly stubbed, leaving a black screen.
- Added to movie_player.rs: `prepareToPlay` (posts MPMoviePlayerLoadStateDidChangeNotification), `loadState` (=3), `isPreparedToPlay`, `backgroundView`, `setMovieSourceType:` and the `MPMoviePlayerLoadStateDidChangeNotification` constant. Notifications are confirmed to be posted, but the game still never calls `-play`, so it stays black. NEXT: find out what the game's load-state handler checks (the build in place logs `loadState`/`isPreparedToPlay`/`playbackState` polls; verify with a run that actually reaches the movie).
- Testing note: the last automated run was cut short because another app (not touchHLE) had focus on the gen 1; no further blind taps.
