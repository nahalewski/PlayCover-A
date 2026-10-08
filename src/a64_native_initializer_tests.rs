/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Opt-in native execution of the exact diagnostic used on Samsung.
//! Uses the actual backup ZIP and original cache; never creates mock libraries.
use std::{fs::File, io::Read, path::PathBuf};

#[test]
#[ignore = "Explicit actual IPA/cache required; batch dependency and initializer diagnostics"]
fn actual_ipa_regression() {
    let result = ipa_regression();
    match result {
        Ok(stage) => println!("PLAYCOVER_REGRESSION_STAGE: {stage}"),
        Err(error) => println!("PLAYCOVER_REGRESSION_BOUNDARY: {}", error.replace('\n', " | ")),
    }
    // Passing this collector means an observation was recorded, not app success.
    println!("PLAYCOVER_REGRESSION_COLLECTED: diagnostic only; no app-main or gameplay receipt");
}

fn ipa_regression() -> Result<&'static str, String> {
    let ipa = std::env::var_os("PLAYCOVER_NATIVE_IPA").ok_or("explicit IPA required")?;
    let cache = std::env::var_os("PLAYCOVER_NATIVE_CACHE").ok_or("explicit cache required")?;
    let mut archive = zip::ZipArchive::new(File::open(ipa).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let roots = archive.file_names().filter(|name| {
        let parts = name.split('/').collect::<Vec<_>>();
        parts.len() == 3 && parts[0] == "Payload" && parts[1].ends_with(".app") && parts[2] == "Info.plist"
    }).map(str::to_owned).collect::<Vec<_>>();
    if roots.len() != 1 { return Err(format!("Expected one main app Info.plist, found {}", roots.len())); }
    let root = roots[0].strip_suffix("Info.plist").unwrap().to_owned();
    let unity = archive.by_name(&format!("{root}Frameworks/UnityFramework.framework/UnityFramework")).is_ok();
    let mut read = |name: &str| -> Result<Vec<u8>, String> {
        let mut member = archive.by_name(name).map_err(|e| format!("IPA member {name}: {e}"))?;
        if member.size() > 512 * 1024 * 1024 {
            return Err(format!("Harness input limit (512 MiB): IPA member {name}"));
        }
        if member.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(format!("Symlink IPA member refused: {name}"));
        }
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    };
    let plist = plist::Value::from_reader(std::io::Cursor::new(read(&roots[0])?)).map_err(|e| e.to_string())?;
    let executable = plist.as_dictionary().and_then(|d| d.get("CFBundleExecutable"))
        .and_then(|v| v.as_string()).ok_or("Missing CFBundleExecutable")?;
    if executable.is_empty() || executable.contains('/') || executable.contains('\\') || executable.contains('\0') {
        return Err("Unsafe main executable name".into());
    }
    let bundle = root.strip_prefix("Payload/").unwrap();
    let path = format!("/{bundle}{executable}");
    let prefix = format!("/{bundle}");
    let main = read(&format!("{root}{executable}"))?;
    let mut reader = |path: &str| -> Result<Vec<u8>, String> {
        let relative = path.strip_prefix(&prefix).ok_or_else(|| format!("External dependency {path}"))?;
        if relative.split('/').any(|p| p.is_empty() || p == "." || p == "..") || relative.contains('\0') || relative.contains('\\') {
            return Err("Unsafe bundle dependency path".into());
        }
        read(&format!("{root}{relative}"))
    };
    if std::env::var("PLAYCOVER_REGRESSION_PROBE").as_deref() == Ok("legacy-cpp-prepare") {
        println!("PLAYCOVER_REGRESSION_MODE: legacy_cpp_cache_preparation");
        super::cache_legacy_cpp_prepare(&main, &path, &PathBuf::from(cache), &mut reader)?;
        return Ok("legacy_cpp_dependencies_prepared; initializer/app startup untested");
    }
    if std::env::var("PLAYCOVER_REGRESSION_PROBE").as_deref() == Ok("session-initializer") {
        println!("PLAYCOVER_REGRESSION_MODE: retained_session_libsystem_initializer");
        super::cache_session_initializer_test(&main, &path, &PathBuf::from(cache), &mut reader, unity)?;
        return Ok("retained_session_libsystem_initializer_returned; full runtime/app startup untested".into());
    }
    if std::env::var("PLAYCOVER_REGRESSION_PROBE").as_deref() == Ok("session-image-info") {
        println!("PLAYCOVER_REGRESSION_MODE: retained_session_image_info_initializer");
        super::cache_session_image_info_test(&main, &path, &PathBuf::from(cache), &mut reader, unity)?;
        return Ok("image_info_session_libsystem_initializer_returned; full runtime/app startup untested");
    }
    println!("PLAYCOVER_REGRESSION_MODE: {}", if unity { "unity_libsystem_initializer" } else { "bundle_libsystem_initializer" });
    super::cache_bundle_initializer_test(&main, &path, &PathBuf::from(cache), &mut reader, unity)?;
    Ok("original_libsystem_initializer_returned; remaining framework/app startup untested")
}

#[test]
#[ignore = "Requires PLAYCOVER_NATIVE_IPA and PLAYCOVER_NATIVE_CACHE; actual guest code executes"]
fn actual_terraria_original_libsystem_initializer() {
    let ipa = PathBuf::from(std::env::var_os("PLAYCOVER_NATIVE_IPA").expect("explicit actual IPA path required"));
    let cache = PathBuf::from(std::env::var_os("PLAYCOVER_NATIVE_CACHE").expect("explicit original cache path required"));
    let mut archive = zip::ZipArchive::new(File::open(&ipa).expect("open actual IPA")).expect("read actual IPA ZIP");
    let root = "Payload/Terraria.app/";
    let mut read_member = |name: &str| -> Result<Vec<u8>, String> {
        let mut member = archive.by_name(name).map_err(|error| format!("Actual IPA lacks {name}: {error}"))?;
        if member.size() == 0 || member.size() > 256 * 1024 * 1024
            || member.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("Invalid/oversized/symlink actual IPA member {name}"));
        }
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).map_err(|error| format!("Actual IPA member read {name}: {error}"))?;
        Ok(bytes)
    };
    let metadata = read_member(&format!("{root}Info.plist")).expect("actual main Info.plist");
    let metadata = plist::Value::from_reader(std::io::Cursor::new(metadata)).expect("parse actual Info.plist");
    let dictionary = metadata.as_dictionary().expect("actual Info.plist dictionary");
    assert_eq!(dictionary.get("CFBundleExecutable").and_then(|v| v.as_string()), Some("Terraria"));
    assert_eq!(dictionary.get("CFBundleIdentifier").and_then(|v| v.as_string()), Some("com.505games.terraria"));
    let main = read_member(&format!("{root}Terraria")).expect("actual main Mach-O");
    let result = super::cache_initializer_test(&main, "/Terraria.app/Terraria", &cache, |path| {
        let relative = path.strip_prefix("/Terraria.app/")
            .ok_or_else(|| format!("Native reader refuses external dependency {path}"))?;
        if relative.is_empty() || relative.split('/').any(|part| part.is_empty() || part == "." || part == "..") || relative.contains('\0') {
            return Err("Invalid logical guest bundle dependency path".into());
        }
        read_member(&format!("{root}{relative}"))
    });
    match result {
        Ok(()) => {
            println!("PLAYCOVER_NATIVE_OUTCOME: initializer_returned; diagnostic only, no framework readiness or gameplay receipt");
            assert!(std::env::var_os("PLAYCOVER_NATIVE_EXPECT_BOUNDARY").is_none(), "Expected a boundary, but initializer returned");
        }
        Err(error) => {
            println!("PLAYCOVER_NATIVE_BOUNDARY: {error}");
            if let Ok(expected) = std::env::var("PLAYCOVER_NATIVE_EXPECT_BOUNDARY") {
                assert!(!expected.is_empty() && error.contains(&expected), "Different actual native boundary than requested: {error}");
                println!("PLAYCOVER_NATIVE_OUTCOME: expected_boundary; initializer did not complete, no readiness receipt");
            } else {
                panic!("Actual native initializer boundary: {error}");
            }
        }
    }
}
