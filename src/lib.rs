/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! touchHLE is a high-level emulator (HLE) for early iOS apps.
//!
//! In various places, the terms "guest" and "host" are used to distinguish
//! between the emulated application (the "guest") and the emulator itself (the
//! "host"), and more generally, their different environments.
//! For example:
//! - The guest is a 32-bit application, so a "guest pointer" is 32 bits.
//! - The host is a 64-bit application, so a "host pointer" is 64 bits.
//! - The guest can only directly access "guest memory".
//! - The host can access both "guest memory" and "host memory".
//! - A "guest function" is emulated Arm code, usually from the app binary.
//! - A "host function" is a Rust function that is part of this emulator.

// Allow the crate to have a non-snake-case name (touchHLE).
// This also allows items in the crate to have non-snake-case names.
#![allow(non_snake_case)]
// The documentation for this crate is intended to include private items.
// rustdoc complains about some public macros that link to private items, but
// we're forced to make those macros public by the weird macro scoping rules,
// so this warning is unhelpful.
#![allow(rustdoc::private_intra_doc_links)]

#[macro_use]
mod log;
#[cfg(feature = "a64")]
mod a64;
mod abi;
mod audio;
mod bundle;
mod cpu;
mod debug;
mod dyld;
mod environment;
mod font;
mod frameworks;
mod fs;
mod gdb;
mod gles;
mod image;
mod libc;
mod licenses;
mod mach_o;
mod matrix;
mod mem;
mod objc;
mod options;
mod paths;
mod stack;
mod window;

// Environment is used very frequently used and used to be in this module, so
// it is re-exported to avoid having to update lots of imports. The other things
// probably shouldn't be, but they need a new home (TODO).
// Unlike its siblings, this module should be considered private and only used
// via re-exports.
use environment::{Environment, MutexId, MutexType, ThreadId, PTHREAD_MUTEX_DEFAULT};

use std::path::PathBuf;

pub use touchHLE_version::*;

/// This is the true entry point on Android (SDLActivity calls it after
/// initialization). On other platforms the true entry point is in src/bin.rs.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "C" fn SDL_main(
    _argc: std::ffi::c_int,
    _argv: *const *const std::ffi::c_char,
) -> std::ffi::c_int {
    // Rust's default panic handler prints to stderr, but on Android that just
    // gets discarded, so we set a custom hook to make debugging easier.
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s
        } else {
            "(non-string payload)"
        };
        if let Some(location) = info.location() {
            echo!("Panic at {}: {}", location, payload);
        } else {
            echo!("Panic: {}", payload);
        }
    }));

    // The Java side (MainActivity.getArguments) passes the app to run and any
    // options as arguments. With no arguments, this brings up the built-in app
    // picker.
    let mut args = vec![String::new()]; // stands in for argv[0]
    for i in 1.._argc.max(1) {
        let arg_ptr = unsafe { *_argv.add(i as usize) };
        if !arg_ptr.is_null() {
            let arg = unsafe { std::ffi::CStr::from_ptr(arg_ptr) };
            args.push(arg.to_string_lossy().into_owned());
        }
    }
    match main(args.into_iter()) {
        Ok(_) => echo!("touchHLE finished"),
        Err(e) => echo!("touchHLE errored: {e:?}"),
    }
    0
}

const USAGE: &str = "\
Usage:
    touchHLE [PATH] [OPTIONS]

PATH should be a path to a .app bundle or .ipa file.

If no app path or special option is specified, a GUI app picker is displayed.

Special options:
    --a64-selftest
        Test the experimental ARM64 CPU and standalone Mach-O loader.

    --a64-run=PATH
        Run an experimental ARM64 Mach-O, including supported local dylibs.

    --a64-runtime=PATH
        Read ARM64 runtime dependencies from this filesystem root.
        Extracted Apple shared-cache images still require cache/runtime support.

    --a64-cache-info=PATH
        Validate an original ARM64 dyld cache and describe its sparse mappings.

    --a64-cache-map-test=PATH
        Privately map and decode cache slide information without executing it.

    --a64-cache-import-test=PATH
        Check an ARM64 app's direct imports against this original cache.
        Reports symbol availability without running the app or Apple code.

    --a64-cache-prepare=PATH
        Map an original ARM64 cache and bind app imports without executing
        Apple initializers or main; unsupported runtime requirements fail.

    --a64-legacy-cpp-prepare=PATH
        Validate the original iOS 11 legacy C++ provider and bind app imports
        against its coherent cache; does not execute initializers or main.

    --a64-cache-image-info-prepare=PATH
        Bind against an original cache with explicit loader-owned image info.
        Reports mapped images; does not execute initializers or main.

    --a64-cache-session-test=PATH
    --a64-cache-session-image-info-test=PATH
        Test original libSystem initialization with retained process state.
        Does not establish full runtime readiness or execute app main.

    --a64-cache-entry-prefix-test=PATH
        Single-step a diagnostic ARM64 app entry prefix with owned services.
        Stops before uninitialized cached code; does not establish app startup.

    --a64-cache-resolver-test=PATH
        Test the audited original-cache atomic resolver with a virtual commpage.

    --help
        Display this help text.

    --copyright
        Display copyright, authorship and license information.

    --info
        Print basic information about the app bundle without running the app.
";

#[cfg(feature = "a64")]
fn read_a64_runtime(root: Option<&std::path::Path>, dependency: &str) -> Result<Vec<u8>, String> {
    let root = root.ok_or_else(|| {
        format!("ARM64 dependency {dependency} needs a runtime root; use --a64-runtime=PATH")
    })?;
    if root.join(".shared-cache-exports").is_file() {
        return Err(format!(
            "{dependency}: extracted shared-cache libraries require original cache mappings and iOS runtime services; ARM64 shared-cache loading is not implemented"
        ));
    }
    let relative = dependency.trim_start_matches('/');
    if relative
        .split('/')
        .any(|part| part == ".." || part.contains('\\') || part.contains(':'))
    {
        return Err("Invalid ARM64 runtime dependency path".into());
    }
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let path =
        std::fs::canonicalize(root.join(relative)).map_err(|e| format!("{dependency}: {e}"))?;
    if !path.starts_with(&root) {
        return Err("ARM64 runtime dependency escapes runtime root".into());
    }
    std::fs::read(path).map_err(|e| format!("{dependency}: {e}"))
}

pub fn main<T: Iterator<Item = String>>(mut args: T) -> Result<(), String> {
    echo!(
        "touchHLE {}{}{} — https://touchhle.org/",
        branding(),
        if branding().is_empty() { "" } else { " " },
        VERSION,
    );
    if GITHUB_RUN_ID.is_some() && !branding().is_empty() {
        echo!(
            "Built from branch {:?} of {:?} by GitHub Actions workflow run {}/{}/actions/runs/{}.",
            GITHUB_REF_NAME.unwrap(),
            GITHUB_REPOSITORY.unwrap(),
            GITHUB_SERVER_URL.unwrap(),
            GITHUB_REPOSITORY.unwrap(),
            GITHUB_RUN_ID.unwrap()
        );
    }
    echo!();

    {
        let base_path = paths::user_data_base_path();
        log!("Base path for touchHLE files: {}", base_path.display());
        paths::prepopulate_user_data_dir();
    }

    let _ = args.next().unwrap(); // skip argv[0]

    let mut bundle_path: Option<PathBuf> = None;
    let mut just_info = false;
    let mut option_args = Vec::new();
    let mut options = options::Options::default();
    let mut app_args = None::<Vec<String>>;
    let mut a64_run = None::<PathBuf>;
    let mut a64_runtime = None::<PathBuf>;
    let mut a64_cache_info = None::<PathBuf>;
    let mut a64_cache_map_test = None::<PathBuf>;
    let mut a64_cache_import_test = None::<PathBuf>;
    let mut a64_cache_prepare = None::<PathBuf>;
    let mut a64_legacy_cpp_prepare = None::<PathBuf>;
    let mut a64_image_info_prepare = None::<PathBuf>;
    let mut a64_cache_session_test = None::<PathBuf>;
    let mut a64_cache_session_image_info_test = None::<PathBuf>;
    let mut a64_cache_services_test = None::<PathBuf>;
    let mut a64_cache_entry_prefix_test = None::<PathBuf>;
    let mut a64_cache_initializer_test = None::<PathBuf>;
    let mut a64_cache_resolver_test = None::<PathBuf>;

    for arg in args {
        if let Some(ref mut app_args) = app_args {
            app_args.push(arg);
        } else if arg == "--args" {
            app_args = Some(Vec::new());
        } else if arg == "--help" {
            echo!("{}", USAGE);
            echo!("{}", options::OPTIONS_HELP);
            return Ok(());
        } else if arg == "--copyright" {
            echo!("{}", licenses::get_text());
            return Ok(());
        } else if arg == "--info" {
            just_info = true;
        } else if arg == "--a64-selftest" {
            #[cfg(feature = "a64")]
            return a64::selftest();
            #[cfg(not(feature = "a64"))]
            return Err("ARM64 support requires a build with --features a64".into());
        } else if let Some(path) = arg.strip_prefix("--a64-run=") {
            a64_run = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-runtime=") {
            a64_runtime = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-info=") {
            a64_cache_info = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-map-test=") {
            a64_cache_map_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-import-test=") {
            a64_cache_import_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-prepare=") {
            a64_cache_prepare = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-legacy-cpp-prepare=") {
            a64_legacy_cpp_prepare = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-image-info-prepare=") {
            a64_image_info_prepare = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-session-test=") {
            a64_cache_session_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-session-image-info-test=") {
            a64_cache_session_image_info_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-services-test=") {
            a64_cache_services_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-initializer-test=") {
            a64_cache_initializer_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-entry-prefix-test=") {
            a64_cache_entry_prefix_test = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--a64-cache-resolver-test=") {
            a64_cache_resolver_test = Some(PathBuf::from(path));
        // Parse an option and store a backup in option_args so that we can
        // reapply them after file options are loaded. This ensures that
        // command line options take precedence over file options.
        } else if options.parse_argument(&arg)? {
            option_args.push(arg);
        } else if bundle_path.is_none() {
            bundle_path = Some(PathBuf::from(arg));
        } else {
            echo!("{}", USAGE);
            echo!("{}", options::OPTIONS_HELP);
            return Err(format!("Unexpected argument: {arg:?}"));
        }
    }

    if let Some(path) = a64_cache_resolver_test {
        #[cfg(feature = "a64")]
        return a64::cache_resolver_test(&path);
        #[cfg(not(feature = "a64"))]
        return Err("ARM64 support requires a build with --features a64".into());
    }

    if let Some(path) = a64_cache_map_test {
        #[cfg(feature = "a64")]
        return a64::cache_map_test(&path);
        #[cfg(not(feature = "a64"))]
        return Err("ARM64 support requires a build with --features a64".into());
    }

    if let Some(path) = a64_cache_info {
        #[cfg(feature = "a64")]
        return a64::cache_info(&path);
        #[cfg(not(feature = "a64"))]
        return Err("ARM64 support requires a build with --features a64".into());
    }

    if let Some(path) = a64_run {
        #[cfg(feature = "a64")]
        {
            let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let app_root = path
                .parent()
                .ok_or("ARM64 executable has no parent directory")?;
            let executable = format!(
                "/__a64_app/{}",
                path.file_name()
                    .ok_or("ARM64 executable has no filename")?
                    .to_string_lossy()
            );
            let code = a64::run_file_with_reader(&bytes, &executable, |dependency| {
                if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                    read_a64_runtime(a64_runtime.as_deref(), dependency)
                } else {
                    let relative = dependency.strip_prefix("/__a64_app/").ok_or_else(|| {
                        format!("ARM64 dependency outside app/runtime roots: {dependency}")
                    })?;
                    let candidate = std::fs::canonicalize(app_root.join(relative))
                        .map_err(|e| format!("{dependency}: {e}"))?;
                    if !candidate.starts_with(app_root) {
                        return Err("ARM64 dependency escapes app directory".into());
                    }
                    std::fs::read(candidate).map_err(|e| format!("{dependency}: {e}"))
                }
            })?;
            echo!("ARM64 guest exited with code {code}");
            return Ok(());
        }
        #[cfg(not(feature = "a64"))]
        return Err("ARM64 support requires a build with --features a64".into());
    }

    if options.dumping_options.symbols {
        let mut file = std::fs::File::create(&options.dumping_file).map_err(|e| e.to_string())?;
        dyld::Dyld::dump_host_symbols(&mut file).unwrap();
        return Ok(());
    }

    let bundle_path = if let Some(bundle_path) = bundle_path {
        bundle_path
    } else {
        #[cfg(target_os = "android")]
        {
            // Android's native launcher owns IPA selection and settings. An
            // empty/restored SDL activity exits instead of showing a second UI.
            echo!("No IPA selected; returning to the PlayCover-A launcher.");
            return Ok(());
        }
        #[cfg(not(target_os = "android"))]
        {
            let mut options = options::Options::default();
            // Apply command-line options only (no app-specific options apply)
            for option_arg in &option_args {
                let parse_result = options.parse_argument(option_arg);
                assert!(parse_result == Ok(true));
            }
            if options.headless {
                return Err(
                    "No app specified. Use the --help flag to see command-line usage.".to_string(),
                );
            }
            echo!(
            "No app specified, opening app picker. Use the --help flag to see command-line usage."
        );
            let (bundle_path, mut extra_options) = environment::app_picker::app_picker(options)?;
            option_args.append(&mut extra_options);
            bundle_path
        }
    };

    // When PowerShell does tab-completion on a directory, for some reason it
    // expands it to `'..\My Bundle.app\'` and that trailing \ seems to
    // get interpreted as escaping a double quotation mark?
    #[cfg(windows)]
    if let Some(fixed) = bundle_path.to_str().and_then(|s| s.strip_suffix('"')) {
        log!("Warning: The bundle path has a trailing quotation mark! This often happens accidentally on Windows when tab-completing, because '\\\"' gets interpreted by Rust in the wrong way. Did you meant to write {:?}?", fixed);
    }

    let bundle_data = fs::BundleData::open_any(&bundle_path)
        .map_err(|e| format!("Could not open app bundle: {e}"))?;
    let (bundle, fs) = match bundle::Bundle::new_bundle_and_fs_from_host_path(
        bundle_data,
        /* read_only_mode: */ false,
    ) {
        Ok(bundle) => bundle,
        Err(err) => {
            return Err(format!("Application bundle error: {err}. Check that the path is to an .app directory or an .ipa file."));
        }
    };

    let app_id = bundle.bundle_identifier();
    let minimum_os_version = bundle.minimum_os_version();
    let required_device_capabilities = bundle.required_device_capabilities();
    let device_family = bundle.device_family_array();

    echo!("App bundle info:");
    echo!("- Display name: {}", bundle.display_name());
    echo!("- Version: {}", bundle.bundle_version());
    echo!("- Identifier: {}", app_id);
    if let Some(canonical_name) = bundle.canonical_bundle_name() {
        echo!("- Internal name (canonical): {}.app", canonical_name);
    } else {
        echo!("- Internal name (from FS): {}.app", bundle.bundle_name());
    }
    echo!(
        "- Minimum OS version: {}",
        minimum_os_version.unwrap_or("(not specified)")
    );
    echo!(
        "- Required device capabilities: {}",
        if !required_device_capabilities.is_empty() {
            required_device_capabilities.join(", ")
        } else {
            "(not specified)".to_string()
        }
    );
    echo!(
        "- Device family: {}",
        if !device_family.is_empty() {
            device_family
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            "(not specified)".to_string()
        }
    );
    echo!();

    if let Some(version) = minimum_os_version {
        let (major, minor_etc) = version.split_once('.').unwrap();
        let minor = minor_etc
            .split_once('.')
            .map_or(minor_etc, |(minor, _etc)| minor);
        let major: u32 = major.parse().unwrap();
        let minor: u32 = minor.parse().unwrap();
        if major > 4 || (major == 4 && minor > 0) {
            echo!("Warning: app requires OS version {}. Only apps for iOS 4.0 and earlier are currently supported.", version);
        }
    }

    if required_device_capabilities.contains(&"opengles-2")
        || required_device_capabilities.contains(&"opengles-3")
    {
        echo!("Warning: app requires OpenGL ES 2.0+ support. Only OpenGL ES 1.1 is currently supported.");
    }

    if just_info {
        return Ok(());
    }

    // Keep universal apps on the mature ARM32 runtime when they have an ARM32
    // slice. ARM64-only bundles use the separate experimental loader.
    #[cfg(feature = "a64")]
    {
        let executable = fs
            .read(bundle.executable_path())
            .map_err(|_| "Could not read app executable".to_string())?;
        if a64::is_arm64_only(&executable)? {
            let executable_path = bundle.executable_path();
            if let Some(cache_path) = &a64_cache_session_image_info_test {
                let directory = executable_path.as_str().rsplit_once('/')
                    .ok_or("ARM64 executable has no bundle directory")?.0;
                let unity_path = format!("{directory}/Frameworks/UnityFramework.framework/UnityFramework");
                let with_unity = fs.exists(fs::GuestPath::new(&unity_path));
                return a64::cache_session_image_info_test(&executable, executable_path.as_str(), cache_path,
                    |dependency| {
                        if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                            read_a64_runtime(a64_runtime.as_deref(), dependency)
                        } else {
                            fs.read(fs::GuestPath::new(dependency)).map_err(|_| format!("Could not read ARM64 dependency {dependency}"))
                        }
                    }, with_unity);
            }
            if let Some(cache_path) = &a64_cache_session_test {
                let directory = executable_path.as_str().rsplit_once('/')
                    .ok_or("ARM64 executable has no bundle directory")?.0;
                let unity_path = format!("{directory}/Frameworks/UnityFramework.framework/UnityFramework");
                let with_unity = fs.exists(fs::GuestPath::new(&unity_path));
                return a64::cache_session_initializer_test(&executable, executable_path.as_str(), cache_path,
                    |dependency| {
                        if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                            read_a64_runtime(a64_runtime.as_deref(), dependency)
                        } else {
                            fs.read(fs::GuestPath::new(dependency)).map_err(|_| format!("Could not read ARM64 dependency {dependency}"))
                        }
                    }, with_unity);
            }
            if let Some(cache_path) = &a64_cache_initializer_test {
                return a64::cache_initializer_test(&executable, executable_path.as_str(), cache_path,
                    |dependency| {
                        if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                            read_a64_runtime(a64_runtime.as_deref(), dependency)
                        } else {
                            fs.read(fs::GuestPath::new(dependency)).map_err(|_| format!("Could not read ARM64 dependency {dependency}"))
                        }
                    });
            }
            if let Some(cache_path) = &a64_cache_entry_prefix_test {
                return a64::cache_entry_prefix_test(
                    &executable,
                    executable_path.as_str(),
                    cache_path,
                    |dependency| {
                        if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                            read_a64_runtime(a64_runtime.as_deref(), dependency)
                        } else {
                            fs.read(fs::GuestPath::new(dependency)).map_err(|_| {
                                format!("Could not read ARM64 dependency {dependency}")
                            })
                        }
                    },
                );
            }
            if let Some(cache_path) = &a64_cache_services_test {
                return a64::cache_services_test(
                    &executable,
                    executable_path.as_str(),
                    cache_path,
                    |dependency| {
                        if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                            read_a64_runtime(a64_runtime.as_deref(), dependency)
                        } else {
                            fs.read(fs::GuestPath::new(dependency)).map_err(|_| {
                                format!("Could not read ARM64 dependency {dependency}")
                            })
                        }
                    },
                );
            }
            if let Some(cache_path) = a64_legacy_cpp_prepare.as_ref()
                .or(a64_image_info_prepare.as_ref()).or(a64_cache_prepare.as_ref()) {
                let reader = |dependency: &str| {
                        if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                            read_a64_runtime(a64_runtime.as_deref(), dependency)
                        } else {
                            fs.read(fs::GuestPath::new(dependency)).map_err(|_| {
                                format!("Could not read ARM64 dependency {dependency}")
                            })
                        }
                    };
                return if a64_legacy_cpp_prepare.is_some() {
                    a64::cache_legacy_cpp_prepare(&executable, executable_path.as_str(), cache_path, reader)
                } else if a64_image_info_prepare.is_some() {
                    a64::cache_image_info_prepare(&executable, executable_path.as_str(), cache_path, reader)
                } else {
                    a64::cache_prepare(&executable, executable_path.as_str(), cache_path, reader)
                };
            }
            if let Some(cache_path) = &a64_cache_import_test {
                return a64::cache_import_test(
                    &executable,
                    executable_path.as_str(),
                    cache_path,
                    |dependency| Ok(fs.read(fs::GuestPath::new(dependency)).ok()),
                );
            }
            let result =
                a64::run_file_with_reader(&executable, executable_path.as_str(), |dependency| {
                    if dependency.starts_with("/System/") || dependency.starts_with("/usr/") {
                        read_a64_runtime(a64_runtime.as_deref(), dependency)
                    } else {
                        fs.read(fs::GuestPath::new(dependency))
                            .map_err(|_| format!("Could not read ARM64 dependency {dependency}"))
                    }
                });
            match result {
                Ok(code) => {
                    echo!("ARM64 guest exited with code {code}");
                    return Ok(());
                }
                Err(error) => {
                    let error = format!("Could not run ARM64 app: {error}");
                    if options.popup_errors {
                        window::show_error_messagebox(None, &error);
                    }
                    return Err(error);
                }
            }
        }
    }

    // Apply options from files
    fn apply_options<F: std::io::Read, P: std::fmt::Display>(
        file: F,
        path: P,
        options: &mut options::Options,
        app_id: &str,
    ) -> Result<(), String> {
        match options::get_options_from_file(file, app_id) {
            Ok(Some(options_string)) => {
                echo!(
                    "Using options from {} for this app: {}",
                    path,
                    options_string
                );
                for option_arg in options_string.split_ascii_whitespace() {
                    match options.parse_argument(option_arg) {
                        Ok(true) => (),
                        Ok(false) => return Err(format!("Unknown option {option_arg:?}")),
                        Err(err) => return Err(format!("Invalid option {option_arg:?}: {err}")),
                    }
                }
            }
            Ok(None) => {
                echo!("No options found for this app in {}", path);
            }
            Err(e) => {
                echo!("Warning: {}", e);
            }
        }
        Ok(())
    }
    let default_options_path = paths::DEFAULT_OPTIONS_FILE;
    match paths::ResourceFile::open(default_options_path) {
        Ok(mut file) => apply_options(file.get(), default_options_path, &mut options, app_id)?,
        Err(err) => echo!("Warning: Could not open {}: {}", default_options_path, err),
    }
    let user_options_path = paths::user_data_base_path().join(paths::USER_OPTIONS_FILE);
    match std::fs::File::open(&user_options_path) {
        Ok(file) => apply_options(file, user_options_path.display(), &mut options, app_id)?,
        Err(err) => echo!(
            "Warning: Could not open {}: {}",
            user_options_path.display(),
            err
        ),
    }
    echo!();

    // Apply command-line options
    for option_arg in option_args {
        let parse_result = options.parse_argument(&option_arg);
        assert!(parse_result == Ok(true));
    }

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Environment::new(bundle, fs, options.clone(), app_args.unwrap_or_default())
    }));
    let env = match res {
        Ok(ret) => match ret {
            Ok(env) => env,
            Err(e) => {
                if options.popup_errors {
                    window::show_error_messagebox(None, e.as_str());
                }
                return Err(e);
            }
        },
        Err(e) => {
            if options.popup_errors {
                let error_string = if let Some(s) = e.downcast_ref::<&str>() {
                    s
                } else if let Some(s) = e.downcast_ref::<String>() {
                    s
                } else {
                    "(non-string payload)"
                };
                window::show_error_messagebox(None, error_string);
            }
            std::panic::resume_unwind(e)
        }
    };
    env.run();
    Ok(())
}
