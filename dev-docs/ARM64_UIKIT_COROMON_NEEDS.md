# Coromon 1.4.1 UIKit/QuartzCore needs (raw evidence)

Generated from the IPA by Mach-O/nib analysis scripts (session scratchpad
`uikit_needs/`: macho_objc.py, nibdump.py, xref.py). Summary and ranking are
in ARM64_UIKIT_PLAN.md section 2. No binary contents beyond symbol names.

## Analysis report

```text
Coromon 1.4.1 (Corona 3722, arm64 thin, classic dyld-info, not chained fixups) -- UIKit/QuartzCore needs at startup
BV = binary-verified (selector/class/symbol present AND call site seen in disassembly of the named method), SO = source-only/inferred.
IMPORTANT: runtime-sources/solar2d-platform is a NEWER, modified fork (scene lifecycle, "iOS 27 SDK" comments, loadWindowFromNib,
showSplashScreen, statusBarOrientation, UIScreen.scale...). None of these exist in the binary. Binary wins everywhere below.

(a) Deps: Coromon LC_LOAD: AudioToolbox AVFoundation CFNetwork CoreGraphics CoreMedia CoreMotion CoreText GameKit GLKit Foundation
ImageIO MapKit MediaPlayer MessageUI MobileCoreServices OpenAL QuartzCore Security StoreKit SystemConfiguration UIKit WebKit
SafariServices LocalAuthentication CoreFoundation CoreLocation CoreTelephony DeviceCheck libobjc libsqlite3 libc++ libz libSystem
@rpath/MetalANGLE @rpath/libswiftCore/Darwin/Dispatch/ObjectiveC/Foundation. WEAK: GameController AssetsLibrary Photos Twitter SwiftUI
AuthenticationServices Social CloudKit FileProvider libswiftDataDetection/OSLog/UniformTypeIdentifiers, @rpath libswiftCoreFoundation/
CoreImage/Metal/QuartzCore/os/UIKit. MetalANGLE: QuartzCore Metal Foundation CoreFoundation UIKit CoreGraphics libobjc libc++ libSystem,
WEAK OpenGLES.

(b) App classes over UIKit/MGLKit: AppViewController : CoronaViewController : MGLKViewController : UIViewController;
CoronaView : MGLKView : UIView (MGLKView +layerClass = MGLLayer : CALayer); CoronaNullGestureRecognizer : UIGestureRecognizer (never
added: no addGestureRecognizer: selref); Rtt_AVPlayerView, Rtt_iOSWebViewContainer : UIView; CustomAlertController : UIAlertController;
GIDActivityIndicatorViewController, appleSignDel, gameCenterHandle : UIViewController; GIDSignInButton : UIControl; googleDel : UIResponder.
AppDelegate and CoronaAppDelegate are NSObject subclasses. Categories on UIKit: UITouch(CoronaViewExtensions) locationInCoronaView:,
UIViewController(APMScreenClassName) (+load!), UIWindow(APMScreenClassName), UIImage(GIDAdditions_Private).

(e) NIB REQUIRED. main() = NSAutoreleasePool alloc_init; UIApplicationMain(argc, argv, nil, nil) (BV: x2=x3=0). Info.plist NSMainNibFile=
MainWindow (NSMainNibFile~ipad=MainWindow-iPad). Decoded MainWindow.nib (NIBArchive, 18 objects): File's Owner proxy (=UIApplication),
UIClassSwapper{UIClassName=AppDelegate, original UICustomObject}, UIWindow{UIBounds 0,0,320,480 (iPad 768x1024), UIResizesToFullScreen=1,
UIAutoresizingMask=12, opaque}; outlets: Owner.delegate -> AppDelegate, AppDelegate.window -> UIWindow; UINibVisibleWindowsKey=[UIWindow].
Without it the delegate is nil and window ivar nil -> nothing renders. Emulator may parse this NIBArchive or synthesize: [[AppDelegate
alloc] init]; app.delegate=it; w=[[UIWindow alloc] initWithFrame:screenBounds]; [delegate setWindow:w]. UILaunchStoryboardName=LaunchScreen
but no LaunchScreen.storyboardc in the IPA (system-rendered anyway; ignore). UIStatusBarHidden=YES, orientations LandscapeRight/Left.

(c) Required UIKit/QuartzCore, in first-use order.
 Tier 1 (load-time): every non-lazy bind must resolve or dyld fails: all classes/metaclasses/constants in (d).
 Pre-main +load (BV): UIViewController(APMScreenClassName)+load -> GULSwizzler class_replaceMethod on -[UIViewController viewDidAppear:]
  and viewDidDisappear: (UIViewController must have real IMPs); GTMSessionFetcher/UploadFetcher +load and APMMeasurement +load
  -> NSNotificationCenter addObserver for UIApplicationDidFinishLaunchingNotification. FIRApp registerInternalLibrary (no UIKit).
 1 UIApplicationMain: create UIApplication (sharedApplication, delegate/setDelegate:), load nib as above (AppDelegate init->initSelf,
  UIWindow initWithCoder:/initWithFrame:, setWindow:).  [nib content BV; UIKit internals SO]
 2 -[AppDelegate application:willFinishLaunchingWithOptions:] (BV): +[CoronaAppDelegate handlesUrl:] on launchOptions[UIApplicationLaunchOptionsURLKey];
  CoronaAppDelegate initWithEnterpriseDelegate: reads NSBundle infoDictionary "CoronaDelegates", NSClassFromString each
  (gamecenterDel: class absent -> nil; universalLinksDelegate, googleSignInDel: plain NSObject init).
 3 -[AppDelegate application:didFinishLaunchingWithOptions:] -> initializeWithApplication:options: (BV, order):
  a [[AppViewController alloc] initWithNibName:nil bundle:nil] -> UIViewController initWithNibName:bundle: (super), MGLKViewController constructor.
  b setViewController:, [vc setWantsFullScreenLayout:YES] (deprecated, must exist as no-op).
  c [vc view] -> UIKit must call loadView. -[CoronaViewController loadView]: nibName, [super loadView] (msgSendSuper2),
    [[UIScreen mainScreen] bounds] (must be LANDSCAPE W>H), MGLContext initWithAPI:, [[CoronaView alloc] initWithFrame:context:]
    -> MGLKView initWithFrame: -> UIView initWithFrame: (must instantiate +layerClass = MGLLayer, alloc/init -> constructor ->
    CAMetalLayer or CALayer sublayer, addSublayer:, setFrame:), setAutoresizingMask:, setEnableSetNeedsDisplay:; setView: (MGLKViewController
    setView: -> super setView:, isKindOfClass, view setDelegate:/setController:). Then UIKit must call viewDidLoad: CoronaViewController
    viewDidLoad -> MGLKViewController viewDidLoad -> UIViewController viewDidLoad; setContext:, setDrawableDepthFormat:.
  d [view setAutoresizingMask:18], [window setRootViewController:vc] (UIKit adds vc.view to window, sizes it to window bounds ->
    CoronaView setFrame:/setBounds: overrides, UIView layer frame sync), [window makeKeyAndVisible].
    CoronaView didMoveToWindow (BV): window, screen, respondsToSelector:nativeScale, [[window screen] nativeScale], setContentScaleFactor:
    -> UIView must forward to layer setContentsScale: (MGLLayer overrides; drawableSize = bounds x contentsScale) (SO for UIKit behaviour).
  e initializeRuntimeWithApplication -> runView:withPath:parameters: (BV): CoronaView initializeRuntimeWithPlatform:runtimeDelegate:,
    runWithPath:parameters: -> addApplicationObserver (NSNotificationCenter UIApplicationWillResignActive/DidBecomeActive),
    [MGLContext setCurrentContext:], bindDrawable (MGLLayer surface creation), beginRunLoop -> willBeginRunLoop:, [view display]
    (MGLKView display -> displayAndCapture: -> [self bounds], drawRect: -> CoronaView drawView -> first frame, present).
    IPhoneScreenSurface: view contentScaleFactor, bounds x4 (BV region, stripped). [UIApplication sharedApplication], launchOptions objectForKey:.
  f Frame pacing: Rtt::IPhoneTimer -> vc.preferredFramesPerSecond= (BV, 4 sites) / isPaused -> MGLKViewController setPreferredFramesPerSecond:
    -> pause/resume -> [CADisplayLink displayLinkWithTarget:self selector:@selector(frameStep)], setPreferredFramesPerSecond:,
    addToRunLoop:[NSRunLoop mainRunLoop] forMode:NSDefaultRunLoopMode; pause = removeFromRunLoop:forMode:. Needs _glView (set in setView:),
    NOT viewDidAppear:. frameStep: CACurrentMediaTime, update (delegate nil), [glView display]. No timestamp/duration used.
 4 After delegate returns: post UIApplicationDidFinishLaunchingNotification (GTM/APM observers), make nib visible windows visible,
  applicationDidBecomeActive: + UIApplicationDidBecomeActiveNotification (CoronaView resume, MGLK appDidBecomeActive:), call
  viewWillAppear:/viewDidAppear: on root VC (MGLKViewController viewDidAppear: -> resume + registers UIApplicationWillResignActive/
  DidBecomeActive observers; APM swizzle -> APMScreenViewReporter registers more observers). Then spin main CFRunLoop forever.
 5 Touches (BV): UIKit sends touchesBegan/Moved/Ended/Cancelled:withEvent: to CoronaView. Uses NSSet anyObject/count/fast-enum,
  UITouch locationInView: (+ view contentScaleFactor), phase, view isMultipleTouchEnabled (setMultipleTouchEnabled: only via Lua).
  No tapCount/force/previousLocationInView/timestamp on touches in binary. pressesBegan/Ended exist (UIPress) - optional.
 Later/optional (BV selrefs, not on boot path): setIdleTimerDisabled:/isIdleTimerDisabled, safeAreaInsets, traitCollection/userInterfaceStyle,
  UIDevice currentDevice model/systemVersion/identifierForVendor/userInterfaceIdiom, setStatusBarHidden:withAnimation:,
  UIAccelerometer sharedAccelerometer/setUpdateInterval:/setDelegate:, keyWindow/connectedScenes (Firebase/GID), UIAlertController,
  canOpenURL:/openURL:options:completionHandler:, registerUserNotificationSettings:, text: UIGraphicsBeginImageContextWithOptions,
  NSString drawAtPoint:withAttributes:/boundingRectWithSize:options:attributes:context:, UIFont fontWithName:size:/boldSystemFontOfSize:,
  UIColor colorWithRed:green:blue:alpha:/whiteColor/clearColor, UIImage imageWithContentsOfFile:/CGImage, UIImagePNGRepresentation.
 AppViewController: supportedInterfaceOrientations=0x18, application:supportedInterfaceOrientationsForWindow:=0x18, prefersStatusBarHidden=YES (BV).
 Absent from binary (source-only, NOT needed): UIScreen scale, statusBarOrientation/statusBarFrame, UIDevice orientation,
 loadNibNamed:owner:options:, addGestureRecognizer:, layoutSubviews, tapCount, force.

(d) Imports (all must exist). Coromon UIKit classes: UIAccelerometer UIActivityIndicatorView UIActivityViewController UIAlertAction
UIAlertController(+meta) UIAlertView UIApplication UIColor UIControl(+meta) UIDevice UIFont UIGestureRecognizer(+meta) UIImage UIImageView
UIImpactFeedbackGenerator UINavigationController UINotificationFeedbackGenerator UIPageViewController UIPasteboard UIPopoverController
UIResponder(+meta) UIScene UIScreen UIScrollView UISelectionFeedbackGenerator UITabBarController UITouch UIView(+meta)
UIViewController(+meta) UIWindow UIWindowScene. Functions: UIApplicationMain UIGraphicsBeginImageContextWithOptions UIGraphicsEndImageContext
UIGraphicsGetCurrentContext UIGraphicsGetImageFromCurrentImageContext UIImagePNGRepresentation UIAccessibilityIsGuidedAccessEnabled/
IsReduceMotionEnabled/IsReduceTransparencyEnabled/ShouldDifferentiateWithoutColor. Constants: UIApplicationDidBecomeActive/DidEnterBackground/
DidFinishLaunching/WillEnterForeground/WillResignActive Notification, UIApplicationLaunchOptionsURLKey, ...UserActivityDictionaryKey,
UIApplicationOpenSettingsURLString, UIBackgroundTaskInvalid, UIKeyboardBoundsUserInfoKey, UIKeyboardDidHide/DidShowNotification,
UIPasteboardNameGeneral, UIPasteboardTypeListImage/String/URL, UISceneWillConnectNotification, UIWindowDidResignKeyNotification,
UIWindowLevelAlert, UIAccessibilityTraitButton, NSFontAttributeName, NSForegroundColorAttributeName, 16 UIActivityType* strings.
QuartzCore: CADisplayLink, CACurrentMediaTime. libswiftUIKit (bundled, weak): UIActionSheet UIAlertView UIColor UIFocusSystem UIFontMetrics
UIImage UIScreen UIView, UIApplicationMain, UIContentSizeCategoryCompareToCategory/IsAccessibilityCategory, UIGraphics* (4).
21 SwiftUI symbols are weak (may be NULL). No kCAEAGLDrawableProperty*/kEAGL* imports anywhere.

(f) MetalANGLE: UIView/UIViewController(+meta), UIImage (snapshot only), CALayer(+meta), CAMetalLayer, CAEAGLLayer, CADisplayLink,
CACurrentMediaTime, UIApplicationDidBecomeActive/WillResignActiveNotification, weak EAGLContext, MTLCreateSystemDefaultDevice + MTL*
descriptors. MGLLayer constructor branches on rx::IsMetalDisplayAvailable() (MTLCreateSystemDefaultDevice):
 Metal path: [[CAMetalLayer alloc] init], setFrame:/bounds/frame, addSublayer:, setDevice:, setPixelFormat:, setFramebufferOnly:,
  setContentsScale:/contentsScale, setDrawableSize:/drawableSize, setAllowsNextDrawableTimeout:, nextDrawable -> drawable.texture,
  removeFromSuperlayer (WindowSurfaceMtl, BV).
 Fallback: plain CALayer sublayer + WindowSurfaceEAGL creates CAEAGLLayer, EAGLContext initWithAPI:/initWithAPI:sharegroup:/
  setCurrentContext:, renderbufferStorage:fromDrawable:, presentRenderbuffer:, contentsScale, frame (BV).
 Emulator must make one path work; the other path's classes must still exist. MGLLayer.checkLayerSize reads bounds/contentsScale every
 present; drawableSize = bounds*contentsScale.

(g) Scripts (scratchpad\uikit_needs): macho_objc.py (Mach-O deps/binds/ObjC classes/classrefs/selrefs; dyld-info + chained),
nibdump.py (NIBArchive decoder), dis_main.py (capstone disasm of main/ObjC methods with stub/selref annotation), xref.py (linear-sweep
reference index, labels = nearest preceding symbol/IMP), corona_funcs.py, check_sels.py. Outputs: cor_*.txt, mgl_*.txt (deps, imports,
classes, classrefs, superrefs, selrefs, xref), libswift*_imports.txt, MainWindow_nib.txt, startup_dis.txt, startup_calls.txt,
mgl_mglkit_calls.txt, uikit_selector_check.txt, cor_corona_funcs.txt.
Caveats: Corona C++ (Rtt::IPhone*) is stripped, so xref labels there are wrong (e.g. CADisplayLink/setFrameInterval:/NSTimer block
labelled -[ReachabilityCallbackDelegate setNetworkReachability:] is really Rtt_AppleTimer, likely not on boot path; the
setPreferredFramesPerSecond: block labelled CoronaViewController setContext: is IPhoneTimer). UIKit-internal behaviour (when loadView/
viewDidLoad/viewDidAppear/didMoveToWindow fire, nib outlet KVC) is from Apple semantics, not verified here. Firebase configure path
after launch and Lua-driven calls (display.newText, system.* APIs) not traced.
```

## Coromon main binary: UIKit/QuartzCore imports (87)

```text
_CACurrentMediaTime                                          QuartzCore                     uses=1
_OBJC_CLASS_$_CADisplayLink                                  QuartzCore                     uses=1
_NSFontAttributeName                                         UIKit                          uses=1
_NSForegroundColorAttributeName                              UIKit                          uses=1
_OBJC_CLASS_$_UIAccelerometer                                UIKit                          uses=1
_OBJC_CLASS_$_UIActivityIndicatorView                        UIKit                          uses=1
_OBJC_CLASS_$_UIActivityViewController                       UIKit                          uses=1
_OBJC_CLASS_$_UIAlertAction                                  UIKit                          uses=1
_OBJC_CLASS_$_UIAlertController                              UIKit                          uses=2
_OBJC_CLASS_$_UIAlertView                                    UIKit                          uses=1
_OBJC_CLASS_$_UIApplication                                  UIKit                          uses=1
_OBJC_CLASS_$_UIColor                                        UIKit                          uses=1
_OBJC_CLASS_$_UIControl                                      UIKit                          uses=1
_OBJC_CLASS_$_UIDevice                                       UIKit                          uses=1
_OBJC_CLASS_$_UIFont                                         UIKit                          uses=1
_OBJC_CLASS_$_UIGestureRecognizer                            UIKit                          uses=1
_OBJC_CLASS_$_UIImage                                        UIKit                          uses=2
_OBJC_CLASS_$_UIImageView                                    UIKit                          uses=1
_OBJC_CLASS_$_UIImpactFeedbackGenerator                      UIKit                          uses=1
_OBJC_CLASS_$_UINavigationController                         UIKit                          uses=1
_OBJC_CLASS_$_UINotificationFeedbackGenerator                UIKit                          uses=1
_OBJC_CLASS_$_UIPageViewController                           UIKit                          uses=1
_OBJC_CLASS_$_UIPasteboard                                   UIKit                          uses=1
_OBJC_CLASS_$_UIPopoverController                            UIKit                          uses=1
_OBJC_CLASS_$_UIResponder                                    UIKit                          uses=1
_OBJC_CLASS_$_UIScene                                        UIKit                          uses=1
_OBJC_CLASS_$_UIScreen                                       UIKit                          uses=1
_OBJC_CLASS_$_UIScrollView                                   UIKit                          uses=1
_OBJC_CLASS_$_UISelectionFeedbackGenerator                   UIKit                          uses=1
_OBJC_CLASS_$_UITabBarController                             UIKit                          uses=1
_OBJC_CLASS_$_UITouch                                        UIKit                          uses=1
_OBJC_CLASS_$_UIView                                         UIKit                          uses=3
_OBJC_CLASS_$_UIViewController                               UIKit                          uses=5
_OBJC_CLASS_$_UIWindow                                       UIKit                          uses=2
_OBJC_CLASS_$_UIWindowScene                                  UIKit                          uses=1
_OBJC_METACLASS_$_UIAlertController                          UIKit                          uses=1
_OBJC_METACLASS_$_UIControl                                  UIKit                          uses=1
_OBJC_METACLASS_$_UIGestureRecognizer                        UIKit                          uses=1
_OBJC_METACLASS_$_UIResponder                                UIKit                          uses=1
_OBJC_METACLASS_$_UIView                                     UIKit                          uses=2
_OBJC_METACLASS_$_UIViewController                           UIKit                          uses=3
_UIAccessibilityIsGuidedAccessEnabled                        UIKit                          uses=1
_UIAccessibilityIsReduceMotionEnabled                        UIKit                          uses=1
_UIAccessibilityIsReduceTransparencyEnabled                  UIKit                          uses=1
_UIAccessibilityShouldDifferentiateWithoutColor              UIKit                          uses=1
_UIAccessibilityTraitButton                                  UIKit                          uses=1
_UIActivityTypeAddToReadingList                              UIKit                          uses=1
_UIActivityTypeAirDrop                                       UIKit                          uses=1
_UIActivityTypeAssignToContact                               UIKit                          uses=1
_UIActivityTypeCopyToPasteboard                              UIKit                          uses=1
_UIActivityTypeMail                                          UIKit                          uses=1
_UIActivityTypeMarkupAsPDF                                   UIKit                          uses=1
_UIActivityTypeMessage                                       UIKit                          uses=1
_UIActivityTypeOpenInIBooks                                  UIKit                          uses=1
_UIActivityTypePostToFacebook                                UIKit                          uses=1
_UIActivityTypePostToFlickr                                  UIKit                          uses=1
_UIActivityTypePostToTencentWeibo                            UIKit                          uses=1
_UIActivityTypePostToTwitter                                 UIKit                          uses=1
_UIActivityTypePostToVimeo                                   UIKit                          uses=1
_UIActivityTypePostToWeibo                                   UIKit                          uses=1
_UIActivityTypePrint                                         UIKit                          uses=1
_UIActivityTypeSaveToCameraRoll                              UIKit                          uses=1
_UIApplicationDidBecomeActiveNotification                    UIKit                          uses=1
_UIApplicationDidEnterBackgroundNotification                 UIKit                          uses=1
_UIApplicationDidFinishLaunchingNotification                 UIKit                          uses=1
_UIApplicationLaunchOptionsURLKey                            UIKit                          uses=1
_UIApplicationLaunchOptionsUserActivityDictionaryKey         UIKit                          uses=1
_UIApplicationMain                                           UIKit                          uses=1
_UIApplicationOpenSettingsURLString                          UIKit                          uses=1
_UIApplicationWillEnterForegroundNotification                UIKit                          uses=1
_UIApplicationWillResignActiveNotification                   UIKit                          uses=1
_UIBackgroundTaskInvalid                                     UIKit                          uses=1
_UIGraphicsBeginImageContextWithOptions                      UIKit                          uses=1
_UIGraphicsEndImageContext                                   UIKit                          uses=1
_UIGraphicsGetCurrentContext                                 UIKit                          uses=1
_UIGraphicsGetImageFromCurrentImageContext                   UIKit                          uses=1
_UIImagePNGRepresentation                                    UIKit                          uses=1
_UIKeyboardBoundsUserInfoKey                                 UIKit                          uses=1
_UIKeyboardDidHideNotification                               UIKit                          uses=1
_UIKeyboardDidShowNotification                               UIKit                          uses=1
_UIPasteboardNameGeneral                                     UIKit                          uses=1
_UIPasteboardTypeListImage                                   UIKit                          uses=1
_UIPasteboardTypeListString                                  UIKit                          uses=1
_UIPasteboardTypeListURL                                     UIKit                          uses=1
_UISceneWillConnectNotification                              UIKit                          uses=1
_UIWindowDidResignKeyNotification                            UIKit                          uses=1
_UIWindowLevelAlert                                          UIKit                          uses=1
```

## MetalANGLE: UIKit/QuartzCore imports (13)

```text
_CACurrentMediaTime                                          QuartzCore                     uses=1
_OBJC_CLASS_$_CADisplayLink                                  QuartzCore                     uses=1
_OBJC_CLASS_$_CAEAGLLayer                                    QuartzCore                     uses=1
_OBJC_CLASS_$_CALayer                                        QuartzCore                     uses=2
_OBJC_CLASS_$_CAMetalLayer                                   QuartzCore                     uses=1
_OBJC_METACLASS_$_CALayer                                    QuartzCore                     uses=1
_OBJC_CLASS_$_UIImage                                        UIKit                          uses=1
_OBJC_CLASS_$_UIView                                         UIKit                          uses=1
_OBJC_CLASS_$_UIViewController                               UIKit                          uses=1
_OBJC_METACLASS_$_UIView                                     UIKit                          uses=1
_OBJC_METACLASS_$_UIViewController                           UIKit                          uses=1
_UIApplicationDidBecomeActiveNotification                    UIKit                          uses=1
_UIApplicationWillResignActiveNotification                   UIKit                          uses=1
```
