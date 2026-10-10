# ARM64 UIKit layer plan (Coromon 1.4.1 first)

Status (2026-10-10): milestones 1, 3, 4 and part of 5 pass desktop tests. Nothing has run under genuine libobjc or on a device.
Owner: UIKit agent. Files: `src/a64_uikit*.rs` (core, image, asm, state, quartz,
mgl, app, tests). Shared-file hook: a single
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

### 3.1 A synthetic Mach-O image (implemented)

`a64_uikit_image.rs` builds one `MH_DYLIB` image at a caller-chosen base
(16 KiB aligned). Its install name is
`/System/Library/Frameworks/UIKit.framework/UIKit`. It holds the UIKit
classes, the QuartzCore `CALayer`, and the MetalANGLE stand-in classes. Each
class records its provider (UIKit, QuartzCore or `@rpath/MetalANGLE...`), so
`Layout::exports()` yields `(provider, symbol, address)` entries for the binder.

**`__TEXT` (RX)**
- the header;
- the load commands: `LC_SEGMENT_64` ×2, `LC_ID_DYLIB` and a synthetic `LC_UUID`;
- `__text`, which holds:
  - a hub (`ldr x16,=service; br x16`);
  - one 8-byte trampoline per method (`movz x17,#index; b hub`);
  - the generated functions (`_UIApplicationMain`, `_touchHLE_UIKit_createLayer`);
  - the owned `-dealloc` thunks;
- `__objc_methname`, `__objc_classname` and `__objc_methtype`.

**`__DATA` (RW)**
- `__objc_classlist` and `__objc_imageinfo` (flags 0x40);
- `__objc_selrefs`, for every selector the host or generated code sends;
- `__got`, the bound slots for `objc_msgSend`, `objc_msgSendSuper2` and
  `objc_alloc_init`, written by `UiKit::link`;
- `__objc_const` (`class_ro_t` and method lists with entsize 24);
- `__objc_data` (`class_t` and metaclasses);
- `__data`: the static singletons, plus a 4 KiB scratch area used by the pump
  record and the class-name string buffer.

`a64_uikit_asm.rs` is a small AArch64 assembler for the generated code. Each
encoding it uses was checked against capstone; its tests pin the exact words.

The metadata is the genuine LP64 objc2 layout, so one image serves two
Objective-C runtimes:

- **The emulator-owned runtime** (desktop tests and the owned-Foundation path).
  Pass `Layout.class_list` plus the root to `Registry::register`, then apply
  `selector_fixups` to the image's `__objc_selrefs`.
- **The genuine cached libobjc.** A test proves that `ObjcImage::read` accepts
  the header. Put the image in the dyld ObjC "mapped" notification ahead of the
  app image. libobjc's `map_images` then realizes the classes and fixes
  selectors in place, which is why everything is RW. The root comes from the
  cached `_OBJC_CLASS_$_NSObject` and `_OBJC_METACLASS_$_NSObject`, and the
  empty cache from `__objc_empty_cache`.

**Constraints**
- Classes and raw-isa singletons stay below 64 GiB. objc4's `ISA_MASK` on
  non-ptrauth arm64 is `0xffffffff8`; the same mask is applied in the
  layer-creation routine.
- No ivar lists: every class has `instance_size` 8. objc4's
  `reconcileInstanceVariables` only moves a subclass's ivars when
  `instanceStart < super.instanceSize`, so app subclasses compiled against the
  real SDK keep their compiled offsets. The owned heap also requires classes
  without ivars.

### 3.2 Dispatch: one bridge service and an index per method (implemented)

Every method IMP is its own trampoline. It loads its index into x17 and
branches to the one service, `_touchHLE_UIKit`. The service reads the index
with `ServiceFrame::dispatch_index()`, the frameworks agent's existing x17 hook.

The first four indices are internal entries:

| Index | Entry |
| --- | --- |
| 0 | dealloc cleanup |
| 1 | pump start |
| 2 | pump next |
| 3 | attach layer |

The other indices map to a `(class, Method)` table entry. The handler receives
the arguments in AAPCS64 order: `x2..x7`, CGRect/CGPoint in `d0..d3`, float in
`s0`, and BOOL in the low byte. Results go back in `ReturnValues`; a CGRect is
returned as an HFA in `v0..v3`.

- **Bridge cost:** the whole layer uses one bridge slot.
- **Current size:** 12 classes, about 260 methods.
- **Swizzling is no longer a risk.** Dispatch follows the IMP, not the
  selector name, so `method_exchangeImplementations` (Coromon's GULSwizzler
  swizzles `viewDidAppear:`) behaves as it does on iOS.

### 3.3 Calling back into guest code (implemented)

- **Queued calls.** `ServiceFrame::request_guest_call` runs `objc_msgSend`,
  `objc_retain`/`objc_release` and `objc_alloc_init` after the handler
  returns. Completions reach the UIKit state through a `Weak` reference.
- **Tail re-dispatch.** `-[UIViewController view]` and `-[UIView layer]` queue
  their work, then re-send themselves, so guest overrides of `-loadView`,
  `-viewDidLoad` and `+layerClass` run.
- **Generated guest routines.** `_touchHLE_UIKit_createLayer` performs
  `[[object_getClass(view) layerClass] alloc] init` in guest code. The
  `UIApplicationMain` pump is also generated code.

### 3.4 Memory ownership (implemented)

- **Instances.** Instances come from guest `+alloc`. Host state is keyed by
  address (`state::Model`, `mgl::MglState`).
- **Disposal.** UIView, UIViewController, CALayer and MGLContext get owned
  `-dealloc` thunks: `cleanup(self,_cmd); objc_msgSendSuper2(&{self, cls},
  @selector(dealloc))`.
  - Cleanup releases the strong references the object held: subviews, the root
    view controller, the background color, the backing layer, sublayers, the
    controller's view, and an MGLKView's context.
  - Limit: at most 7 releases per dealloc (the bridge continuation cap).
- **Singletons.** UIApplication, UIScreen and UIDevice are static, immortal
  objects with a raw isa.

### 3.5 UIApplicationMain and the run loop (implemented, desktop)

`_UIApplicationMain` is a generated guest loop:

1. Call the start entry.
2. Call the next entry; stop if it returns 0.
3. Load `x0..x7`, `d0..d3` and the entry address from the scratch record.
4. Call the entry (`blr`), store `x0`/`x1`/`d0` back into the record, and repeat.

Every callback is therefore a genuine guest call on the guest's main stack, and
the host only decides the next call (`app::Launcher`).

**Launch sequence.** This is UIKit's sequence for
`UIApplicationMain(argc, argv, nil, nil)`, driven by a main-nib launch plan:

1. `objc_getClass` on the nib's delegate class, then `objc_alloc_init` it.
   `UIApplication.delegate` is set.
2. `objc_alloc_init` the window, giving screen bounds (`UIResizesToFullScreen`).
3. `-setWindow:` connects the outlet.
4. `respondsToSelector:` is checked, then
   `application:willFinishLaunchingWithOptions:` is called.
5. The same check and call for `application:didFinishLaunchingWithOptions:`.
6. The nib's visible window becomes key, and the application state becomes Active.
7. `applicationDidBecomeActive:` is called if the delegate responds.
8. `viewWillAppear:` and `viewDidAppear:` go to the key window's root view controller.
9. Frames: each resumed MGLKViewController receives `frameStep` once per
   frame. The virtual media clock advances 1/60 s per frame.

**Main-nib launch plan.** `app::parse_nib` reads compiled NIBArchive files.
Applied to the real Coromon `MainWindow.nib`, it gives delegate `AppDelegate`,
window `UIWindow`, visible. That is an `#[ignore]`d test that needs
`PLAYCOVER_COROMON_NIB`.

**Not done yet**
- **Notifications.** Nothing is posted: DidFinishLaunching and DidBecomeActive
  need the milestone 2 NSString constants. The launch log records this.
- **Wall-clock pacing.** There is no real vsync wait yet.
- **Idle stops.** With no frame source, the pump stops and logs "idle". On a
  device it must wait for touch or timer events instead, which needs a
  long-running, non-tick-limited main from agent A.

### 3.6 CALayer and the MetalANGLE decision (implemented)

**CALayer** is owned here (QuartzCore provider). It is a host record holding
geometry, `contentsScale`, opacity, hierarchy and delegate. A view's backing
layer is created at `-initWithFrame:`/`-init` through the guest
`+layerClass`, so guest overrides are honoured (tested). The layer's bounds,
position, scale, hidden and opaque state follow the view.

**MetalANGLE decision: replace it with a stand-in; neither Metal nor EAGL.**
MGLLayer selects CAMetalLayer with an MTLDevice, or its legacy GL backend
(CAEAGLLayer plus EAGLContext from the cached OpenGLES.framework). Both paths
need GPU kernel services (IOGPU and IOSurface) that this runtime does not have.

Coromon imports exactly three MetalANGLE classes (MGLContext, MGLKView,
MGLKViewController) and 72 `gl*` functions. So:

- MGLContext, MGLKView, MGLKViewController and MGLLayer are owned classes,
  following MetalANGLE's MGLKit sources. MGLKView subclasses our UIView and
  MGLKViewController subclasses our UIViewController.
- `-display` makes the context current on the view's MGLLayer surface, sends the
  guest `-drawRect:` (CoronaView overrides it) and then presents.
- Resume and pause register the controller as a frame target.
- The bundled MetalANGLE binary must not be loaded.
- **Interface with agent B:** `mgl::GlesHost`, which covers create/destroy
  context, make-current with a surface, bind default framebuffer, and present.
  Agent B implements it over host GLES, together with the `gl*` family (its
  M7). `RecordingGles` is the desktop test double.

**CADisplayLink objects** are not implemented. MGLKViewController's frame
loop is driven directly. Corona's separate CADisplayLink/NSTimer timer is not
on the startup path, according to the needs analysis.

## 4. Extensions needed in shared files

- **`a64_objc_namespace.rs`:** no change.
- **`a64_bridge.rs`:** no change. UIKit uses one slot and reuses the
  frameworks agent's `dispatch_index()`.
- **Binding (agent A's loader).** Route the app's UIKit, QuartzCore and
  MetalANGLE imports to `Layout::exports()`.
  - The constants come later, in milestone 2.
  - Exclude the cached UIKitCore and the bundled MetalANGLE from the image
    closure.
  - The main binary uses classic dyld-info binds, so this goes through the
    legacy bind path.
- **UIKitCore's dependency cone.** If libobjc sees UIKitCore's header, the
  cache's preoptimized class table makes `objc_getClass("UIView")` return
  Apple's class. GameKit, StoreKit, MapKit, MessageUI, WebKit, SafariServices
  and MediaPlayer all pull UIKitCore in, so they must be stubbed, not loaded as
  genuine code.
- **ObjC notification (agent A).** Append the image's `ObjcImage` ahead of the
  app.

### 4.1 Bind routing (ready in UIKit files, not yet wired)

`a64_uikit_bind.rs`: `Routes::route(provider, symbol)` resolves imports from the
owned providers (UIKit, QuartzCore, `@rpath/MetalANGLE`) to image addresses,
and `Routes::coverage` classifies an import list. A miss for an owned provider
is a real gap: the loader must fail, or weak-zero the import, and never fall
back to the cached library.

Coromon's real main-binary imports were checked on 2026-10-10 (ignored test,
needs `PLAYCOVER_COROMON_BINARY`):

| Result | Count | What it covers |
| --- | --- | --- |
| Routed | 16 | UIApplicationMain, the core UIKit classes and metaclasses, and the 3 MGL classes |
| Missing (required) | 143 | 72 MetalANGLE `gl*` functions (frameworks gles family), plus 71 UIKit/QuartzCore items (see below) |
| Missing (weak) | 5 | |

The 71 UIKit/QuartzCore items are:
- classes: UIColor, UIFont, UIImage, UITouch, UIControl, UIAlertController, UIGestureRecognizer and others;
- `CADisplayLink` and `CACurrentMediaTime`;
- the notification and key NSString constants;
- the `UIGraphics*`, `UIImagePNGRepresentation` and `UIAccessibility*` functions.

These 71 items are the milestone 2 work list.

The loader hook stays a few marked lines in agent A's bind path. Rule: for a
provider in `OWNED_PROVIDERS`, call `route()` before resolving against the
cache. The hook has not been added, because agent A had not landed its dyld
and loader changes as of playcover/main bbf45ca6.

## 5. Milestones (each with a desktop test)

1. **DONE.** Synthetic image, dispatch, core classes, view loading,
   retain/release and dealloc.
2. **C exports and constants: next.** Covered:
   - stubs for the UIGraphics and UIAccessibility functions;
   - static CFString constants (isa `___CFConstantStringClassReference`) for
     every imported NSString constant;
   - stub classes for all 36 imported classes;
   - posting launch notifications through genuine NSNotificationCenter.

   Test: bind Coromon's real import list (from the IPA, ignored when it is
   absent) and assert that no UIKit, QuartzCore or MetalANGLE symbol is missing.
3. **DONE (desktop): UIApplicationMain pump and nib launch.** Test: a guest
   delegate records callback order and receives the window outlet.
4. **DONE (desktop): CALayer and `+layerClass`.** Test: a guest `+layerClass`
   override is honoured, and geometry and scale propagate.
5. **Partly done: frame pacing.** MGLKViewController frames from the pump with
   a virtual clock (3-frame test).
   - Real vsync wait: open.
   - CADisplayLink objects: open.
6. **Touches.** Open: UITouch/UIEvent, NSSet delivery, and `locationInView:`.
7. **Genuine-libobjc integration.** The cached libobjc registers the image in
   the cache-session harness. Needs agent A's initializers.
8. **Host GLES.** A `GlesHost` implementation over the device GL (agent B's
   `gles` family). It renders on the tablet.

## 6. Dependencies on the other agents

- **Agent A (initializers and loader).**
  - libobjc and Foundation initialized;
  - mapped notification includes the image;
  - bind routing to `Layout::exports()`;
  - UIKitCore and MetalANGLE excluded;
  - a non-tick-limited main for the pump;
  - a probe that calls `uikit::install` with a free base below 64 GiB.
- **Agent B (frameworks).**
  - `GlesHost` plus `gl*` forwarding to host GLES;
  - CoreGraphics for the UIGraphics text path;
  - NSString/NSArray creation for UIKit returns (`windows`, `systemVersion`)
    if agent B owns that Foundation bridging.
