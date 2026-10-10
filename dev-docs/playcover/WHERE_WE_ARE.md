64-bit iOS Runtime Shared Cache Auto-Download Fixed: Fixed 64-bit architecture detection for fat Mach-O binaries containing arm64 slices (`readMachOMetadata` in `IpaInfo.kt`), added synchronous fallback parsing for missing `infoCache` entries in `LauncherActivity.kt` `startGame()`, enhanced `RuntimeCacheManager.kt` download & extraction with HTTP/HTTPS redirect handling (up to 5 redirects), flattened ZIP subdirectories for `dyld_shared_cache_arm64*` entries, added direct raw stream fallback, and provided launch continuation on prompt cancel. Verified all 548 Rust unit tests pass cleanly. Built release APK `app-release.apk` and deployed to Samsung Galaxy Tab S11 (`SM-X930` / `gts11uwifi`) preserving user data.

Zenonia 5 / In-Game Store Purchases on Samsung Tab S11 verified: Built release APK `app-release.apk` with Rust `aarch64-linux-android` target, installed on Samsung Galaxy Tab S11 (`SM-X930` / `gts11uwifi`) preserving app data. Executed `Zenonia5_v1.0.8.ipa` (`com.gamevil.zenonia5free` v1.0.8 ARMv7 slice) with `--unlock-store-purchases` enabled. Implemented `CFArrayGetValues` (`src/frameworks/core_foundation/cf_array.rs`), `-[NSURLCache initWithMemoryCapacity:diskCapacity:diskPath:]` / `setSharedURLCache:` (`src/frameworks/foundation/ns_url.rs`), instance `-[NSObject description]` & `-[NSObject debugDescription]` (`src/frameworks/foundation/ns_object.rs`), `-[UIView setNeedsLayout]` & `-[UIView layoutIfNeeded]` (`src/frameworks/uikit/ui_view.rs`), and fallback `b"0"` data for 0-byte HTTP responses to dead legacy server URLs (`src/frameworks/foundation/ns_url_connection.rs`). App launches cleanly, passes game splash screen and loading screens, and runs main game loop with zero crashes (`tablet_latest_screen_fixed.png`).

Zenonia S / App Version Spoofing per-game option implemented: Added `--reported-app-version=X` flag in `src/options.rs`, live settings command `app_version=X` in `src/live_settings.rs`, version string overriding in `NSBundle` (`objectForInfoDictionaryKey:`, `infoDictionary`) and `CFBundle` (`CFBundleGetValueForInfoDictionaryKey`), default version spoofing (`2.10.0`, the final Zenonia S release) for `com.gamevil.zenoniaonline.ios.apple.global.normal` in `GameSettings.java`, and Version Spoofing UI in `MainActivity.java`.

In-game Store Purchases / DLC unlock setting implemented: Added `--unlock-store-purchases` flag in `src/options.rs`, live settings command `store=unlock` handling in `src/live_settings.rs`, StoreKit IAP auto-granting framework support (`SKPaymentQueue`, `SKPayment`, `SKMutablePayment`, `SKPaymentTransaction`, `SKProduct`, `SKProductsRequest`, `SKProductsResponse`, `SKReceiptRefreshRequest`), and per-game toggle in Android UI (`GameSettings.java`, `MainActivity.java`). All 548 Rust unit tests pass cleanly.

Zenonia 4 Pixel 9 Pro Fold bring-up verified: Implemented OES_vertex_array_object (`glGenVertexArraysOES`, `glBindVertexArrayOES`, `glDeleteVertexArraysOES`, `glIsVertexArrayOES`) in `src/frameworks/opengles/gles_guest.rs` and `CFStreamCreatePairWithSocketToHost` alongside `_touchHLE_CFReadStream` NSStream delegate/run-loop methods in `src/frameworks/core_foundation/cf_read_stream.rs`. App compiles cleanly and launches on Pixel 9 Pro Fold (`comet`), initializing graphics, loading save files, handling server socket creation for 218.145.70.37:32153, and running the main game loop with zero crashes.

Quest metadata-work534: all534native testsPASS (zero failures,nine ignored). Scoped selected class/protocol metadata-derived allowance moves originalstartup beyond protocol-name scan to UndefinedInstruction PC0x1800bac40 LR0x1800beee0 SP0x1bbdf72e0. Evidence pixel-fold-tests/quest-metadata-work source_stabletrue. This snapshot has not yet been deployed after Samsung532verifiedallocation/PIDfix. No initializercompletion/main/title/gameplay. Nextwork is actualoriginalinstruction/caller audit.


Native533 protocol diagnostic verified: original cache-guarded NULterminated34byte name, cursor distance7, mapped35bytes; no malformed/infiniteinput evidence. All533testsPASS. Evidence pixel-fold-tests/quest-protocol-input with source_stabletrue. Delta implementing count-derived privatework allowance from selected class/protocol pointertables; max20M/public1M remain. This is diagnostic startup, no appmain/title/gameplay.


Samsung532 actual Quest PID9873 reproduces PC1800beca8/LR1800b77c8 after successful owned allocations/deallocations beyond old16MiB arena. Evidence pokemon-quest-tablet-quota532.json/.log; APK/data/SHA verified. Tablet remains Awake/StayOn USBtrue. No title/gameplay. Next bounded protocol-name input diagnostic ready and compiling.


Quest quota532: 532 native tests passed, zero failed, nine ignored. Android build succeeded; Samsung SM-X930 USB R52Y8066STA installed APK SHA256 d84a7dee9cc80f5cd5b765ba0850dd3a57b9a8399b4de60efbe31c34ae93c9d3 with data preserved (tablet-quest-quota532-install.json). Genuine owned VM unmap and BSD20 process identity implemented. Corrected legacy_session/cache_init_probe anonymous arena from erroneous16MiB span to existing256MiB quota span, without raising quota/backendcap; meaningful20MiB regression passed. Native Quest now passes prior allocation/PID gaps and exhausts scoped7.6M allowance at original protocol-name scan PC1800beca8 LR1800b77c8. Delta owns bounded metadata diagnostics in a64_legacy_objc_notify.rs/a64_bridge.rs; Dynamic audit agrees no newVMfault evidenced. Tablet actual532 test now verified; see newest evidence. No initializer completion, title, or gameplay verified.


Quest Samsung528 verified APK8b8ebebe2131ec5b7517e12e9e26dfe6700c481794aad77eec87b3ca4a41ef20 installed/hash/data preserved. PID6820 private exact15G77 7.6M allowance advances beyondpreviouscap to realnonemptyanonymousVMdeallocate0x200020c000+0x14000. Dynamic implements actualARM64backendunmap+ownedledger/protection/reusable reconciliation; exclusivea64.rs/a64.cpp backendownership announcedcommunications (not src/cpu.rs/32). Deltaauditsactualallocatorcaller/rounding. Publicbridge1Munchanged; full528testsPASS. Evidence pokemon-quest-tablet-private-work528.json/.log +quest-private-worknativeproof. TabletAwake/StayOntrue USB. No mapperreturn/libSystemreturn/title/gameplay.


Native526 latephase trace corrected (earlySVCs no longer consume8landmarks): quest-late-progress sourceproof stable, actualNXinsertion rem122214 then originalObjCmetadata1800c01cc/LR1800d02e8 rem0 afterall300headersappended. Currentx19=90 isnotvalidheader; samplesgenericfieldnotclassreceipt. Delta/Dynamic auditingactualclass/selector progress andstrictprivatelegacypermit; general1Mcapunchanged. Samsungstill525APK342124... untilnextsubstantivebuild.


Quest Samsung525 memoryadvice verified APK3421245982dfe5d73d58fca990196db67d5e8e38eabc4b2230fc7baaf89ce86d installed/datapreserved. PID5153 actualBSD75 address2000100182,size12,MADV_FREE_REUSABLE7 errno0 thenrealanonymousVM maps7/8. Next1MbudgetexhaustPC1800ad2d8 LR1800ad308 SP1bbdf73c0 depth1. Delta/Dynamic auditingnextphase genuineprogress/metadata beforeanylargerprivatepermit; generalcapunchanged. Full525testsPASS. ObjCmapperstillnotreturned/no title/gameplay. TabletAwake/StayOntrue USB. Evidence pokemon-quest-tablet-memory-advice525.json/.log.


Quest Samsung522 APKff7f409dbf67ec95724ae3fbfe0331f4dd747babe4976eb621e0c31cdced6f14 installed/data preserved. PID3520 verifiedscoped1M realheaderprogress andoldW16 -3clockserviced; nextBSD75 PC18095c1e4 LR1809c9b80 args[2000100182,c,7,c,200000c2c0,0], remaining235490ticks. Dynamicimplementsactualmemoryadvice/errno ownedvalidation; Deltaauditscaller. Full522testsPASS; nocapproblemhere. ObjCmappernotyetreturned/no title/gameplay. TabletAwake/StayOntrue USB. Evidence pokemon-quest-tablet-clock-width522.json/.log.


Quest finite1M scopedwork native521PASS advancesmapper to actualclocktrap W16zeroextended0xfffffffd (4294967293), PC18095bbe4 LR1800b4658. Existing -3 sharedclock implementation misses32bitencodednegative trap. Dynamic implementingauditedoldtrap decoding withoutalteringplatformselector/BSDtags. Requirednativeproofsource_stabletrue; evidence pixel-fold-tests/quest-scoped-work. SamsungstillAPK0943a10...519milestone untilnextcombinedbuild. No mappercompletion/title/gameplay.


Quest Samsung519 ObjectiveC begins: APK0943a10e65d7060b08187d7343579675b4ed65ae9baf2ce0d40332f1ca266304 installed/hashverified/data preserved. PID32328 all427TLVcallbacks return then actualoriginalthreecallbackABI invokes mapped for300selectedObjCimages. Stops100ktickbudget PC18086b874 LR18086b870 SP1bbdf7530 depth1. Frozen-native quest-objc-progress nowproves8changingrealLastHeader pointers withstableFirstHeader andvaryinggetsectiondata/strcmp/headerwork sites under100kticks; requiredsourceproof stable. Delta implementing exact15G77/count-scopedfiniteallowance withinexisting1M ceiling; no globalcapchange. Full519testsPASS afterfixingexact16byte __objc_imageinfo name bothproduction/test. TabletAwake/StayOntrue USB. Evidence pokemon-quest-tablet-objc-notify519.json/.log. No mappercompletion/libSystemreturn/title/gameplay.


Quest Samsung516-loaderpolicy MAJOR verified: APKbcf9b3f1e87b386b8df3e64629459d2ab04f636563ef65f66f703209d76d6023 installed/data preserved. PID30806 ALL427originaladdimagecallbacksRETURNED; genuinecoldmalloc VMmap/protect andpthreadTLVkey/descriptor setup executed. Next missing __dyld_objc_notify_register aftersharedclock/issetugid. Delta implementingoriginal15G77ObjCnotify ABI andactualmappedcallback delivery. Full516testsPASS +3authenticTLVopt-inPASS. TabletAwake/StayOntrue USB. Evidence pokemon-quest-tablet-loader-policy516.json/.log. Still no entirelibSystemreturn/title/gameplay.


Quest Samsung516 notificationpatch installed APK4f2738e3b8a3e74aa8635eda4806b32e82cf6e725df58ce5aa2420e9a7bb42a5/datapreserved. PID30142 starts real synchronous427notifications then stops BEFOREcompletion at __dyld_process_is_restricted. Delta implementing genuineownedloaderpolicy query. Full516testsPASS +isolatedoriginalTLV/key/mutex/descriptor opt-inPASS; noall427completion or gamebootclaim. TabletAwake/StayOntrue USB. Evidence pokemon-quest-tablet-add-images516.json/.log.


Quest Samsung514 threadhelpers verified: APK608a4c3b944998cce3820a5152c74209c838570a132ccc11afc2b857449d8e4f installed/hashverified/datapreserved. PID29389 genuineversion13flathelper retention then originalTLV advances to missing __dyld_register_func_for_add_image. Delta implementingreal synchronous427total selectednotifications (3ordinary+424cache), genuinekeys/descriptorwrites/protectionjournal, notregistration-onlyack. Full514testsPASS; separatelyoriginalkeycallbacks opt-inPASS256keys. TabletAwake/StayOntrue USB; no title/gameplay. Evidence pokemon-quest-tablet-thread-helpers514.json/.log.


Quest Samsung513 verified image-slide lookup fix: APK07034a840b04ee7c9bc27f7f8aada12b58b2bc90062d8960cc2057893701f21e installed/hashverified/datapreserved. ActualPID28767 now advances originaldyld lookup/get-image-slide then explicitly requires __dyld_register_thread_helpers. Delta implementing genuine retainedversion13table; Original pthread keycallback opt-in regression now registered and PASS against15G77: actualkeycreate/set/get/exhaustion256keys; fixture adds samegenuineROcommpage usedproduction. Evidence /tmp/a64-quest-pthread-context-proof.json +execution.log. No initializerreadiness claim. Evidence pokemon-quest-tablet-dyld-lookup513.json/.log; 513 testsPASS. TabletAwake/StayOntrue USB; no title/gameplay.


Quest Samsung511 verified next progress: APK04c7a570e6e3ab5b420ae8e4b2719f98420dd153505bc1373df994ef302e8fc9 installed/data preserved. PID28170 original512MiB nano FIXED request genuinely returns KERN_NO_SPACE3, original fallback/getentropy executes; next NULLPC4 LR18084ec58 SP1bbe08f40 in legacydyld. Delta implementing audited genuine oldlookup/helper protocol. Tablet Awake/StayOntrue USBpowered. Evidence pokemon-quest-tablet-fixed-vm511.json/.log; 511 testsPASS, no gameboot.


Quest priority Samsung test: updated APK74ebfbef0dc40e58f6254ae7f5fc36c0d23df10570dbc3d1b9d8a5873c200e58 installed/hashverified/data preserved; 509 Rust testsPASS. Actual Quest1.0.3 ARM64 PID27575 advances original40-byte registration/clock/semaphore/hostrelease/ownedentropy then stops genuine anonymousVM flags0xb000000 at18095bd34. Dynamic assigned actual VM semantics implementation. Tablet Awake/StayOntrue/USBpowered verified. Evidence pixel-fold-tests/pokemon-quest-tablet-semaphore509.json/.log. No title/gameplay.


503 RustPASS after actualcanonicalselectorlookup test reader ownership correction. Terraria genuinecachelookupnowadvancesNULLLR1800c4610 (terraria-dyld-selector), Delta nextcaller. Legacy501 actual40-byte registrationpublishesownedstate andall3reachMIG206SystemClock request (legacy-registration); genuineold206 adapterbuilt/testnext. Samsung499actual3tests complete/common40byteboundary (legacy-tablet-boot-summary.json), currentAPK1b7a...; stayawakeonUSBtrue overriding30min displaysetting. No initializerreturn/main/title.

Samsung installed499 APK1b7a807aa1634f6faa5f3de75bef168f0b1dd1e7f50a6445dff0bad47bd89996 hashverified/data preserved. Actual Terraria PID22962 reaches __dyld_get_objc_selector NULLLR1800c14b4 after genuine image processing; Delta implements original selector table lookup. Quest103 tabletPID23144 reaches older40-byte BSD366 registration after genuineMach31/threadID/TSD transition; rootcompilesadaptercandidate next. Chained IBII/III testsunderway on tablet, rootno concurrentlaunches. USBchargingstayawakeverifiedtrue overrides displayed30minute timeout; securityunchanged. No title/main/gameplay.

Latest499PASS: sealed original11 TSD transition retained across bridge context restoration; actual3legacyapps reach original BSD36640-byte registration (legacy-original-tsd). Scoped a64_initializer_budget permit validates original20H392 UUID/entry/code/notificationcount while ordinary APIs stay1M; actual Terraria now reaches nextNULL LR1800c14b4 instead of timeout (terraria-scoped-initializer-budget). Dynamic implements legacy40-byte registration; Delta exactnextcaller. Samsung stayawake verified true onUSB, originalsetting0 now15, security unchanged (tablet-stay-awake.json). Coherentcache/3ARM64IPAs fullystagedSHAverified privatefiles (legacy-tablet-stage.json); Android499 build/install/test next.

493 Rust tests pass; genuine oldMach31 hostpriority and BSD372 ownedthreadID native all3 now reach original platform TLS selector2 syscall2147483648 at0x18097f540 with actualstaticTSD1b3288c20. Caller proves self/errno/threadID alreadyfilled, Machportslot3 stillzero. Dynamic owns retained transition validation. Terraria notification work allowance18.925M was rejected by existing GuestBridge1M cap before guestexecution; Delta correcting scopedpolicy to remain within cap first. Evidence legacy-owned-thread-id and terraria-objc-work-budget. Do not confuse policy unit pass with actualexecution. Pixelinstalled487APK6dc1 verified3oldinitializers; newer493 graph notdeployedyet.

Installed Pixel latest APK6dc1a9bde0e0bf668083197f022b6cbaae1668e701d2dd076a638ecaca41247b, Android build successful/hash verified/data preserved (comet-legacy-original-startup-install.json). Contains487-test genuine old initializer route and persistentDomainForName patch. Chained owns three actual legacy initializer device checks + IBII32 nextselector test. Dynamic implements evidenced oldMach31 ABI; Delta modernObjC progress trace hookup. No device startup completion/title yet.

Latest487PASS verified original11 startup: a64_legacy_session.rs validates actual15G77 cache/soleinitializer/prologue and retains real original flat kernel callbacks, primordial owner/TSD/stack/process. Argument and TSD allocators preserve actual mapped compatibility snapshot/getter. Frozen Quest/IBII/III execute original initializer and all stop at genuine Mach trap-31 PC0x18095bde8, real MIG200 host_info flavor5/count8. Request40bytes captured in legacy-original-arenas logs. Dynamic implementing oldmessageABI using existing real RPC/right/reply service; no fake message completion. Original pthread/fullruntime/main/title still unverified. Android487 build underway; installedPixel still23993186 until hashverified update.

Latest root integration verified480PASS: a64.rs wrapper cache_session_image_info_test, CLI --a64-cache-session-image-info-test=PATH, native regression session-image-info. Actual frozen Quest/IBII/III all complete coherent iOS11 mapping/binding plus authentic owned snapshot/getter, then reach explicit original11 libSystem/libdyld protocol gate. Existing strict modern session unaffected. Evidence pixel-fold-tests/legacy-session-compat; no initialization/main/title. Dynamic owns realoriginal11 session implementation; Delta Terraria bounded progress audit; Chained IBII32 namedpersistent-domain API. Installed Pixel APK remains23993186 pending substantive next startup patch.

Pixel actual HOST_VM_INFO capacity fix verified on IBII ARM32: assertion gone, next missing NSUserDefaults persistentDomainForName: (objc/messages.rs273, LR0x7c0a45). Evidence infinity-blade-ii-comet-normal-host-fixed.* APK23993186. Exact Quest1.0.7 latest normal launch is ARM64 and fails Security.framework runtime root; it does not reproduce the user's ARM32 dump. CFBundleShortVersion1.0.7 vs CFBundleVersion3 explains log-version label, not architecture mismatch. No verified title/gameplay.

Latest Pixel installed APK23993186aa4ee6ee6c99b0c92333b94be4807bf86ea44ee5e85847a8f9912271 includes HOST_VM_INFO capacity fix and480-test snapshot; install/hash verification preserves data. Exact device Quest1.0.7 backup SHA4b6719c89b71b50b546acd4093ca9adc3710b5a2df70dbce58e88833605473cf is thin ARM64 (cryptid0), despite user-supplied ARM32 register dump. Earlier 'Quest1.0.7 ARM32' wording describes the dump, not verified executable architecture. Verify package/runtime attribution before claiming Quest resolved. IBII ARM3215vs60 capacity assertion independently reproduced. Agent retesting both.

480 Rust tests pass, zero failed, five ignored. HOST_VM_INFO ARM32 caller capacity fix in libc/mach/host.rs accepts oversized buffers, rejects undersized buffers without writes, and returns actual written word count. User Quest1.0.7 ARM32 crash and agent IBII15vs60 failure establish this actual bug; thin ARM64 Quest1.0.3 is a separate backup/path. Staging includes host.rs. Latest genuine Terraria stall now has active pre-restoration PC0x1800c4100 LR0x1800c40d8 depth1 (terraria-objc-loop-trace); Delta audits original loop. Dynamic integrates genuine owned legacy image-info service into Quest coherent-cache session before original iOS11 initializer audit. No boot claimed.

MediaToolbox copied and hash-verified on Pixel Fold 9 Pro at /data/user/0/org.touchhle.android.a64test/files/ios-runtime/legacy-extracted/System/Library/Frameworks/MediaToolbox.framework/MediaToolbox. Exact9,005,888 bytes/SHA256 e290d05dfd7a610372bda77b96f9eeee0cccbfe436be914ab6db66de3f82c8df. Copy evidence pixel-fold-tests/mediatoolbox-comet-copy.json. Artifact remains a separate cache-bound reference image; coherent iOS11 cache is the execution provider. Actual Quest legacy dependency preparation passes on Pixel; no verified initializer/main/game boot.

MediaToolbox local artifact verified: ios-runtime/legacy-extracted/MediaToolbox.framework/MediaToolbox from genuine iOS11.4.1 15G77 ARM64 cache UUID7336d75f301433e7843fe1f3522fc52f. Size9,005,888, SHA256 e290d05dfd7a610372bda77b96f9eeee0cccbfe436be914ab6db66de3f82c8df; exact installname /System/Library/Frameworks/MediaToolbox.framework/MediaToolbox and 35 dependencies parsed. provenance.json alongside artifact. Cache-resolved addresses/zero independent rebase stream mean standalone_ready=false. Preserve coherent cache execution. Chained copying separate app-private Pixel artifact per user; await verified device path/hash.

Samsung 478-test APK df1f9730373fbe91a1d9a35ff4e5562fe1fcaef6fa0f81452888fe969350fcd5 installed/hash-verified with data preserved. Actual scoped ARM64 PID2580 confirms tick-budget exhaustion after genuine cache-range query; no panic or game boot. Evidence tablet-cache-range-install.json and terraria-tablet-cache-range.log/.json/.png. Chained assigned user-requested Pokémon Quest and Infinity Blade II/III boot tests on Pixel9ProFold192.168.0.22:41105; separate device ownership from root Samsung. No results for those tests yet.

Latest native milestone: 478 Rust tests pass, zero failed, five ignored. `a64_dyld_cache_range.rs` implements the genuine `_dyld_get_shared_cache_range` ABI from the original cache header: modern sharedRegionSize/mappedSize, actual header base, checked all-region containment and guest size output, exact original wrapper/export guards. Actual Terraria advances beyond NULL LR0x1800c3e24, then hits `guest call tick budget exhausted`. Current diagnostic omits PC; Dynamic adds bounded failure context and Delta audits the actual stalled path before any budget change. Evidence pixel-fold-tests/terraria-cache-range and /tmp/a64-cache-range-proof.json. No libSystem completion or game title/gameplay. Android build of this frozen 478-test snapshot underway; Samsung installed APK remains 96b5...476 milestone.

Device collector correction verified: four focused tests pass. `tools/test_ipa_device.py` now scopes a unique run marker to exact MainActivity process `org.touchhle.android.a64test:game`, excluding Claude's 32-bit process and the launcher. Untouched terraria-tablet-callback-stack-scoped raw log verifies ARM64 PID1042, no panic, and the expected shared-cache-query MemoryError0/LR0x1800c3e24. Earlier unknown-PID JSON is superseded by this verified analysis; next device capture uses corrected collector.

Current verified milestone: 476 Rust tests pass, zero failed, five ignored. Original Objective-C mapped callbacks now execute; the actual Darwin stack probe passes with bounded 64 KiB callback stacks and unmapped 4 KiB guards. `a64_bridge.rs` owns checked runtime stack geometry, and `a64_host_services.rs` accounts for the actual reservation. Tiny unit-test bridge geometry remains supported. Native Terraria next stops at `_dyld_get_shared_cache_range` (wrapper 0x1a6c7da04, caller LR 0x1800c3e24). Matching dyld1042.1 requires the actual cache header/base and mapped VM span including gaps, not allocated-byte totals. Delta implements that service and Dynamic the context hookup. Evidence: pixel-fold-tests/terraria-callback-stack; no game boot.

Samsung latest APK SHA256 96b5e9319fb29664c4ccbe32d0c807448c1702331c2660e8e29135d2490f6f83 installed/hash-verified with user data preserved after successful Android build. Installation evidence: pixel-fold-tests/tablet-callback-stack-install.json. Actual device ARM64 process32559 confirms next missing shared-cache query (PC0, LR0x1800c3e24), matching native. Evidence: terraria-tablet-callback-stack.log/.json/.png. Summary first_panic includes unrelated earlier process32480 and must not be treated as this ARM64 failure; Chained is correcting collector scoping. No title/gameplay.

Samsung471milestone APK3964cc98eefb3f5223ee606c085bf18d3a5302eb331a204763711c2934343d4f installed/hashverified/data preserved afterAndroidunit/buildPASS. ActualTerraria tabletconfirms SDKquery andgenuineMachRPC8000 registration result0 thenNULLLR1800c2474 (PC0Android/4native). Next _dyld_objc_register_callbacks verifiedVERSION1 descriptor [1,map1800c5770,init1800c728c,unmap1800d99d8,patch1800e09c4]. Agentsimplement actualmapped image notifications/scopedprotection, no registration-only success. Evidence terraria-tablet-restartable.log/.json/.png +tablet-restartable-install.json. No gameboot/runtimefullready.

Verified471RustPASS/0failed/5ignored after correctnamespacependingticketfixture. GenuineMachRPC8000 validates5actualObjCranges/RXtargets, retainsone-timesinglethreadprocesscatalogue andrealMIGreply; scheduler explicitAST recoverytested(boundaries,blockedwake, scalar/SIMD/SP preserve,voluntaryyieldunchanged). ActualTerraria reachesnextNULLLR1800c2474: _dyld_objc_register_callbacks wrapper1a6c7f2d4 with40byteversioneddescriptor; Delta/Dynamic/Chained implementingactualmappednotifications, no registration-only fake success. Evidence terraria-restartable; 471Androidbuildactive, rootsolebuilder. No title/gameplay.

Verified464RustPASS/0failed/5ignored. Genuine dyld_program_sdk_at_least nowreads actualselectedmainMachOSDK/platform andsupports observedfall2020thresholdiOS14.0; no reportedOS/fixedbool. ActualTerraria advances toMachRPC8000 task_restartable_ranges_register (116bytes/5ranges); Dynamicimplements boundedledger/MIG, Chained actualCpuSchedulerrecovery, Delta auditscaller/completebody. FrozenlegacyIBII/III/QuestpreparationregressionstillallPASS afterSDK/sessionrefactor. Evidence terraria-sdk-query +legacy-cpp-session-regression. Samsunginstalled856...462milestone; no title/gameplay.

Samsung retained-session APK856781a8089750da0b3f06fb2b4a3d74b6a95f3cef97eff1cf3d44f086719387 builtAndroidPASS/installedhashverified/data preserved. ActualtabletTerraria confirms realhelperreturn, sharedclock,535hardwareabsence,327guestcredentialquery thenNULLcallee LR1800c24ac (PC0Android/4native). Deltaidentifiedactual dyld_program_sdk_at_least wrapper1a6c7d7e4; SDK/platform/version-set implementationactive, usingmainactualmetadata notreportedOS. Evidence terraria-tablet-session-credentials.log/.json/.png +tablet-session-credentials-install.json. Native462PASS, no runtimefullready/title/gameplay.

Verified462RustPASS/0failed/5ignored. RetainedSession completion validates genuine dyld1042.1 helper+globalkeys+TLVcatalogue and returns tooriginal libSystem caller, no fullreadyreceipt. ActualTerraria reachesObjectiveC:535 BPassisthardware-unavailable fallback then327 ownedcredentialtaint query nowimplemented/verified; nextgenuineNULLcall PC4 LR1800c24ac SP22f854ea0 auditedbyDelta. Evidence pixel-fold-tests/terraria-session-completion,terraria-objc-bp,terraria-credentials. New462Androidbuildactive rootsolebuilder; installedSamsungcurrentlye973452milestone. No title/gameplay.

PersistentSession integration verified457RustPASS/0failed/5ignored. NewcachedSession owns same PreparedCacheApp CPU/bridge plusExecutionSession realMachrights/VM/FD/registration/RcScheduler/TSD acrosscalls; actualTPIDRROgetter validatespre/postowner. Realstatepersistence/quarantine testsPASS. Frozen provenTerraria sessionprobe reaches4TLV/6desc+lock/lazyroute then guardedhelpercompletion; no libSystemreturn/runtime receipt. Matchingdyld1042.1 sourceaudit supports validating exactcompletedkeys/catalogue/helperassociation and returning tooriginalcaller; Delta/Dynamic implementingthatnext. Evidence pixel-fold-tests/terraria-persistent-session. LatestinstalledSamsungstill e973...452testmilestone.

SamsungUSB verified runtime milestone:452RustPASS/0failed/5ignored; Androidunitchecks/buildPASS after staging Claude SymbolIndex/Compat+JNI/Security dependencies. APK e973be7c9d1c2f2c71e462db80e0bdddea944b13c3e367a765cc2a7c64b797f6 installed/hashverified on R52Y8066STA preservingdata. Actual Terraria initializer diagnostic binds5973imports across5ordinary/64cachedependencies, executes34SVC and genuine malloc/set/get for4TLVimages/6descriptors, real loaderlock released/lazyrouteinstalled. Nextgate persistentprocesssession/remainingdyldbootstrap; no runtimeinitreceipt/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-image-info-tlv.log/.json/.png +tablet-image-info-tlv-install.json. Delta/Dynamic implement persistentSession next; rootsolebuilder.

Verified runtime continuation:448RustPASS/0failed/5ignored. New genuine loader-owned LP64version1 image-info snapshot +RXgetter resolves Quest ARM64 CydiaSubstrate import via explicit compatibility service, not forged cachedexport. Frozen/build-proven threelegacycases all dependency/binding preparation PASS (IBII/III/Quest); initializers/main untested. Terraria baseline still originallibSystem34SVC→4TLVimages/6descriptors→loader-lock/futurethread gate. Dynamic implementing lock/lazyTLVroute. Evidence pixel-fold-tests/legacy-cpp-image-info and terraria-image-info-baseline. Newtools/build_arm64_proof.py +runner requiredproof;6Python testsPASS. No newAPK/device milestone yet.

Quest next-gap clarification: embedded ARM64 CydiaSubstrate imports __dyld_get_all_image_infos from libSystem; genuine iOS11 ARM64 provider has no public export. Matching SDK11.4 advertises that export only for ARMv7/ARMv7s. Private cache implementation is not an exported provider. Remaining work requires genuine loader-owned image-info ABI compatibility, not a missing downloadable library or fabricated alias.

Verified legacy C++ follow-up:445Rust tests PASS, zero failed, five ignored. Corrected genuine uint64 signed-addend semantics for Quest PhysX RTTI and fixed staging of a64_legacy.rs. Infinity Blade II/III dependency preparation still PASS. Quest now requires original __dyld_get_all_image_infos export route; investigation active. Latest frozen three-case evidence pixel-fold-tests/legacy-cpp-prepare-verified/REPORT.md. Initializers and gameplay untested.

Verified legacy C++ acquisition/integration (2026-10-07): genuine ARM64 libstdc++.6.dylib 104.2 plus libc++abi/libSystem audit extracts acquired from original iOS11.4.1 cache; coherent-cache preparation implemented, not mixed into iOS16. Reference SDK11.4 and SDK16.4 acquired with provenance (16.4 lives in Theos iPhoneOS16.5.sdk folder). Frozen native build444PASS/0failed/5ignored; SDK symlink resolver3PASS. Fifty original-byte/UUID guarded imported selectors integrated. Infinity Blade II/III preparation PASS; Quest reaches legacy bind target overflow, loader investigation active. Initializers/app-main/gameplay untested; no new APK/device milestone. See ios-runtime/LEGACY_CPP_INTEGRATION.md and pixel-fold-tests/legacy-cpp-prepare-final/REPORT.md.

Verified ARM64 regression integration: 441 Rust tests PASS, zero failed, five opt-in probes ignored; 12 Python inventory/grouping/summary tests PASS. New reusable tools/run_arm64_regression.sh inventories actual IPA metadata/encryption, hashes/deduplicates backups, freezes one native build and collects bounded original libSystem diagnostics. Baseline inspected31files and probed29 unencrypted ARM64 backups (including3 development fixtures); encryptedGarageBand skipped, Pokedex32bit excluded. Patched17-app retest verified six audited resolver routes and real1MiB VMalignment advance; seven realapps reach genuine TLV publication then loader-lock/future-thread boundary. tvOS packages flagged separately. Final five-app budget retest verified stable441-test snapshot: AION2 passesconstructorlimit thenfails genuine WebRTC install-name alias matching; Minecraft passesconstructorlimit thenneeds thread-local abslseed binding; RE4/Village pass512MiB mapping preparation thenrequire audited dispatch_data_create resolver; Terraria publishes4 realTLVimages/6descriptors thenstops loader-lock/future-thread setup. Allbaseline29 probes plus17 and5 affected-app retests completed with frozen-source checks. Remaining common namedresolver gaps are dispatch_retain/data_create/block_cancel/barrier_async and OSAtomicEnqueue; audit ios-runtime/ARM64_BATCH_RESOLVER_AUDIT.md. Six packages are actuallytvOS, three are fixtures; preserve classifications andfirst-boundary-only scope. Readyreport ARM64_REGRESSION.md/.json is authoritative perapp latest evidence. Summary ARM64_REGRESSION.md/.json; per-run logs pixel-fold-tests/arm64-regression[-patched|-budget]. No new APK/device/gameplay milestone.


Integrated candidate batch: 430 Rust tests PASS, zero failed, four ignored. New owned null-callback CFDictionary lifecycle/services, shared-clock -3/-89 adapter, and actual mapped-image TLV parser/tests integrated. Frozen tlv-dictionary-integrated PCprobe26.484secs parsed two real ordinary TLV plans (609bytes/2descriptors;8bytes/1descriptor;zeroinitializercallbacks), then rejected cachedheader0x186de4000 nonzerodescriptorkey before bootstrap. Delta verifyingcacheTLVkeysemantics; no clearingkeys/pretendedTLSsetup. No newAPK or title/gameplay. Evidence pixel-fold-tests/terraria-native-tlv-dictionary-integrated.log/.json.


Verified original dyld helper progress: PC dyld-helper-call-provenance23.136secs executes two genuine guest pthread-key callbacks using version6helpers, then explicitly stops before loaded-image TLS descriptor setup/loader-lock publication. Last integrated count421PASS; no dyldinitreceipt/title/gameplay. Newrepoaudits active unidbg/WinObjC/ipaSim/Inferno; check placeholders/version/licensing beforeadaptation. Evidence pixel-fold-tests/terraria-native-dyld-helper-call-provenance.log/.json.


Source reuse integration now421RusttestsPASS, zero failed, fourignored: sharedCFNumber producer/conversion/lifecycle and coherentCPU CNTPCT/timebase helpers centrallycompiled/tested, version6dyldhelperadapter compiled. Actualdyld-version6-helpers PCprobe19.816secs stops before guestexecution on provider/target provenanceguard mismatch; Delta investigating exactsymbolmetadata, no startupadvanceclaim. PlayCover/PlayTools cloned/audited; display/input reuseworker active, no foreignMacruntime imported. No newAPK/title/gameplay.


Verified source-reuse milestone: 410 integrated Rust tests PASS, zero failed, four ignored. Fixed staging of Claude CGImage dependency cg_data_provider.rs. Frozen PC zero-deallocate-fixed-staging19.725secs passes XNU-correct Mach-12 zero-size no-op and zone protection, reaches nextnullcall PC0x4 LR0x1d1c29784 after33traps. Delta verifying actualdyldhelpersbootstrap version6; boundeddescriptor ready not initialized. Workers activelyadapting Apple/Darling selected APIs; source/licenseprovenance in ios-runtime/REUSED_RUNTIME_CODE.md andVM_REUSE_NOTES.md. No newAPK/title/gameplay.


Verified Mach allocate and ARC milestone: 409 integrated Rust tests PASS, zero failed, four ignored. Frozen PC mach-allocate-arc25.667secs passes actualMach-10 zeroed4KiBallocation/writeback, reaches nextMach-12 deallocate PC0x1c260cea4 LR0x1c260def8 args(task0x103,addr0xdeaddeaddeaddead,size0) after32traps. Workers investigating exactDarwinzero-lengthsemantics; no fakeunmap. ARCnesteddealloc/publicationtests pass butTerrariagameinitnotcompleted. No newAPK/title/gameplay. Evidence pixel-fold-tests/terraria-native-mach-allocate-arc.log/.json.


Verified immutable-range milestone: 391 integrated Rust tests PASS, zero failed, four ignored. Frozen PC immutable-range20.248secs clears previousnullcall and reaches actual Mach-10 allocate, PC0x1c260ce8c LR0x1c2610cf8, args(task0x103,addressptr0x22f854998,size0x4000,flags0x1000001) after31traps. Dynamic implementing realfourargumentallocation; Delta ownsrouting. ARCsharedruntimeintegration ready butnotyetcompiled. No newAPK or title/gameplay. Evidence pixel-fold-tests/terraria-native-immutable-range.log/.json.


Verified real guard-page protection: 389 integrated Rust tests PASS, zero failed, four ignored. Frozen PC guard-page-protection probe21.057secs passes six genuine mprotect operations including PROT_NONE guard pages and actual zone R/RW transitions, reaches next MemoryError(0) PC0x4 LR0x19455c000 SP0x22f8549b0 after29traps. Delta investigating exact next loader immutable-range call; ARCintegration proceeds. Ben clarified Codex64bitTerraria, Claude32bit. No newAPK or title/gameplay. Evidence pixel-fold-tests/terraria-native-guard-page-protection.log/.json.


Verified terminal-query milestone: 367 integrated Rust tests PASS, zero failed, four ignored. Frozen PC terminal-query probe24.677secs passes both genuine stderr nonTTY ioctl checks and two additional MachVM allocations, reaches actual BSD74 mprotect(addr0x2000010000,len0x4000,prot0), PC0x1c2612e88 LR0x19455e1d0 after23traps. Real protection backend compiled but dedicatedCPUtests/adapter notyetintegrated. Workers own adapter/routing and ARCpublication corrections. Claude doing32bitAndroid/Samsungbuild; root nativeonly. No newAPK or Terraria title/gameplay. Evidence pixel-fold-tests/terraria-native-terminal-query.log/.json.


Verified restricted loader-policy milestone: 364 Rust tests passed, zero failed, four ignored. Frozen PC restricted-policy probe18.925 seconds clears prior nullcall and reaches actual BSD54 ioctl on stderr fd2, request0x4004667a, PC0x1c2611358 LR0x1c2611398 SP0x22f854930 after19traps. Delta assigned genuine standardFD/ioctl policy. Claude supplied uncompiled ObjC ARC and mprotect helpers; runtime worker assigned real protection backend. No new364APK yet and no title/gameplay. Evidence pixel-fold-tests/terraria-native-restricted-policy.log/.json.


Android image-slide milestone built successfully after 361 passing Rust tests. APK SHA256 5d5a503c8fd8bef36be3dc5da0eda7d25fce4df863d16086f27f7b3930348001 installed and hash verified on Samsung USB R52Y8066STA and Pixel 9 Pro Fold 192.168.0.22:41105, user data preserved. Latest PC boundary is _dyld_process_is_restricted, LR 0x19455a5fc; Delta owns enforced loader-policy implementation. Claude cowork.md documents coordination and ownership. Samsung execution of this APK pending. No Terraria title/gameplay.


Verified image-slide integration: 361 Rust tests passed, zero failed, four ignored. Frozen PC probe image-slide-routing completed in 18.519 seconds; exact audited libdyld slide entry routes to a genuine loaded-header/signed-slide ledger. Previous null-call LR 0x19455df58 cleared; next actual MemoryError(0) PC 0x4 has LR 0x19455a5fc and SP 0x22f854a00 after 18 supervisor traps. Agents classify the next original malloc caller. Full Android milestone build started; no new APK installed yet. No runtime initialization completion or Terraria title/gameplay. Evidence pixel-fold-tests/terraria-native-image-slide-routing.log and .json.


Verified native fast-loop milestone: 357 integrated Rust tests passed. Original libSystem initializer now passes 18 supervisor traps, including system clock, semaphore creation, secure entropy read and getentropy, and genuine absent shared-memory lookup. Frozen native malloc-failure-context probe completed in 18.994 seconds and stopped with MemoryError(0), PC 0x4, LR 0x19455df58, SP 0x22f8549c0 captured before context restoration. Original malloc caller points to __dyld_get_image_slide; agents are investigating the real loader service. Legacy commpage CPU counts already coherently report one CPU; no speculative topology patch. These latest helpers are native verified, not yet in the device APK. No initializer completion or Terraria title/gameplay. Evidence: pixel-fold-tests/terraria-native-malloc-failure-context.log and .json.


Verified actual Samsung pthread registration milestone: APK64d0957f3742b5efd5e3e35d78976113e4770939446c9ebaaadb4664c05546ca built344RustPASS/0failed/4ignored, installed/hashverifiedSamsungUSB andPixel9ProFold preservingdata. GenuineCpuScheduler adoptsmainThreadId1/actualstack22f740000..22f840000; BSD366 validatesrealRXcallbacks and56byteoffsetdata, realatomiccopyout/publishedregistration/privateinstalledCTL derivesimplemented0x4000001e; originaliOS calleracceptsandcontinues. Nextactualhost206 SYSTEM_CLOCK request36bytes/clock0 viaMach47 PC1c260d030, args[22f854f10,200000003,2400001513,60300000703,ce00000000,60300000000,30,0]. SameboundaryPCnative18secinclcompile confirmsfastpipeline. ClockSend/providerRPChelpers nowbeingintegratednotyettested. No runtimeinitcompletion/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-pthread-registration.log and terraria-native-registered-thread-control.log.


Central registration integration corrected and verified: actualsingleScheduler reexportused (no duplicated scheduler types); sharedCPUwrapper priority accessors and realCPUcontext regression included. 340RusttestsPASS/0failed/4ignored in /tmp/playcover-registration-integrated-tests.log. ProcessRegistration validatescapturedBSD366fields andownedRXentrypoints; current-threadQoSsubset updatesgenuinepriorityscheduler. These newcore patches notyetinAPK121a/currentdevices and notwiredasBSD366 success. Agents implementingtruthfulregistration capability/SETSELF services andCPUadoption; full0x4000001e maskcannotbefaked. Nativefastprobe workflowverified, seeios-runtime/TERRARIA_NATIVE_PROBE.md.


Verified Samsung bootstrap-context milestone: APK121aabc0467e4f82b1364cdcb0a5168b058d6854d8b151f9f732b0a61198aeea,336RustPASS/0failed/4ignored, installed/hashverifiedSamsungandPixel9ProFold preservingdata. Originalpthread parser requires literal0x hexprefix; correctedrealmain_stack/OSrandomptr_munge/ownedth_port clearsBSD202 fallback and reachesBSD366__bsdthread_register PC0x1c26148d4 LR0x1d1b98d40 args[1d1b96724,1d1b96718,4000,22f854ec0,38,a0]. Nativefastprobe reproducesexactSamsungCPUboundary; 8.63secguest/28.288secfirstinclcompile,18.721secprefixrun inclcompile. Actual56byte registration fields captured in terraria-native-pthread-registration-audit.log:version56,dispatch160,TSD224,return40,mach24,joinable392,quantum960. Realpriority scheduler integrated; pending registration/SETSELF feature services requiretruthfulcapabilitiesbeforeadvertising0x4000001e. Currentnewregistrationintegrationfailsmoduleimport(standalonetestsnotcentralproof); workerfixingactualsingleSchedulerexport. No runtimecompletion/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-bootstrap-prefix.log.


Verified host-send release milestone: APK6b21c914c1bda1b826d8d318ea98a985268c0912d111eeb4ee7fc3002537a24c,328RustPASS/0failed/3ignored, installed/hashverifiedSamsungandPixel9ProFold data preserved. Originalguest Mach-18 task103/host403 nowreleasesactualuref returns0, receive503preserved. NextverifiedBSDtrap202 __sysctl PC0x1c260e53c LR0x1d1b9a28c args[22f854f00,2,22f854f08,22f854ec0,0,0], originalpthread fallback for absentmain_stack bootstrapmetadata. Delta retaining actualmainstackloaderdescriptor, dynamic_vm boundedformatter, chainedauditbsdthreadregistration. SupplygenuineRWapplemain_stack/OSrandomptr_munge/ownedth_port; no guessedUSRSTACK/sysctlsuccess. No runtimecompletion/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-host-release.log.


Verified HOST_PRIORITY_INFO milestone: APK5f06dde4ae12e7c84a03bcb3430581e974f16aabf734f09058f3152ac6a2f784,326RustPASS/0failed/3ignored, installed/hashverifiedSamsungandPixel9ProFold preservingdata. OriginaliOS mach_msg2 host_info request receives actualboundedvirtualDarwinpriorityreply andguestparsercontinues. NextverifiedMach-18 deallocate PC0x1c260cee0 LR0x1c260ddd0 SP0x22f854ea0 args[103,403,0,22f854f30,c800000000,50300000000]. Workerimplements ownedHostSenduref release; nextmainthreadstartupcontext audited. No runtimeinitcompletion/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-host-priority.log.


Verified Mach message request capture: Samsung APK9fdc5e646532d83bfbe050817745a76d4f1ec6d4d9d0d34714b0aa5ccab3466a,321testsPASS. Actualmach_msg2(-47) options0x200000003, message40bytes bits0x1513, host0x403,reply0x503,voucher0,id200,NDR0000000001000000,flavor5 HOST_PRIORITY_INFO,count8; receive_size0x140. All8args [22f854d40,200000003,2800001513,50300000403,c800000000,50300000000,140,0]. Worker implementing bounded genuinevirtualhostpriority RPC with realrightvalidation/sendoncetransaction; rootnextintegration/test pending. No message delivered or initialization success yet. Evidence pixel-fold-tests/terraria-tablet-host-message-audit.log.


Verified reply-port construction milestone: APK9390d9540cbd4d783b7faa4c09b0aaba2d96dab4726bad16be119dfee38e3617,320RustPASS/0failed/3ignored, installed/hashverifiedSamsungandPixel9ProFold data preserved. Originalinitializer Mach-24 constructsrealMPO_REPLY_PORT receive name0x503, returns0 andguest continues. NextexactMach-47 (mach_msg2) PC0x1c260d030 LR0x1c261eb18 SP0x22f854ce0 x0..5=[22f854d40,200000003,2800001513,50300000403,c800000000,50300000000]. Capturing exact40-byte host_info request before implementing real bounded virtualhostpriority service. No runtimecompletion/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-reply-construct.log.


Verified host identity milestone: APK935ee50ddc8497e555399b051649f11682cdef5ab32e2a9d58933a5250c97b72 built with315RustPASS/0failed/3ignored and installed/hashverified onSamsungUSB R52Y8066STA andPixel9ProFold192.168.0.22:41105 preservingdata. ActualoriginallibSystem passes task/reply/TSD/VMallocation and host_self(-29) name0x403 in pthread_init. Nextverifiedtrap-24 mach_port_construct PC0x1c260cf28 LR0x1c260f5fc SP0x22f854cd0 args[103,22f854d00,0,22f854d1c,49000001,3]. Agents assigned real constructed MPO_REPLY_PORT receive right/options validation, constructhelper andnextMIGhost_info. No runtimeinitcompletion/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-host-identity.log.


Verified anonymous VM milestone: Samsung APK9b96f75d10d005d35d95d7d0e1ade3234a917df1c463bbdd1b81ff1666cdaf94 installed/hashverified;314 Rust PASS/0failed/3ignored, Android build successful. Original libSystem Mach-15 now actually maps32KiB zeroed RW memory, writesback address, returns0; originalguest allocator continues. Nextactual unsupportedMach-29 (host_self_trap; caller ___pthread_init+144, not thread_self) PC0x1c260cf64 LR0x1d1b9a14c SP0x22f854ec0 args[1e29fd298,22f865000,22f865010,22f865040,49000001,3]. Agents assigned exactthreadself ABI/caller andnextbootstrap. No ctorcompletion/initreceipt/title/gameplay. Evidence pixel-fold-tests/terraria-tablet-anonymous-vm.log.


Verified Samsung corrected Mach-name probe: APK f5d4dfc0a1031e483076eecffc72efb075dd492bc67e63e9294e3c3185ef6758, 310 Rust tests passed / 0 failed / 3 ignored. Original libSystem passed task_self (0x103), reply port (0x303), and primordial TSD read. Next evidenced trap -15 requires anonymous VM map: task0x103, address pointer0x22f854e78, size0x8000, mask0, flags0x49000001, protection3. Earlier APK941e02 probe used invalid port generation bits and is superseded. Genuine bounded VM helper ready; integration/build/device retest pending. No libSystem completion or Terraria title/gameplay. Evidence pixel-fold-tests/terraria-tablet-mach-generation.log.


Exact nextfaultclassificationCORRECTED fromoriginalcachesymbol lookup:PC1d1b03630 is __os_once variant libsystem_platform, notlibdyld. MRS TPIDRRO_EL0 thenreadbase+0x18 (TSDslot3) faults24: missinggenuineprimordialthreadTSD. Delta implementingdiagnosticthreadrecord with actualscheduler/TSDownership; chained audits exactslot3ABI. No fabricateddyldtable/initreceipts. Last307testAPK2935183 originallibSystempassed2Machtraps thenstopped; no gameboot.


NEW REAL INITIALIZER EXECUTION SamsungAPK2935183546a8c0ea3583837d78363ebe1935e951bc7c3dfdce82904627562bb1 installed/hashverified;307RustPASS/AndroidBUILDsuccessful. DecoderhandlesS_INIT_FUNC_OFFSETS andptrlists genuinecacheprovider/RXproof. Actualoriginal libSystem0x1d1c296c4 executed correct5argABI; genuinevirtualMach task_self-28pc1c260cf58name100/replyport-26pc1c260cf40name101 succeeded. Next MemoryError24PC1d1b03630; contextrestored/discardeddiagnosticCPU/noinitreceipt. Deltaassignedexactdyldbootstraplookup/implementation, classworkerkernelstate. Evidence terraria-tablet-libsystem-probe.*. Ordinarygamepathnotinitialized/title/gameplay.


Classpublicationmodel/installhook integrated2scopedtestsPASS; fullnextbuildpendingcacheinitializerexecution. Actualdependencystrengthaudit confirmsARKitSTRONGordinal26LC_LOAD_DYLIB; do notskip. Runtimeworker identifiedlibSystemactual __init_offsets(type0x16)initializer0x1d1c296c4; kernel/pthreadnoowninitializerlistsbecauselibSystemcallsstartup. Audittooltools/audit_cached_initializers.py +terraria-libsystem-initializer-audit.json. Agentsimplement genuine prerequisitefunctionexecution/Darwinservices/publicationhooktests. LastSamsung3c2971f5 registrationPASS39deps/595ctorspending; no title/gameplay.


Unityregistration ACTUALPASS SamsungAPK3c2971f5fa5f5c350f8520761c08f650ca4a6c379fc12ded50c30710b1706fb5 installed/hashverified;292RustPASS/AndroidBUILDsuccessful. RequestedUnityclosure4ordinaryimages excludesmain;759173chargedbytes493211reads; duplicateNSBundlefailurecleared. Nextexactgate39dependencyruntimeinitreceiptsfirstARKit.framework/ARKit,595Unityconstructors/0+loadunexecuted. Deltaassignedrealinitializerexecution+weak/strongdepsaudit; chainedclasspublication+atomicinstallhook. Metadata/mappingpasseddoesnotmeanUnityinitialized. Evidence terraria-tablet-unity-scope.*. No title/gameplay.


Metadata-budget/dedup ACTUALPASS Samsung347c6bce9b99fe2054358d28daa241d8ae1cac03441adf9b2a6e8a5e73d550f2,290RustPASS/AndroidBUILDsuccessful. Registrationcharged522248bytes/345245reads, priorUICommand1MiBfailurecleared. Nextexacterror duplicateObjectiveCclassnameNSBundle frommixingownedmainredirectwithUnityoriginalclassrefs; DeltaassignedrequestedUnity+actualordinarydependencyclosure scopefix(noalias). Evidence terraria-tablet-metadata-budget.*. NoUnityinit/title/gameplay.


Selectorrepair ACTUALPASS SamsungAPK74e0665cd2fa8a0ca7f8d45011a429d843369449bb889b06c0b6ad63488f47ed:286RustPASS/AndroidBUILDsuccessful, installedhashverified. Validated originalcachev16 selectorbase0x1822991cf; NSAssertionHandler emptymethod failure cleared. Nextrealregistration class0x1e1da36c8 hostservice memorybudgetexceeded(default1MiB/readwritepercallback). Delta assigned boundedmetadataregistrationbudget/flow, classworker lookup+readoptimization. Evidence terraria-tablet-cache-selectors.*. NoUnityloaded/title/gameplay.


Cachedclass blocker identified from original cache:0x1dcd47278 =Foundation NSAssertionHandler; NSObject islibobjc0x1e1d2e618. Chained repairing exactmethod-list format. Completepremaplog saved terraria-tablet-unity-premap-complete.log. Device test wait now configurable(default45sec,max60) because expandedUnitygraph takes26.5sec; py_compilePASS. LastinstalledSamsung7ba85d/283testsPASS; noUnityinitializer/title/gameplay.


ActualUnitypremapPASS onSamsungAPK7ba85dfbaa585d68ffa9c2891425579d3aa25cb00527432d4f7815a56795975d:283RustPASS/AndroidBUILDsuccessful/hashinstalled.5ordinaryimages64cachedeps5973bindings(1612cache)50selected37selectors2810675200mappedbytes916deferredconstructors. ActualmainUnityload atLR0x10000408c now reaches mappedclass registration and fails cached class/isa/superclass0x1dcd47278 emptyObjectiveCmethodmetadata. Chained assigned genuine cachedformat parser repair; loader remains frozen. NoUnityinit/title/gameplay. Evidence terraria-tablet-unity-premap.*; initial25sectestmaycapturebefore26seccacheprepare, use fullnewlog additionally.


Latest actualSamsungAPK16f6fe86b2b54fdce413bfd5eae1562fedfca3e40e9e9d565f80f157b8ff370b installed/hashverified;280RustPASS/AndroidBUILDsuccessful. Unityload coordinator now genuinely invoked with exact framework executable at guestLR0x10000408c. Next actualerror selectedUnityimage is not mapped: initial3ordinary main dependency graph excludes dynamically loadedUnity. Delta assigned genuine incremental mapping/cache linking/provenance merge; classworker coordinates addedimage registration. Evidence terraria-tablet-unity-load.*; noUnityloaded/title/gameplay. Fold remainsdf80manualbuild.


NEW ACTUAL SAMSUNG MILESTONE: APK541c0e6b588603ca04535dd2cd51828caf7fded7aeba7d656fe2280f18488fe6 installed/hashverified.277RusttestsPASS/AndroidBUILDsuccessful. Real Terraria entry now passes former objc_msgSend boundary via1verifiedNSBundleclassref,7canonicalselectors,1verifiedmsgSendGOT; genuine ownedNSString initialization. Nextactualerror _touchHLE_NSBundle_load requires image/dependency/+load execution and registration receipts; no fake loadedflag. Evidence pixel-fold-tests/terraria-tablet-owned-foundation.*. Agents reassigned realUnityload/init/classregistration. No title/gameplay; Fold latestinstalleddf80 remains manualtest.


Foundation startup helper integrated and centrally compiles (0 scoped tests, typecheck only). ARC release upgrade2actualCPUtestsPASS preserves bound trampoline/address and executes dealloc. Thread publication3native testsPASS. New runtime routing/cache bridge glue still Delta-owned in progress; no new APK/device milestone. Latest Foldinstalleddf80, lastSamsungprefix27instructions.


Approved priority: single actual Terraria Foundation startup path. Typed ObjC slot transaction integrated/scoped3testsPASS including stale/unwritable preflight and rollback after partial write. Remaining Delta Foundation helper/cache/msgSend routing awaits genuine in-place ARC release upgrade coordinated with chained; worker owns narrow bridge replacement and executionservice variant. Thread worker independent publication groundwork; does not delay startup. No new game execution milestone.


User-requested Pixel9ProFold latest deployment COMPLETE: APK df80ab9efce250567bf80959786dfa93f384c31b301b99f15eb9a333c0e661a4 installed -r on192.168.0.22:41105 modelPixel9ProFold; installed base.apk SHA verified.268Rusttests PASS/0failed/3ignored,AndroidBUILDsuccessful, including3newTPIDRnative tests. All frozen ready patches included; unfinished actual Foundation startup routing/helper and typed slot transaction excluded. User manual testing; no game launch/rootdevice gameplay claim. Evidence pixel-fold-tests/fold9-latest-runtime-install.json.


Active next milestone: Delta assembling real Foundation startup helper and exact cache/msgSend routing; chained implementing transactional typed main-image NSBundle classrefs/selectorrefs with provider/addend/permission checks and rollback; Razer owns narrow TPIDR_EL0 wrapper API and unpublished-thread clean initialization. Root builder free, waiting READY before snapshot; preserved265test build log pixel-fold-tests/foundation-integrated-build.log. No device advancement claimed.


Foundation namespace/string/bundle patches now INTEGRATED:265 Rust tests PASS,0failed,3ignored; Android BUILD SUCCESSFUL. Real CPU bundle/string lifecycle regression passed, including actual owned class initialization/allocation, bundle singleton/path/string append and ARC disposal/reuse. Thread preparation native fixtures also pass. APK 9565d87b63838a688e6de2d05a199d56aa71506fa4c397d5c6a91616734b8a98 built but not installed; actual Terraria startup redirect transaction/helper remains worker-owned next. Do not claim first gamecall cleared; last Samsung27instructions unchanged. Evidence /tmp/playcover-foundation-build.log in WSL.


Scheduled pthread + real allocation-helper combined build PASS:260 tests enumerated (3 ignored), Android build successful. APK 1c63eb8ecb3382c7f87a2c10f30155b7ec3b549e04ac1116444c20a259b76d14 built, not installed yet; defer next device regression until startup routing/namespace changes arrive. No new Terraria execution milestone. All workers continue namespace/bundle services/thread creation preparation.


Scheduled pthread adapter integrated: scoped10tests PASS including native thread TLS/mutex isolation,4actual guest destructor callback passes with exiting identity and failure quarantine/no replay. Shared real ObjC allocation helper copied; combined Rust/Android build running. Namespace/bundle-method adapter still worker-owned unfinished; do not claim Terraria advanced. Last Samsung remains b2cf19f prefix27instructions.


Latest Samsung installed/hash-verified APK b2cf19f1e39bbf7b11309800c91f6f3acd9b053fdcb11d46c447551514e2e42b. Integrated registration/bundle/scheduler/conditions build passed247 Rust tests plus Android build. Samsung real entry-prefix regression unchanged27instructions then cached _objc_msgSend0x1800ba400;285bindings/14selected/36deferred. Evidence terraria-tablet-entry-prefix-scheduler.*; screenshot focusNotificationShade, log verifies diagnostic completion. No title/gameplay. Fold still previous0b1 build, manual-testing device unchanged this round. Workers active owned Foundation namespace/services and borrow-safe scheduler teardown; dynamic pthread adapter READY but pending latest teardown revision.


Combined registration/bundle/scheduler integration:247 Rust tests passed,0 failed,3 explicit-cache tests ignored. Includes genuine native CPU scalar/SIMD/PSTATE/SP/thread-register switching and guest TLS destructor before exit. Android packaging pending. Modules remain groundwork, not ordinary Terraria startup routing; current on-device boundary unchanged.


Current integration: original mapped-image ObjC registration, genuine bundle/load receipts and cooperative scheduler modules registered/copied into central build. Condition-variable integrated build previously passed234-test suite/Android; not yet deployed. Workers active on owned NSObject/NSBundle namespace, bundle method services and scheduler-current pthread adapter. Root central build running; new modules not yet proven on device. Last device prefix remains27instructions to _objc_msgSend, no title/gameplay.


Latest verified build 0b1a033cb2fc76d60812417511b38f1959178e3aaa07d58dc35ef1fe81aa5fe8 installed/hash verified Samsung and Pixel9ProFold. Integrated Objective-C initialization/execution, TLS and mutex tests plus Android build passed. Samsung prefix regression still27instructions, stops at exact cached _objc_msgSend 0x1800ba400 (verified original cache symbol). Execution services compiled/tested but not wired into ordinary Terraria startup. Agents continue mapped class registration, genuine bundle/load receipts and condition variables. Evidence terraria-tablet-entry-prefix-objc-mutex.*. No title/gameplay.


Verified Samsung entry-prefix milestone: APK a4fe82860b756ca845e8e9479542c2c039a8f8107a8845b68866aa4aa73cf5b0 executed 27 real Terraria main instructions and stopped before cached runtime PC 0x1800ba400. 285 bindings,14 selected imports,36 cached dependencies still uninitialized. Evidence pixel-fold-tests/terraria-tablet-entry-prefix.json/.log/.png. No title screen/gameplay. TLS and startup full Rust/Android build passed. Mutex services READY for next central build.


Startup/TLS integration in progress: frozen entry-prefix runner and real pthread key services registered for coordinated build. Three workers active: Objective-C initialization/dispatch, bundle metadata/load receipts, and pthread mutex behavior. Root owns build and Samsung test; no title screen or gameplay verified.


Updated 2026-10-07. Read this before rescanning source. Tasks are tracked in
`NEEDS_TO_DO.md`; project rules are in `AGENTS.md`. Check recorded source/build
identities when applying patches; a READY patch is not an installed fix.

## Current priority

Latest coordinated build b9555408fb5c5b8567505ebf6230396185d457ae00918e3ddf0bd705a0889751
installed/hashverified on both SamsungTabS11 and Pixel9ProFold (user manual
testing only on Fold). Integrated Rust suite and Android build passed.
Samsung ACTUAL selectedservice diagnostic PASS:14appimports routed; genuine
guesttrampolines executed ownedCFStringcreate/length/release andObjCnilretain.
No Apple/app initializers/main executed;36cacheddependencies uninitialized.
Evidence terraria-tablet-selected-services.json/.log. Diagnostic-only routing
is not enabled for ordinary game launch. Next real startuppolicy/TLS/threading
andobjectallocation integration required; no title screen/gameplay verified.

Latest APK f8b3d7076202c05eb8cd014fc47e903eaac86f9f8fef14f1f5cadb8681caa375
installed/hash-verified on TabS11 and Pixel9ProFold; user now owns Fold manual
testing, root uses Samsung only. Unified CF adapter2 actualbridge tests and full
Rust/Android build passed. iOS16 original cache44files/3,329,294,336bytes copied
to Tab `/sdcard/Android/data/org.touchhle.android.a64test/files/ios-runtime/cache`
with everyfileSHA verified. ACTUAL Android cacheprepare PASS:3ordinaryimages,
36cacheddependencies,285bindings,7auditedselectors;2733469696mappedbytes.
No initializers/main executed;36cacheddependencies remain uninitialized.
Evidence terraria-tablet-cache-prepare-actual.json/.log. Earlier
terraria-tablet-cache-prepare used filtered runtime_options and did not exercise
cachepreparation; diagnostic flags must use MainActivity extra_args.

All four agents resumed Terraria ARM64 runtime work: root integrates/tests/builds;
delta owns selected host-service bindings; chained owns real guest dealloc
coordination; CF worker owns unified string/UUID/array/data service adapter.
Root registered the six previously frozen CF/lifetime modules and starts the
coordinated integrated test/build. App Store deployed source is preserved.

App Store redesign deployed and verified on TabS11 + Pixel9ProFold, APK SHA
`a7f699ada6c64bc4b24732a884af7dd3d18d894517b15358a8991c25b0bdc484`.
Settings -> Apple account and connection contains endpoint/token/sign-in;
App Store browsing has purple cards/icons/developer/category/rating and separate
owned/store versions. Details expose current US catalog description, release
notes, minimum iOS/size/screenshots where available. Purchases append/deduplicate:
both devices verified50 ->100of1993, detail dialog and Settings route. Android
tests/build passed. Evidence apple-store-redesign-*-100.xml/.png,
store-*-details.png and apple-store-installed.json. No new owned download test.

App Store page added at user's request. Launcher has a separate App Store tab
with public Apple catalog search, and AppleStoreActivity connects to the local
ipatool companion for status/login/purchases/search/owned downloads. Credentials
are entered only in ipatool's computer prompt. Root companion runs on127.0.0.1:
18765; Windows forbids8765. Both requested devices provisioned with private
no-backup tokens and adb reverse18765. APK Android tests/build passed. TabS11
install hash/page/companion status verified; Pixel9ProFold install in progress.
Both requested devices now installed the same APK SHA256
`f218fd14f11e2218ef344ee6a00371491c0a3ca9383c9d2a057d7b6ce9c1abf7`.
App Store page and authenticated companion connectivity verified on both;
screenshots/XML: pixel-fold-tests/apple-store-tab* and apple-store-fold9*.
PC Apple login subsequently succeeded; unlocked companion reports authenticated
true, and TabS11 account page verified signed-in. Actual purchased endpoint
returns50 apps/page and1993 total. Authenticated download remains untested.
Redesign in progress: settings via Launcher Settings -> Apple account and
connection; App Store tab routes directly to browsing Activity with richer
catalog detail and purchased pagination. Downloaded IPA
ZIP is checked on PC and transfer SHA/size before Android import. Apple DRM
is preserved; no playback claim. See APPLE_STORE_DOWNLOADS.md and
APPLE_STORE_COMPANION.md. Process-death download resumption remains incomplete.

User requested iOS 17 and 18 firmware reserves. `tools/obtain_ios17_ios18.py`
started downloading Apple-hosted iPhone11 (iPhone12,1) iOS17.7 build21H16
(7,500,812,531 bytes), followed by iOS18.0. Version-isolated destinations are
`ios-runtime/firmware-ios17` and `ios-runtime/firmware-ios18`; provenance is
saved in each source.json. Download chunks resume; full published SHA256 is
checked. Neither archive is yet recorded as completed/extracted/integrated.
These newer device caches may contain arm64e/PAC requirements and must be
inspected before choosing them as execution providers.

Both reserves subsequently finished full published SHA256 verification:
iOS17.7 iPhone12,1 build21H16 and iOS18.0 iPhone12,1 build22A3354.
Archives are complete; not yet extracted or integrated.

Latest user selection: Terraria is the primary ARM64 game target ("Tesla" was
clarified as Terraria). Copied `C:\Users\Ben\Desktop\ipa\Terraria_4.5.0.ipa`
over authorized USB to Samsung Tab S11 SM-X930, serial R52Y8066STA, ADB 5038.
All 189,283,070 bytes verified by SHA-256
`5cf22795cb7ec546823ff38e9e6f8136411ba39421704c024a80893ca023b806`.
Destination: `/sdcard/Android/data/org.touchhle.android.a64test/files/touchHLE_apps/Terraria_4.5.0.ipa`.
Evidence: `pixel-fold-tests/terraria-tablet-copy.json`. Copy only; no launch or
gameplay claim. Do not repeat this transfer unless its hash changes.

The user wants agents working on real 64-bit game support, with the supplied
modern Resident Evil 4 tested on the Samsung Tab S11. Root coordinates builds
and tablet input. Delta owns resolver/commpage work; chained owns the Apple
ARM64 Objective-C bridge groundwork; the Razer agent completed web/GitHub research
and now owns tablet startup inspection and the updated-build test.
The agent thread limit prevented creating a fourth worker, so the available
Razer agent was reassigned. Its old RE4 audio task is complete.

## Supplied modern Resident Evil 4: verified IPA facts

- File: `C:\Users\Ben\Downloads\resident-evil-4-v1.0.5-iosvizor.ipa`.
- 476,169,240 bytes; SHA-256
  `4c1b8d45c5ff93862b455dae382f8fed2aa530abb2c1980fffa57b8f26558bb2`.
- Thin ARM64 Mach-O, bundle `jp.co.capcom.RE4US`, version 1.0.5.
- Info.plist minimum iOS 17.0; encryption command cryptid 0.
- This is a different app from the older ARM32 RE4 that works on the Razer.
- Direct dependencies include UIKit, Foundation, libobjc, Metal, MetalFX,
  QuartzCore, CoreFoundation, AVFoundation/AVFAudio, AudioToolbox/CoreAudio,
  CoreMedia/CoreVideo/CoreGraphics, GameController, GameKit, CoreHaptics,
  StoreKit, WebKit, Security, SystemConfiguration, libc++ and libSystem.
- Actual libobjc imports include msgSend/msgSendSuper2, class/protocol lookup,
  alloc/alloc_init, ARC retain/release and return handshakes, strong/weak storage,
  autorelease pools, property access, synchronization and enumeration mutation.
- Agent metadata audit found 31 class-list entries, 23 protocols, 1,156 selector
  references and 204 class references. Metadata parsing alone is not dispatch.
- Raw dependency/build-command evidence:
  `pixel-fold-tests/re4-modern-requirements.json`.
- Tablet import completed with SHA verification. Baseline screenshot shows
  `Could not run ARM64 app: invalid initializer array alignment or size`.
  `first_panic: null` in the JSON is not success; the screenshot contains the
  failure. Evidence: `pixel-fold-tests/re4-modern-tablet.png/.json/.log`.
- Updated APK `b149b0f1e1688d76b563216593041a95b963f1f7f3bacb9d8589f73883ad1e15`
  installed successfully preserving data. Wireless `192.168.0.56:45601` then
  went offline/refused connections before updated launch. No tablet USB serial
  appeared on native ADB 5038. Reconnect, verify installed hash and retest.
  USB serial `R52Y8066STA` subsequently became available on port 5038. Updated
  b149 installed hash verified; startup retest reproduces the same initializer
  error (`re4-modern-tablet-updated.png/.json/.log`). Returned to LauncherActivity.
  Root's initializer-cap fix was subsequently verified on USB with installed
  APK `9001966f03e4faf8e3eea773ded70d9cb0074efa5c1616c035461e9566586723`.
  The next actual failure is missing ARM64 CoreFoundation runtime dependency:
  `/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation` needs
  a runtime root (`--a64-runtime=PATH`). Evidence after 40 seconds:
  `pixel-fold-tests/re4-modern-tablet-init-fixed.png/.json/.log`.
  IPA hash remains verified; no retransfer. Returned to LauncherActivity.
  No modern RE4 menus or gameplay are verified.

## ARM64 runtime: established gaps and file ownership

- `src/a64_bridge.rs`: READY bounded nested guest-call and registered host
  service infrastructure. Explicit x0..x7/v0..v7/stack/x8 inputs and x0/x1 plus
  v0..v3 results; RX entry checks, separate RW stack, full CPU context restore
  and tick limit. Services use registered guest SVC trampolines and bounded
  guest memory helpers, never host-pointer casts. Four synthetic machine-code
  tests passed in the coordinated Rust library test and Android build. This does
  not initialize Apple frameworks or enable nonnil Objective-C dispatch.

- `touchHLE-src/src/a64.rs`: experimental ARM64 path; full Apple runtime is
  not initialized. No working guest ARM64 Objective-C host trap/dispatch bridge
  exists. Keep the runtime gate honest; version reporting does not add APIs.
- `src/a64_exports.rs`: old `resolve_symbol` rejects StubAndResolver exports.
  Delta's READY addition exposes `resolve_symbol_with_resolver` with a caller
  callback receiving owner/name/stub/resolver. Default rejection remains.
- `src/a64_cache_resolver.rs`: READY `AuditedResolvers` validates original
  cache UUIDs, executable mappings, scratch page and bounded CPU execution.
  It accepts only verified iOS 11 OSAtomicAdd32 and a finite iOS 16 whitelist
  of actual Terraria imports (dispatch/atomic/pthread/spin/unfair-lock selectors),
  each bound to exact UUID, addresses and instruction bytes. It restores CPU
  context and checks returned RX
  addresses. It does not execute resolved target functions or initialize iOS.
- `src/a64_commpage.rs`: CPU-capability/commpage support associated with the
  audited resolver path; preserve read-only checks and accurate advertised bits.
- `src/a64_cache_symbols.rs`: READY mutable CPU callback API follows canonical
  reexports and retains the defining weak strength. The read-only API still
  rejects resolvers and never executes code.
- `src/a64_linker.rs` and `src/a64_cache_linker.rs`: READY production binding
  integration supplies the audited service and maps its scratch only after
  app images and the normal stack are placed. Mapping overlap/unknown UUIDs
  fail; successful resolver results are cached only after byte and commpage
  validation. Binding summaries count actual capability-resolver executions.
  The full Apple runtime initialization/main execution gate remains closed.
  Integrated Rust tests and Android build passed. The original iOS 16 desktop
  probe also passed, returning `0x18db355c0` without executing that target,
  app initializers or main. Real game binding/startup remains separate.
- Genuine iOS 16 dispatch_once_f resolver address `0x18db916e0` reads capability
  byte `0xfffffc023`, bit 2; non-LSE target `0x18db355c0`, LSE `0x18db64d54`.
  Stub `0x18dba17cc` is a GOT branch. One external symbol tool printed resolver
  and stub swapped; export parsing independently verified the actual addresses.
  Evidence: `ios-runtime/ios16-dispatch-once-resolver-analysis.json`.
- Resolver tests include mutation refusal, canonical weak-strength preservation,
  read-only audit refusal, cached-repeat validation and synthetic real instruction
  patterns. Original-cache probe evidence is
  `pixel-fold-tests/ios16-audited-dispatch-resolver-desktop.json/.log`.
  Native CLI SHA-256 `c6a4b1bf1df7836db7aa8b640098a631d94150cca33f5ad48de5a130de4dc98d`;
  main cache SHA-256 `64d392d818ad12cc34bd9a79ba935c1831b0f29a11cda6a0c1a1c379220bb2c1`.
  Reproduce with `tools/test_a64_audited_resolver.py` in WSL.
- ARM64 CPU wrapper exposes general registers/SP/PSTATE but lacks SIMD register
  get/set. objc_msgSend(nil) must clear the correct SIMD return registers too;
  blanket fake nil results are not a valid nonnil Objective-C implementation.
- Chained is implementing strict Apple ARM64 class/method metadata parsing as
  the next bridge prerequisite. GNUstep libobjc2 is not a drop-in replacement
  for Apple-compiled ObjC2 metadata; host Foundation alone cannot run the guest.
- READY parser `src/a64_objc.rs` is registered as `a64::objc_metadata` in
  `a64.rs`. It handles LP64 class_ro and absolute/relative method lists with
  bounded guest reads, preserves 64-bit addresses and rejects unresolved
  superclass/realized metadata. Standalone and integrated tests passed;
  post-fixup guest-reader wiring is still required. This is not dispatch.

## Research already established: do not rediscover blindly

- User's backup library is now `C:\Users\Ben\Desktop\ipa`; the modern RE4
  file moved there from Downloads. Index: `ios-runtime/IPA_LIBRARY_INDEX.md`
  and `.json`, generated by `tools/index_ipa_library.py`. 23 IPAs contain ARM64;
  17 are ARM64-only and six also contain ARM32. Exact names/versions/minimum iOS,
  sizes/modified times and bounded encryption checks are recorded. No bulk
  device import or file modification occurred. Do not repeat this inventory
  unless files changed or a targeted runtime test needs further information.

- Existing overview: `ios-runtime/GITHUB_RUNTIME_RESEARCH.md`.
- GNUstep Android tools/Foundation/libobjc2: useful reference or host components,
  but guest ABI, memory, thread and run-loop bridges remain necessary.
- Darling/Chameleon: relevant implementation references, not ready Android iOS
  runtime replacements. Swift Android ELF cannot directly run Apple Mach-O Swift.
- PlayCover relies on macOS Apple frameworks; it is not the missing Android
  runtime. MoltenVK translates Vulkan to Metal, the opposite direction needed.
- Research agent identified locally available Indium (0BSD), a Metal-like API
  on Vulkan 1.3, including Iridium AIR-to-SPIR-V translation; darling-metal
  wrappers are MPL-2.0. Exact revisions and Android applicability are being
  checked. Neither currently establishes RE4 MetalFX/game compatibility.
- Primary links/licenses and exact reusable code are required before integration;
  do not treat a research candidate as a installed dependency or working game.
- Completed research: `ios-runtime/ARM64_COMPONENT_RESEARCH_2026-10-07.md`.
  MIT-licensed libffi v3.8.0 and MetalLibraryArchive sources were acquired in
  isolated paths; pins are in `runtime-sources/arm64-research-manifest.json`.
  They are not built or integrated. The audited darling-metal inventory lacks
  a MetalFX implementation, which this modern RE4 directly requires.

## Working baseline and build identities

- Windows source: `touchHLE-src` (shared dirty tree; preserve other edits).
- WSL integration: `/home/ben/touchHLE-a64-integration`.
- WSL target: `/home/ben/touchHLE-a64/target`.
- Test Android package: `org.touchhle.android.a64test`; original app preserved.
- `tools/build_repository_ui.sh`: copies Android UI/resources and tests, runs
  Gradle. `PLAYCOVER_UI_ONLY=1` is valid only when native code is unchanged.
- `tools/build_bitmap_audio.sh`: scoped native copy + Rust tests + Android build;
  update its copy list deliberately for new READY runtime files.
- Only one builder may use the WSL integration at a time. Agents must send READY
  scopes; root copies, tests, builds, installs and releases device input.
- APK `31e4bdd3650a0bfc07f27205bade98d70161338b4001bab51ad2d9b474b13996`:
  architecture labels plus Sonic segmented-control patch; installed Fold.
  Preserved as `pixel-fold-tests/playcover-architecture-labels.apk`.
- APK `b149b0f1e1688d76b563216593041a95b963f1f7f3bacb9d8589f73883ad1e15`:
  coordinated native tests/Android build passed, including audited resolver
  integration, ARM64 ObjC metadata parser and preserved Sonic XRGB1555 patch.
  Saved as `pixel-fold-tests/playcover-a64-resolver-objc.apk`; device installation
  was not yet reported when the original-cache desktop probe completed.
- APK `e2cb49452e461d0602784ce303557e9f68d85d7f5662519ca3bc6f84fd570ae8`:
  working RE4 audio correction, rotation menu and Rock Band measurement logs;
  installed Razer and Tab as last verified. Preserved as
  `pixel-fold-tests/playcover-rotation-audio-sonic.apk`.
- APK `b149b0f1e1688d76b563216593041a95b963f1f7f3bacb9d8589f73883ad1e15`:
  resolver production callback wiring, audited iOS 11/16 resolver support,
  registered Objective-C metadata parser and Sonic XRGB1555 patch. Integrated
  Rust tests and Android build passed. Preserved as
  `pixel-fold-tests/playcover-a64-resolver-objc.apk`; not yet installed.
  Real original-cache resolver probe is delegated to Delta; do not treat this
  as complete Apple framework initialization or commercial game support.

## Device facts

| Device | Identity | Last working ADB address |
| --- | --- | --- |
| Pixel Fold generation 1 | felix | 192.168.0.51:35429 |
| Galaxy Tab S11 Ultra | SM-X930 / gts11uwifi | 192.168.0.56:45601 |
| Pixel 9 Pro Fold | comet | 192.168.0.22:41105 |
| Razer Edge 5G | RZ45-0460 | USB 602305N15309192, native Windows ADB port 5038 |

Wireless ports can change; verify identity. Default Windows port 5037 is the WSL
relay, not native USB ADB. Port 5038 sees USB and paired wireless devices. Do not
reset servers or copy keys merely to rediscover an already connected device.

Tablet update: USB serial `R52Y8066STA` is now authorized on native Windows ADB
port 5038. Prefer this USB connection; old wireless port 45601 stopped working
after the restart. Modern RE4 import is complete and hash verified.

## New RE4 loader/ABI fixes in progress

- Actual first tablet crash: `invalid initializer array alignment or size`,
  shown in `pixel-fold-tests/re4-modern-tablet.png` (not a `Panic at` log line).
  The executable has 12,868 aligned initializer pointers, 102,944 bytes, in
  `__DATA_CONST/__mod_init_func`; the old parser capped sections at 4,096.
- Root changed the finite cap to 16,384 entries, preserving pointer alignment,
  file-backed segment bounds and the separate initializer execution tick budget.
  Large valid table, over-limit table and out-of-file-backed-range tests pass.
  This fix is installed in APK 9001966f (full hash below); tablet retest is active.
- Root added checked guest SIMD-vector accessors in
  `src/cpu/dynarmic_wrapper/a64.cpp` and `a64.rs`. Guest STR Q0 / LDR Q31
  roundtrip and complete CPU-context restore tests pass. Real `MOVI d0,#0`
  test confirms both low/high vector lanes clear; correct ObjC nil returns must
  likewise clear v0-v3, not just scalar x0/x1. These helpers are prerequisites,
  not complete callable Objective-C dispatch.

Latest tablet APK: `9001966f03e4faf8e3eea773ded70d9cb0074efa5c1616c035461e9566586723`,
preserved as `pixel-fold-tests/playcover-a64-re4-init-fix.apk`, installed over
authorized USB `R52Y8066STA` / native ADB 5038. Integrated Rust tests and Android
build passed. Includes initializer cap fix, SIMD accessors, seven audited cached
resolver selectors, and the registered class/selector dispatch planner. Planner
does not itself execute guest IMPs or supply Apple frameworks. Actual modern RE4
startup retest is pending; do not infer gameplay from successful installation.

## Other apps: known outcomes

- Older ARM32 Resident Evil 4 on Razer: Basic Training movement works; distorted
  sound fixed and user confirmed. Actual guest callback writes duplicated
  little-endian stereo samples despite malformed planar ASBD. Scoped correction
  normalizes stride 4 and clears the inconsistent planar flag; keep other apps'
  genuine planar formats unchanged. Do not undo this to solve modern RE4.
- Rock Band on Tab: song gameplay works; per-app scale 2 stored in user options.
  Real renderbuffer 320x480 -> 640x960; same-build samples ~60 FPS. Rotation-menu
  checkbox and Exit IPA verified. Full-song completion/audio quality unverified.
  `pixel-fold-tests/rockband-upscale-tablet.json` is the comparison evidence.
- Rock Band DLC audit: exactly 20 bundled a0..a19 song groups, nine assets each;
  no extra playable local songs found. Store uses package/download routines and
  external EA endpoints. No local enable toggle is justified. Evidence:
  `pixel-fold-tests/rockband-local-content-audit.json`.
- Sonic 1 Fold: bitmap/contentsRect/segmented-control startup gaps cleared.
  READY `cg_image.rs` patch implements evidenced XRGB1555 framebuffer
  (320x224, 5 bits/component, 16 bits/pixel, bitmap info 0x1006), not RGB565.
  Two scoped conversion tests passed; this patch is not yet installed. Preserve
  it while ARM64 is prioritized. Menus/gameplay remain unverified.
- Tapped Out 4.28.0 Razer: both ARMv7 and ARM64, iOS 7 minimum, cryptid 0.
  Actual ARM32 startup failed at `_objc_lookUpClass` before menu. Imported IPA
  preserved, Razer returned to launcher. Evidence `tapped-out-razer.json/.log`.
- Quest/Terraria/Delta: real ARM64 gameplay unverified. Existing app requirement
  reports and resolver/commpage probes are useful evidence, not proof of a full
  Apple runtime. Do not rescan all IPAs when one targeted new failure suffices.

## UI code map

- `android/.../LauncherActivity.kt`: Installed rows now show 32-bit / 64-bit /
  architecture unknown; metadata derives from `IpaInfo.kt` selected Mach-O slice.
  Actual Fold labels verified for Sonic 1/2 and 64-bit Hollow Knight/Spotube.
- `MainActivity.java` + `src/window.rs`: top-left IPA menu; live Match device
  rotation via atomic JNI request, SDL rotation and existing touch transform;
  disabling returns future rotation control to the IPA. Exit kills only the
  isolated own game process and returns to the modern launcher.
- `RepoSources.kt`, `IpaDownloads.kt`, `IpaDownloadService.kt`: persistent repo
  pages/icons/search/category filters and two durable resumable download slots.
  Modern launcher/settings and home-screen shortcut behavior already implemented.

## Documentation maintenance

Terraria UnityFramework explicit-root preparation also PASS (2026-10-07),
using an unchanged provider/full copied bundle and synthetic load scaffold.
Five ordinary images and 64 cached dependencies resolved 5,964 imports
(1,603 against cache). All 916 application initializer functions remain
deferred; 37 exact audited capability selectors executed. The finite resolver
table contains only actual Unity/PlayFab platform/pthread/dispatch imports,
with original-cache instruction evidence in
`pixel-fold-tests/terraria-capability-selector-audit.json`. Scoped tests execute
every exact selector body and preserve CPU context. An ordinary-only weak
coalescing fix preserves first-weak/first-strong local load order; cached
ambiguity refusal remains. Latest Unity test native CLI SHA-256:
`593bfa35b625e39a45a5efee85b95aaeda63f38edf48a5c41e9bf4a46ba6c95e`.
Evidence: `terraria-unityframework-ios16-scaffold-prepare-desktop.json`.
Main, AppleCoreNative, GameKitWrapper and PlayFab prepare also passed their
prior targeted runs. No app/Apple initializer/main or gameplay has executed.
Incremental `a64_cache_resolver.rs` and `a64_linker.rs` are READY for Android.

Terraria original iOS 16 production binding retest (2026-10-07): PASS,
exit 0, using unchanged IPA SHA-256
5cf22795cb7ec546823ff38e9e6f8136411ba39421704c024a80893ca023b806.
Three ordinary images and 36 cached dependencies resolved 285 import records
and wrote application binding slots. Seven exact byte-audited capability
selectors executed: dispatch_once_f, pthread mutex init/lock/unlock,
os_unfair_lock lock/unlock, and voucher_adopt. Scoped resolver tests passed
(3 tests, including mixed-selector cache isolation, modified-byte rejection
and CPU context restoration). Current desktop CLI SHA-256:
91cdf95eae41e2ffd0da616c7c803449a47c19125865ec21747bc3379786fa3c.
Incremental source a64_cache_resolver.rs is READY for coordinated Android
build. No app or Apple initializer/main ran; full runtime execution remains
gated. Evidence: pixel-fold-tests/terraria-ios16-audited-prepare-desktop.json
and .log; reproduce tools/test_terraria_audited_binding.py after the genuine
resolver probe. Exact selector analysis is in
 ios-runtime/ios16-lock-resolver-analysis.json. No gameplay claim is justified.

Record new findings here with paths and actual test evidence. Update installed
APK identity only after successful installation. Mark probes, READY source,
research candidates and verified gameplay distinctly. Rescan only a changed or
unresolved area; do not repeat known inventories and dependency lists.
