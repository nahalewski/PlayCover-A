# Regression run

Device R52Y8066STA. Each row is updated by tools/regress.ps1 when that game is run (last write 2026-10-07 21:03, 25 s per game). 'runs' = still foreground with no panic, not proof it plays correctly.

| IPA | Result | Log lines | First problem |
|---|---|---|---|
| Amateur Surgeon 1.0.1.ipa | runs | 190 |  |
| Angry Birds 8.0.3.ipa | crashes | 989 | Panic at src/libc/stdlib.rs:335:5: App called abort() [unimplemented calls: _host_info, _glBlendEquation, _mkstemp, _glGetActiveUniform] |
| Angry Birds Rio HD 1.1.0.ipa | runs | 119 |  |
| Call of Duty World At War Zombies HD 1.1.0.ipa | runs | 91 |  |
| Call of Duty Zombies 1.3.0.ipa | exited | 114 | left foreground / process ended |
| Candy Crush Saga 1.19.0.ipa | crashes | 860 | Panic at src/libc/stdio/printf.rs:967:34: not implemented: length_modifier Some("ll") [unimplemented calls: _CGColorSpaceGetNumberOfComponents, _glVertexAttrib3fv, _glVertexAttrib4fv, _glVertexAttrib2fv] |
| Civilization.Revolution.v2.1.2.ipa.ipa | runs | 144 |  |
| COMMAND & CONQUER RED ALERT (World) 1.7.0.ipa | runs | 1391 |  |
| Dead Space 1.0.3.ipa | runs | 1639 |  |
| Devil May Cry 4 Refrain 1.01.00.ipa | runs | 93 |  |
| FINAL FANTASY VII G-BIKE 1.1.0.ipa | crashes | 173 | Panic at src/mem.rs:357:9: Attempted null-page access at 0x0 (0x4 bytes) |
| FlappyBird_1.2.ipa | runs | 97 |  |
| jp.pokemon.pokemonquest-1.0.7-Decrypted.ipa | runs | 13 |  |
| LEGO Harry Potter Years 1-4 2.4.ipa | runs | 154 |  |
| MARVEL VS CAPCOM 2 1.00.00.ipa | crashes | 75 | Panic at src/environment/mutex.rs:172:21: Attempted to lock non-error-checking mutex #40 for thread 1, already locked by same thread! [unimplemented calls: _pthread_attr_getschedparam] |
| Mega Man II 1.5.1.ipa | crashes | 42 | Panic at src/frameworks/uikit/ui_image.rs:127:43: called `Result::unwrap()` on an `Err` value: "unknown image type" |
| Minecraft PE 1.1.7.ipa | crashes | 3634 | Panic at src/libc/sysctl.rs:310:17: not implemented: Unknown sysctlbyname parameter machdep.cpu.vendor! [unimplemented calls: ___sincosf_stret, __ZNSt9bad_allocC2Ev] |
| Mirror's Edge 1.4.72.ipa | exited | 190 | left foreground / process ended |
| Modern Combat 5 Blackout 1.0.1.ipa | runs | 25 |  |
| N.O.V.A.2.HD 1.1.7.ipa | crashes | 172 | Panic at src/gles/gles_generic.rs:284:9: not implemented |
| NFSU 1.2.5.ipa | crashes | 292 | Panic at src/frameworks/audio_toolbox/audio_file.rs:305:17: not yet implemented |
| PAC-MAN REMIX 1.0.0.ipa | crashes | 75 | Panic at src/frameworks/foundation/ns_run_loop.rs:105:5: assertion failed: msg![env; mode isEqualToString:default_mode] // [unimplemented calls: _ExtAudioFileWrapAudioFileID] |
| Plants vs. Zombies 2 126.ipa | crashes | 2216 | Panic at src/mem.rs:357:9: Attempted null-page access at 0x10 (0x1 bytes) [unimplemented calls: _sigaltstack, _task_swap_exception_ports, _pthread_attr_setstack, _NXGetLocalArchInfo, _NXGetArchInfoFromCpuType, __dyld_register_func_for_add_image] |
| Playboy 1.1.45.ipa | runs | 43 |  |
| Pocket God 1.39.ipa | runs | 2567 |  |
| Prince of Persia Warrior 1.0.8.ipa | crashes | 179 | Panic at src/environment.rs:1704:25: not implemented: TODO: implement exit routines for threads! [unimplemented calls: _pthread_kill] |
| Rock Band 1.1.38.ipa | runs | 234 |  |
| Scribblenauts Remix 7.8.ipa | crashes | 505 | Panic at src/mem.rs:357:9: Attempted null-page access at 0x0 (0x4 bytes) |
| Secret of Mana 1.0.0.ipa | runs | 105 |  |
| Sonic 1 1.2.6.ipa | runs | 193 |  |
| Sonic 2 1.2.2.ipa | runs | 220 |  |
| Sonic 20th Anniversary 1.1.0.ipa | crashes | 112 | Panic at src/frameworks/uikit/ui_view.rs:527:14: insertion index (is 1) should be <= len (is 0) |
| Sonic CD 2.0.0.ipa | crashes | 340 | Panic at src/mem.rs:357:9: Attempted null-page access at 0x4 (0x4 bytes) [unimplemented calls: _CFStringLowercase, _NXGetLocalArchInfo, _SecCertificateCreateWithData, _open_dprotected_np, __dyld_register_func_for_add_image, __dyld_register_func_for_remove_image] |
| Sonic The Hedgehog 4 Episode I 1.2.ipa | crashes | 69 | Panic at src/frameworks/foundation/ns_string.rs:902:14: not implemented: 2147483649 |
| Spore Origins 1.0.7.ipa | runs | 221 |  |
| STREET FIGHTER IV 1.00.06.ipa | crashes | 141 | Panic at src/mem.rs:357:9: Attempted null-page access at 0x0 (0x1 bytes) [unimplemented calls: _object_getClassName] |
| Tony.Hawk's.Pro.Skater.2 v1.2.1.ipa | exited | 221 | left foreground / process ended |
| Worms 2 Armageddon 1.06.ipa | crashes | 149 | Panic at src/objc/methods.rs:35:29: not implemented [unimplemented calls: _class_addMethod] |
| Zenonia 1.0.ipa | crashes | 75 | Panic at src/mem/allocator.rs:636:9: assertion failed: size.is_multiple_of(PAGE_SIZE) && size >= PAGE_SIZE [unimplemented calls: _CFURLCreatePropertyFromResource, _CFReadStreamCreateWithFile, _CFReadStreamOpen] |
| ZENONIA 2 1.9.ipa | crashes | 106 | Panic at src/libc/stdlib.rs:353:5: App assertion failed: (0), function MC_knlGetResource, file /Users/allmypassion/Desktop/Projects/Zenonia/Zenonia2/Zenonia2/WI [unimplemented calls: _CFReadStreamCreateWithFile, _CFURLCreatePropertyFromResource, _CFReadStreamOpen] |
