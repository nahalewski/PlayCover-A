# ARM64 UIKit layer plan (Coromon 1.4.1 first)

Status: milestone 1 implemented and passing desktop tests (2026-10-10).
Owner: UIKit agent. Files: `src/a64_uikit.rs`, `src/a64_uikit_image.rs`,
`src/a64_uikit_state.rs`, `src/a64_uikit_tests.rs`. Shared-file hook: a single
`mod uikit` line in `src/a64.rs`.

## 1. Why an emulator-owned UIKit

The ARM64 runtime executes genuine Apple code from the iOS 16.7.16 shared cache
(libSystem's initializer already returns). Genuine UIKitCore cannot work there:
it needs SpringBoard, BackBoard and FrontBoard, plus CARenderServer and their
Mach services. UIKit is therefore an emulator-owned layer that the app and
MetalANGLE subclass through the ordinary Objective-C ABI. The 32-bit UIKit in
`src/frameworks/uikit/` is a behaviour reference only, because it uses 32-bit
pointers and the touchHLE ObjC runtime.

## 2. What Coromon needs

Evidence comes from a Mach-O and nib analysis of the IPA. The full report and
the exact UIKit/QuartzCore import lists (main binary and MetalANGLE) are in
`dev-docs/ARM64_UIKIT_COROMON_NEEDS.md`. BV means
verified in the binary. SO means taken from source or standard UIKit behaviour.

### Load-time facts

- The main binary uses classic dyld-info binds, not chained fixups.
- It hard-links UIKit and QuartzCore.
- It links MetalANGLE through `@rpath`. MetalANGLE imports UIView,
  UIViewController, CALayer, CAMetalLayer, CAEAGLLayer, CADisplayLink and
  CACurrentMediaTime.
- Every imported UIKit symbol must exist or binding fails. That means 36
  classes, 6 metaclasses, UIApplicationMain, the UIGraphics image-context
  functions, UIImagePNGRepresentation, the UIAccessibility functions, and
  roughly 40 NSString constants (notifications, launch-option keys, UIKeyboard
  and UIPasteboard keys, UIActivityType values, and others).

### Class chains

- AppViewController → CoronaViewController → MGLKViewController → UIViewController.
- CoronaView → MGLKView → UIView. `+layerClass` returns MGLLayer, a CALayer subclass.
- AppDelegate and CoronaAppDelegate are plain NSObject subclasses. The app sets
  them as the UIApplication delegate.

### Startup sequence (BV unless marked)

1. **Before main.** A `+load` on the UIViewController(APMScreenClassName)
   category swizzles `-viewDidAppear:` and `-viewDidDisappear:`, so both must
   be real methods.
2. **UIApplicationMain(argc, argv, nil, nil).** With a nil delegate name, UIKit
   loads **MainWindow.nib**. The nib's File's Owner is UIApplication, whose
   delegate outlet is AppDelegate. AppDelegate's window outlet is a UIWindow,
   and the nib marks that window visible.
3. **The delegate's `-application:willFinishLaunchingWithOptions:` runs,** then
   `-application:didFinishLaunchingWithOptions:`. That second call does the following:
   1. `[[AppViewController alloc] initWithNibName:nil bundle:nil]`, then
      `setWantsFullScreenLayout:`.
   2. `[vc view]` makes UIKit send `-loadView`. Corona's override does the following:
      - calls `[super loadView]`;
      - reads `[[UIScreen mainScreen] bounds]`, which must be landscape;
      - creates `[[CoronaView alloc] initWithFrame:context:]`;
      - calls `setView:`.
   3. UIKit then sends `-viewDidLoad`.
   4. `-[UIView initWithFrame:]` must create the layer from `+layerClass`
      (MGLLayer). This is a QuartzCore dependency.
   5. `setRootViewController:` and `makeKeyAndVisible`.
   6. `-didMoveToWindow` reads `[[window screen] nativeScale]` and passes it to
      `setContentScaleFactor:`. UIView must forward that to
      `-[CALayer setContentsScale:]` (SO).
   7. The run setup does the following:
      - registers for the WillResignActive and DidBecomeActive notifications;
      - draws the first frame through `[view display]`, `drawRect:` and MGLKit;
      - paces later frames with `+[CADisplayLink displayLinkWithTarget:selector:]`,
        `setPreferredFramesPerSecond:` and `addToRunLoop:forMode:`. MGLKit uses
        `CACurrentMediaTime` and never reads the link's timestamp.
4. **After didFinishLaunching,** UIKit does the following:
   - posts UIApplicationDidFinishLaunchingNotification;
   - sends `applicationDidBecomeActive:` and posts the matching notification;
   - sends `-viewDidAppear:` to the root view controller (MGLKViewController
     resumes its display link here);
   - runs the main CFRunLoop forever.
5. **Touches.** `touchesBegan/Moved/Ended/Cancelled:withEvent:` reach CoronaView
   with an NSSet of UITouch objects. Corona uses `locationInView:`, `phase`,
   `anyObject`, `count`, fast enumeration and `isMultipleTouchEnabled`.

### Ranked needs

1. UIApplicationMain, UIApplication, MainWindow.nib objects, the delegate
   callbacks and the run loop.
2. UIScreen (`mainScreen`, `bounds`, `nativeScale`), UIWindow and UIViewController
   view loading (`loadView`, `viewDidLoad`, `setView:`, `viewDidAppear:`).
3. UIView with frame/bounds, hierarchy, `contentScaleFactor`, a `+layerClass`
   layer, `didMoveToWindow`, and `display`/`drawRect:`.
4. CADisplayLink/NSRunLoop pacing, plus the lifecycle notifications through the
   genuine NSNotificationCenter.
5. UITouch/UIEvent delivery.
6. Imported-but-not-startup items: stubs only.
   - UIGraphics* and text drawing;
   - UIAlertController and UIImage;
   - safeAreaInsets and traitCollection;
   - setIdleTimerDisabled:.

## 3. Architecture

### 3.1 A synthetic UIKit Mach-O image (implemented)

`a64_uikit_image.rs` builds an `MH_DYLIB` image at a caller-chosen,
16 KiB-aligned base. Its install name is
`/System/Library/Frameworks/UIKit.framework/UIKit`. It has two segments.

**`__TEXT` (RX)**
- the header;
- the load commands: `LC_SEGMENT_64` ×2, `LC_ID_DYLIB` and a synthetic `LC_UUID`;
- the owned thunks in `__text`;
- the `__objc_methname`, `__objc_classname` and `__objc_methtype` strings.

**`__DATA` (RW)**
- `__objc_classlist` and `__objc_imageinfo` (flags 0x40);
- `__objc_selrefs`;
- a `__got` slot for `objc_msgSendSuper2`;
- `__objc_const` (`class_ro_t` and method lists with entsize 24);
- `__objc_data` (`class_t` and metaclasses);
- `__data`, which holds the static singleton instances.

The metadata is the genuine LP64 objc2 layout, so one image serves two Objective-C runtimes:

- **Emulator-owned runtime** (desktop tests and the owned-Foundation path).
  Pass `Layout.class_list` plus the root to `Registry::register`, then apply
  the returned `selector_fixups` to the image's `__objc_selrefs` slots.
- **Genuine cached libobjc.** `ObjcImage::read` already accepts the header, as
  a test proves. Add the image to the dyld ObjC "mapped" notification list
  ahead of the app image. libobjc's `map_images` then realizes the classes,
  fixes method-list and selref selectors in place (that is why everything is
  RW), and initializes each class cache. The root and empty-cache addresses
  come from the cached `_OBJC_CLASS_$_NSObject`, `_OBJC_METACLASS_$_NSObject`
  and `__objc_empty_cache` exports.

Constraints:

- **Address range.** Class addresses and the raw-isa singletons must stay below
  64 GiB, because objc4 `ISA_MASK` is `0xffffffff8` on non-ptrauth arm64. The
  builder rejects anything higher.
- **No ivar lists.** Every UIKit class has an `instance_size` of 8. objc4's
  `reconcileInstanceVariables` only slides a subclass's ivars when
  `instanceStart < super.instanceSize`. App subclasses compiled against the
  real SDK therefore keep their compiled offsets; the only cost is unused space.
  The owned heap also requires classes without ivars.

### 3.2 Dispatch: one bridge service per class

Each UIKit class gets one dispatcher service, `_touchHLE_UIKit_<Class>`. All of
that class's method IMPs point to it. The handler works in three steps:

1. Read `x1` (the SEL). A SEL is a C-string pointer in both runtimes; the
   handler caches the address-to-name mapping.
2. Find the selector in that class's static `Method` table.
3. Run a Rust handler with the arguments in AAPCS64 order: `x2..x7`, CGRect
   and CGPoint in `d0..d3`, and BOOL in the low byte.

Results go back as `ReturnValues`. A CGRect returns as an HFA in `v0..v3`.

This uses 8 services for milestone 1, plus one shared `-dealloc` cleanup service.

**Known risk: swizzling.** The app's `+load` (GULSwizzler) replaces
`-viewDidAppear:` and `-viewDidDisappear:` on our UIViewController before main.
If any swizzle uses `method_exchangeImplementations`, one of our IMPs could be
reached under a foreign selector, and name-based dispatch would then fail with
"no host method". Mitigation if that is confirmed: give each method an 8-byte
`__text` thunk (`movz x17,#index; b dispatcher`), so the service dispatches by
method index instead of by SEL name. This is not implemented yet. GULSwizzler's
mechanism has not been checked.
There are currently 7 classes, 116 table methods (plus 2 dealloc thunks) and 3 sent selectors.

### 3.3 Calling back into guest code

There are two mechanisms, and both run genuine guest code:

- **Queued calls.** `ServiceFrame::request_guest_call` runs `objc_msgSend`,
  `objc_retain`, `objc_release` or `objc_alloc_init` after the handler returns.
  This covers the retain/release of strong UIKit references (subviews,
  `rootViewController`, `backgroundColor`, the controller's view) and
  `-[UIViewController loadView]`.
- **Tail re-dispatch.** `-[UIViewController view]` queues `[self loadView]` and
  `[self viewDidLoad]`, then re-sends `view` through `objc_msgSend`, so a
  guest override of either method runs. A test proves this.
  - The SELs are read from the image's own fixed-up `__objc_selrefs` slots.
  - A missing view after `-loadView` fails explicitly instead of recursing.

### 3.4 Memory ownership

- **Instances.** Guest `+alloc` (inherited from NSObject) allocates every
  instance, so the genuine allocator or the owned heap owns the memory.
- **State.** Host state is keyed by object address in `state::Model`.
- **Disposal.** UIView and UIViewController get an owned `-dealloc` thunk. In
  compiled form it is `cleanup_service(self,_cmd); objc_msgSendSuper2(&{self,
  cls}, @selector(dealloc))`, where `@selector(dealloc)` comes from the image
  selref and the Super2 address from the `__got` slot.
  - The cleanup removes the host state and queues releases of the strong
    references the object held.
  - A test exercises the chain window → root view controller → view.
  - Current limit: at most 7 releases per dealloc (the bridge continuation
    cap). A batched release thunk comes later.
- **Singletons.** UIApplication, UIScreen and UIDevice are static, immortal
  16-byte objects in `__data` with a raw isa.

### 3.5 Run loop and events (milestone 3)

`UIApplicationMain` must never return and must run guest callbacks
indefinitely. A service handler cannot do that, because it is bounded by depth
and ticks. The design is an owned **guest-side event pump**:

1. `UIApplicationMain`'s IMP is a thunk in `__text` that loops: `bl
   next_event_service`.
2. That service writes a work record (entry, `x0..x7`, `d0..d3`) into a per-pump
   `__data` buffer and returns 1 to continue, or 0 to stop (tests only).
3. The thunk loads the registers from the record and calls `blr entry`.

The real CFRunLoop therefore lives in guest code on the guest's own main
stack. The host decides what happens next:

- the launch sequence, as a script of delegate messages;
- notification posts through the genuine NSNotificationCenter;
- CADisplayLink targets, at vsync;
- touch delivery;
- timers.

Waiting between frames happens inside the `next_event` handler. The session
owner (agent A) must allow a long-running, non-tick-limited main.

### 3.6 What stays out of UIKit

- **CALayer, CAMetalLayer, CAEAGLLayer, CADisplayLink and CACurrentMediaTime**
  belong to agent B (QuartzCore). UIKit needs the interface
  `layer_for_view(view_class) -> CALayer object` (`+layerClass` then
  alloc/init), plus `setContentsScale:`, `setFrame:` and `setBounds:` forwarding.
- **MetalANGLE.** The genuine embedded MetalANGLE subclasses *our* UIView and
  UIViewController. Its MGLLayer probes `MTLCreateSystemDefaultDevice`. Agent B
  decides between two paths: provide Metal/CAMetalLayer, or return nil so it
  falls back to the CAEAGLLayer path backed by host GLES. The other option is a
  stand-in that replaces MetalANGLE entirely. MGLKView and MGLKViewController
  would then become owned classes that subclass ours, and agent B owns them.

## 4. Extensions needed in shared files

- **`a64_objc_namespace.rs`: no change needed.** The UIKit image bypasses it
  and attaches to any existing root. The limits the researcher reported
  (64 classes, 64 KiB, 1024 methods, no ivars or categories) do not constrain
  UIKit.
- **`a64_bridge.rs`.** `MAX_SERVICES=128` per bridge, and only 255 trampolines
  fit the 4 KiB page. UIKit uses about 10 to 15 services; agent B's frameworks
  may need many more. Proposal: a second service page or one dispatcher per
  framework. The continuation cap of 8 limits per-call fan-out.
- **Binding (agents A and B).** Route the app's and MetalANGLE's
  UIKit/UIKitCore imports to `Layout::exports()` plus the C-function and
  constant export table (milestone 2). Also exclude the cached UIKitCore and its
  initializers from the dependency closure. The main binary uses classic
  dyld-info binds, so this goes through the legacy bind path.
- **UIKitCore's dependency cone (agents A and B).** Excluding UIKitCore means
  more than skipping its initializers. If libobjc ever sees UIKitCore's header,
  the cache's preoptimized class table makes `objc_getClass("UIView")` return
  Apple's class instead of ours. Coromon also hard-links GameKit, StoreKit,
  MapKit, MessageUI, WebKit, SafariServices and MediaPlayer, and each of them
  pulls in UIKitCore transitively. Every cached framework in UIKitCore's
  dependency cone must therefore be stubbed, not loaded as genuine code.
- **ObjC notification (agent A).** Append the UIKit image's `ObjcImage` to the
  mapped notification before the app and MetalANGLE.

## 5. Milestones (each with a desktop test)

1. **DONE.** Synthetic image, dispatch and core classes. Covered:
   - UIResponder, UIApplication, UIScreen, UIDevice, UIView, UIWindow and
     UIViewController;
   - geometry, hierarchy, key window and root view controller;
   - view loading with guest overrides;
   - retain/release and dealloc cleanup.

   Tests (`a64::uikit::*`):
   - image validity through `ObjcImage::read` and `Registry::register`;
   - builder rejections;
   - the state model;
   - the end-to-end guest subclass chain through the owned objc runtime,
     including the dealloc chain.
2. **C exports and constants.** Covered:
   - `UIApplicationMain` symbol, UIGraphics stubs and UIAccessibility functions;
   - static CFString constants in `__cfstring`, using an `isa` of
     `___CFConstantStringClassReference`, for every imported NSString constant;
   - stub classes for all 36 imported classes;
   - an export table consumed by the binder.

   Test: bind the real Coromon import list against the table and assert no
   missing UIKit symbols. This is a desktop test that parses the IPA, `#[ignore]`
   when the IPA is absent.
3. **UIApplicationMain pump and launch script.** Covered:
   - a synthesized MainWindow.nib object graph: delegate class from the nib,
     with the window assigned through `setWindow:`;
   - `willFinish`/`didFinish` callbacks, the notifications (through queued
     NSNotificationCenter calls), `viewDidAppear:` and `applicationDidBecomeActive:`.

   Test: a guest delegate subclass records callback order. The pump stops after
   N events.
4. **Layer integration with QuartzCore (agent B).** Covered: `+layerClass`,
   `-layer`, `contentScaleFactor` to `contentsScale`, and frame propagation.

   Test: a guest `+layerClass` override is honoured.
5. **Display link and run-loop pacing.** A CADisplayLink target fires from the
   pump, together with B's CADisplayLink. Test: the virtual clock fires 3 frames.
6. **Touches.** UITouch and UIEvent objects, NSSet delivery through queued calls,
   and `locationInView:` using the view-hierarchy transform. Test: synthetic host
   touch to a guest `touchesBegan:` override.
7. **Genuine-libobjc integration.** The image is registered by the cached
   libobjc in the cache-session harness. Test: the native `actual_ipa_regression`
   probe reaches `-application:didFinishLaunchingWithOptions:`. This needs
   agent A's initializer progress.

## 6. Dependencies on the other agents

- **Agent A (initializers and session).**
  - libobjc and Foundation initialized;
  - the mapped notification includes the UIKit image;
  - binds are redirected;
  - UIKitCore is excluded;
  - a non-tick-limited main for the pump;
  - a `--a64-*` probe that calls `uikit::install` with a free base below
    64 GiB.
- **Agent B (frameworks).**
  - CALayer and its subclasses, CADisplayLink and CACurrentMediaTime;
  - the Metal-or-EAGL decision for MetalANGLE;
  - UIGraphics drawing helpers if the CoreGraphics context is shared;
  - NSString and NSArray creation for UIKit returns (for example `windows`,
    `systemVersion`), if agent B or A owns Foundation bridging.
