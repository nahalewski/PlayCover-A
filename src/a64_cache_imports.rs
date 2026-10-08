/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Direct executable import availability audit. Never binds or executes code.
use super::{cache_map, cache_symbols::CacheSymbols, exports, fixups, thin_arm64_slice, MachO64};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

#[derive(Debug, Default)]
pub(super) struct Audit {
    pub required_resolved: usize,
    pub required_missing: usize,
    pub weak_resolved: usize,
    pub weak_missing: usize,
    pub embedded_images: usize,
    pub dependencies: usize,
    pub imports: usize,
    pub missing: Vec<String>,
    pub missing_by_dependency: BTreeMap<String, (usize, usize)>,
    pub missing_by_reason: BTreeMap<&'static str, (usize, usize)>,
}

enum Source {
    Cache(String),
    Image {
        macho: MachO64,
        symbols: HashMap<String, exports::ExportTarget>,
        dependencies: Vec<usize>,
    },
    Missing,
}

fn normalize(path: &str) -> Result<String, String> {
    if !path.starts_with('/') {
        return Err(format!("import path must be absolute: {path}"));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop().ok_or("import path escapes guest root")?;
            }
            part => parts.push(part),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn directory(path: &str) -> &str {
    path.rsplit_once('/').map_or(
        "/",
        |(parent, _)| if parent.is_empty() { "/" } else { parent },
    )
}

fn expand(path: &str, owner: &str, executable: &str) -> Result<String, String> {
    if path == "@loader_path" {
        normalize(directory(owner))
    } else if path == "@executable_path" {
        normalize(directory(executable))
    } else if let Some(tail) = path.strip_prefix("@loader_path/") {
        normalize(&format!("{}/{tail}", directory(owner)))
    } else if let Some(tail) = path.strip_prefix("@executable_path/") {
        normalize(&format!("{}/{tail}", directory(executable)))
    } else {
        normalize(path)
    }
}

struct Graph<'a, R> {
    executable: &'a str,
    main_rpaths: Vec<String>,
    sources: Vec<Source>,
    indices: HashMap<String, usize>,
    reader: R,
}

impl<R> Graph<'_, R>
where
    R: FnMut(&str) -> Result<Option<Vec<u8>>, String>,
{
    fn image(&mut self, path: String, file: &[u8], is_main: bool) -> Result<usize, String> {
        if let Some(index) = self.indices.get(&path) {
            return Ok(*index);
        }
        if self.sources.len() >= 128 {
            return Err("import audit image graph exceeds 128 images".into());
        }
        let file = thin_arm64_slice(file)?;
        let macho = MachO64::parse_metadata(file)?;
        let index = self.sources.len();
        // These addresses are symbolic availability markers, never guest mappings.
        let base = if is_main {
            macho
                .segments
                .iter()
                .find(|s| s.name == "__TEXT")
                .map_or(0, |s| s.vmaddr)
        } else {
            0x1_2000_0000 + index as u64 * 0x1000_0000
        };
        let symbols = exports::parse_exports(file, base)?
            .into_iter()
            .map(|e| (e.name, e.target))
            .collect();
        self.indices.insert(path.clone(), index);
        self.sources.push(Source::Image {
            macho: macho.clone(),
            symbols,
            dependencies: Vec::new(),
        });
        let mut dependencies = Vec::new();
        for dependency in &macho.dependencies {
            dependencies.push(self.dependency(&dependency.name, &path, &macho.rpaths)?);
        }
        if let Source::Image {
            dependencies: links,
            ..
        } = &mut self.sources[index]
        {
            *links = dependencies;
        }
        Ok(index)
    }

    fn reference(&mut self, key: String, cache: bool) -> Result<usize, String> {
        if let Some(index) = self.indices.get(&key) {
            return Ok(*index);
        }
        if self.sources.len() >= 128 {
            return Err("import audit image graph exceeds 128 images".into());
        }
        let index = self.sources.len();
        self.indices.insert(key.clone(), index);
        self.sources.push(if cache {
            Source::Cache(key)
        } else {
            Source::Missing
        });
        Ok(index)
    }

    fn dependency(&mut self, name: &str, owner: &str, rpaths: &[String]) -> Result<usize, String> {
        // Absolute system install names are resolved against the original cache.
        if name.starts_with('/') {
            return self.reference(normalize(name)?, true);
        }
        let mut candidates = Vec::new();
        if let Some(tail) = name.strip_prefix("@rpath/") {
            for (path, path_owner) in rpaths
                .iter()
                .map(|path| (path, owner))
                .chain(self.main_rpaths.iter().map(|path| (path, self.executable)))
            {
                let prefix = expand(path, path_owner, self.executable)?;
                let candidate = normalize(&format!("{prefix}/{tail}"))?;
                if !candidates.contains(&candidate) {
                    candidates.push(candidate);
                }
            }
        } else {
            candidates.push(expand(name, owner, self.executable)?);
        }
        for candidate in candidates {
            if let Some(index) = self.indices.get(&candidate) {
                return Ok(*index);
            }
            if let Some(file) = (self.reader)(&candidate)? {
                return self.image(candidate, &file, false);
            }
        }
        self.reference(format!("missing:{name}"), false)
    }
}

fn lookup<C>(
    sources: &[Source],
    image: usize,
    name: &str,
    resolver: &mut C,
) -> Result<Option<u64>, String>
where
    C: FnMut(&str, &str) -> Result<Option<u64>, String>,
{
    let mut error = None;
    let result = exports::resolve_symbol(
        image,
        name,
        |index, name| match &sources[index] {
            Source::Cache(path) => match resolver(path, name) {
                Ok(Some(address)) => Some(exports::ExportTarget::Address(address)),
                Ok(None) => None,
                Err(reason) => {
                    error = Some(reason);
                    None
                }
            },
            Source::Image { symbols, .. } => symbols.get(name).cloned(),
            Source::Missing => None,
        },
        |index, ordinal| match &sources[index] {
            Source::Image { dependencies, .. } => {
                let index = usize::try_from(ordinal)
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .ok_or("invalid embedded reexport ordinal")?;
                dependencies
                    .get(index)
                    .copied()
                    .ok_or_else(|| "embedded reexport ordinal outside dependencies".into())
            }
            _ => Err("reexport ordinal requested from unavailable image".into()),
        },
        |index| match &sources[index] {
            Source::Image {
                macho,
                dependencies,
                ..
            } => macho
                .dependencies
                .iter()
                .zip(dependencies)
                .filter_map(|(dependency, index)| dependency.reexport.then_some(*index))
                .collect(),
            _ => Vec::new(),
        },
    );
    if let Some(error) = error {
        return Err(error);
    }
    result
}

fn audit<R, C>(
    bytes: &[u8],
    executable_path: &str,
    reader: R,
    mut resolver: C,
) -> Result<Audit, String>
where
    R: FnMut(&str) -> Result<Option<Vec<u8>>, String>,
    C: FnMut(&str, &str) -> Result<Option<u64>, String>,
{
    let bytes = thin_arm64_slice(bytes)?;
    let main = MachO64::parse_metadata(bytes)?;
    let (offset, end) = main
        .chained_fixups
        .ok_or("import availability audit requires chained fixups")?;
    let imports = fixups::imports(
        bytes
            .get(offset..end)
            .ok_or("import fixups outside image")?,
    )?;
    let executable = normalize(executable_path)?;
    let mut graph = Graph {
        executable: &executable,
        main_rpaths: main.rpaths.clone(),
        sources: Vec::new(),
        indices: HashMap::new(),
        reader,
    };
    let main_index = graph.image(executable.clone(), bytes, true)?;
    let Source::Image { dependencies, .. } = &graph.sources[main_index] else {
        unreachable!()
    };
    let mut audit = Audit {
        imports: imports.len(),
        dependencies: main.dependencies.len(),
        embedded_images: graph
            .sources
            .iter()
            .filter(|source| matches!(source, Source::Image { .. }))
            .count()
            .saturating_sub(1),
        ..Audit::default()
    };
    let mut required_examples = Vec::new();
    let mut weak_examples = Vec::new();
    for import in imports {
        let candidates: Result<Vec<usize>, String> = match import.library_ordinal {
            0 | -1 => Ok(vec![main_index]),
            -2 | -3 => Ok(std::iter::once(main_index)
                .chain(dependencies.iter().copied())
                .collect()),
            ordinal if ordinal > 0 => {
                let index = ordinal as usize - 1;
                dependencies
                    .get(index)
                    .copied()
                    .map(|index| vec![index])
                    .ok_or_else(|| "import ordinal outside executable dependencies".into())
            }
            _ => Err("unsupported special import ordinal".into()),
        };
        let resolution = candidates.and_then(|candidates| {
            for candidate in candidates {
                if let Some(value) = lookup(&graph.sources, candidate, &import.name, &mut resolver)?
                {
                    return Ok(Some(value));
                }
            }
            Ok(None)
        });
        let resolved = matches!(resolution, Ok(Some(_)));
        match (import.weak, resolved) {
            (false, true) => audit.required_resolved += 1,
            (false, false) => audit.required_missing += 1,
            (true, true) => audit.weak_resolved += 1,
            (true, false) => audit.weak_missing += 1,
        }
        if !resolved {
            let library = if import.library_ordinal > 0 {
                main.dependencies
                    .get(import.library_ordinal as usize - 1)
                    .map_or("invalid ordinal", |d| &d.name)
            } else {
                "special ordinal"
            };
            let reason = match resolution {
                Err(error) => error,
                _ => "symbol unavailable".into(),
            };
            let category = if reason == "symbol unavailable" {
                "symbol unavailable"
            } else if reason.contains("ordinal") {
                "invalid ordinal"
            } else if reason.contains("resolver") {
                "unsupported resolver"
            } else if reason.contains("thread-local") || reason.contains("TLS") {
                "unsupported TLS"
            } else {
                "metadata or resolution error"
            };
            for counts in [
                audit
                    .missing_by_dependency
                    .entry(library.to_owned())
                    .or_default(),
                audit.missing_by_reason.entry(category).or_default(),
            ] {
                if import.weak {
                    counts.1 += 1;
                } else {
                    counts.0 += 1;
                }
            }
            let examples = if import.weak {
                &mut weak_examples
            } else {
                &mut required_examples
            };
            if examples.len() < 32 {
                examples.push(format!(
                    "{} {:?} from {:?}: {}",
                    if import.weak { "weak" } else { "required" },
                    import.name,
                    library,
                    reason
                ));
            }
        }
    }
    audit.missing = required_examples
        .into_iter()
        .chain(weak_examples)
        .take(32)
        .collect();
    Ok(audit)
}

pub(super) fn inspect<R>(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    reader: R,
) -> Result<(), String>
where
    R: FnMut(&str) -> Result<Option<Vec<u8>>, String>,
{
    let (plan, cpu) = cache_map::map(cache_path)?;
    let mut symbols = CacheSymbols::new(&plan);
    let audit = audit(bytes, executable_path, reader, |path, name| {
        symbols.resolve(&cpu, path, name)
    })?;
    echo!("[a64] direct import availability: {} imports, {} dependencies, {} embedded images; required resolved {} / missing {}, weak resolved {} / missing {}; {} cached export images inspected. No imports bound or Apple code executed; runtime compatibility remains unverified.", audit.imports, audit.dependencies, audit.embedded_images, audit.required_resolved, audit.required_missing, audit.weak_resolved, audit.weak_missing, symbols.loaded_images());
    for (reason, (required, weak)) in &audit.missing_by_reason {
        echo!("[a64] missing reason {reason:?}: required {required}, weak {weak}");
    }
    let mut dependencies: Vec<_> = audit.missing_by_dependency.iter().collect();
    dependencies.sort_by(|(left_name, left), (right_name, right)| {
        right
            .0
            .cmp(&left.0)
            .then(right.1.cmp(&left.1))
            .then(left_name.cmp(right_name))
    });
    for (library, (required, weak)) in dependencies.iter().take(16) {
        echo!("[a64] missing from {library:?}: required {required}, weak {weak}");
    }
    if dependencies.len() > 16 {
        echo!(
            "[a64] {} additional missing dependency groups omitted",
            dependencies.len() - 16
        );
    }
    for missing in &audit.missing {
        echo!("[a64] {missing}");
    }
    if audit.required_missing != 0 {
        return Err(format!(
            "{} required direct imports unavailable",
            audit.required_missing
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn word(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn string_command(command: u32, minimum: usize, name: &str) -> Vec<u8> {
        let mut bytes = vec![0; (minimum + name.len() + 1 + 7) & !7];
        let length = bytes.len() as u32;
        word(&mut bytes, 0, command);
        word(&mut bytes, 4, length);
        word(&mut bytes, 8, minimum as u32);
        bytes[minimum..minimum + name.len()].copy_from_slice(name.as_bytes());
        bytes
    }
    fn header(file_type: u32, commands: &[Vec<u8>], payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 32];
        word(&mut bytes, 0, 0xfeedfacf);
        word(&mut bytes, 4, 0x0100000c);
        word(&mut bytes, 12, file_type);
        word(&mut bytes, 16, commands.len() as u32);
        word(
            &mut bytes,
            20,
            commands.iter().map(|c| c.len() as u32).sum(),
        );
        for command in commands {
            bytes.extend_from_slice(command);
        }
        bytes.extend_from_slice(payload);
        bytes
    }
    fn executable(dependency: &str, rpath: Option<&str>, imports: &[(u8, bool, &str)]) -> Vec<u8> {
        let mut entry = vec![0; 24];
        word(&mut entry, 0, 0x80000028);
        word(&mut entry, 4, 24);
        let mut commands = vec![entry, string_command(0xc, 24, dependency)];
        if let Some(rpath) = rpath {
            commands.push(string_command(0x8000001c, 12, rpath));
        }
        let mut fixups = vec![0; 32 + imports.len() * 4];
        word(&mut fixups, 4, 28);
        word(&mut fixups, 8, 32);
        word(&mut fixups, 12, (32 + imports.len() * 4) as u32);
        word(&mut fixups, 16, imports.len() as u32);
        word(&mut fixups, 20, 1);
        let mut name_offset = 0;
        for (index, (ordinal, weak, name)) in imports.iter().enumerate() {
            word(
                &mut fixups,
                32 + index * 4,
                u32::from(*ordinal) | (u32::from(*weak) << 8) | (name_offset << 9),
            );
            fixups.extend_from_slice(name.as_bytes());
            fixups.push(0);
            name_offset += name.len() as u32 + 1;
        }
        let mut command = vec![0; 16];
        word(&mut command, 0, 0x80000034);
        word(&mut command, 4, 16);
        word(
            &mut command,
            8,
            (32 + commands.iter().map(|c| c.len()).sum::<usize>() + 16) as u32,
        );
        word(&mut command, 12, fixups.len() as u32);
        commands.push(command);
        header(2, &commands, &fixups)
    }
    fn embedded() -> Vec<u8> {
        let trie = [0, 1, b'_', b'f', b'o', b'o', 0, 8, 2, 0, 0x20, 0];
        let mut command = vec![0; 16];
        word(&mut command, 0, 0x80000033);
        word(&mut command, 4, 16);
        word(&mut command, 8, 48);
        word(&mut command, 12, trie.len() as u32);
        header(6, &[command], &trie)
    }
    #[test]
    fn cache_availability_counts_required_and_weak_misses() {
        let main = executable(
            "/usr/lib/test.dylib",
            None,
            &[
                (1, false, "_ok"),
                (1, false, "_missing"),
                (1, true, "_weak"),
            ],
        );
        let report = audit(
            &main,
            "/Payload/Test.app/Test",
            |_| panic!("cache references must not use the IPA reader"),
            |path, name| {
                assert_eq!(path, "/usr/lib/test.dylib");
                Ok((name == "_ok").then_some(0x1234))
            },
        )
        .unwrap();
        assert_eq!(
            (
                report.required_resolved,
                report.required_missing,
                report.weak_missing
            ),
            (1, 1, 1)
        );
        assert_eq!(report.missing.len(), 2);
    }
    #[test]
    fn embedded_rpath_exports_are_available_without_execution() {
        let main = executable(
            "@rpath/E.framework/E",
            Some("@executable_path/Frameworks"),
            &[(1, false, "_foo")],
        );
        let report = audit(
            &main,
            "/Payload/Test.app/Test",
            |path| {
                assert_eq!(path, "/Payload/Test.app/Frameworks/E.framework/E");
                Ok(Some(embedded()))
            },
            |_, _| panic!("embedded export must not query cache"),
        )
        .unwrap();
        assert_eq!(
            (
                report.required_resolved,
                report.required_missing,
                report.embedded_images
            ),
            (1, 0, 1)
        );
        let missing = audit(
            &main,
            "/Payload/Test.app/Test",
            |_| Ok(None),
            |_, _| Ok(None),
        )
        .unwrap();
        assert_eq!((missing.required_missing, missing.embedded_images), (1, 0));
    }
    #[test]
    fn invalid_ordinals_and_cache_errors_remain_diagnostic_misses() {
        let main = executable(
            "/usr/lib/test.dylib",
            None,
            &[(2, false, "_bad"), (1, false, "_resolver")],
        );
        let report = audit(
            &main,
            "/Payload/Test.app/Test",
            |_| Ok(None),
            |_, _| Err("unsupported resolver export".into()),
        )
        .unwrap();
        assert_eq!(report.required_missing, 2);
        assert!(report.missing[0].contains("ordinal"));
        assert!(report.missing[1].contains("resolver"));
    }

    #[test]
    fn metadata_and_loader_retain_bounded_legacy_streams() {
        let mut command = vec![0; 48];
        word(&mut command, 0, 0x80000022);
        word(&mut command, 4, 48);
        word(&mut command, 8, 80);
        word(&mut command, 12, 2);
        let mut file = header(6, &[command], &[1, 0]);
        assert!(MachO64::parse_metadata(&file).is_ok());
        assert_eq!(
            MachO64::parse_image(&file).unwrap().legacy_fixups.unwrap()[0],
            (80, 2)
        );
        word(&mut file, 40, 81); // stream now exceeds the actual file
        assert!(MachO64::parse_metadata(&file)
            .unwrap_err()
            .contains("outside image"));
        word(&mut file, 40, 32); // stream overlaps load commands
        assert!(MachO64::parse_metadata(&file).is_err());
    }

    #[test]
    fn required_missing_examples_take_priority_over_earlier_weak_imports() {
        let mut imports = vec![(1, true, "_weak"); 40];
        imports.push((1, false, "_required"));
        let main = executable("/usr/lib/test.dylib", None, &imports);
        let report = audit(
            &main,
            "/Payload/Test.app/Test",
            |_| Ok(None),
            |_, _| Ok(None),
        )
        .unwrap();
        assert_eq!(report.missing.len(), 32);
        assert!(report.missing[0].starts_with("required"));
        assert!(report.missing[0].contains("_required"));
        assert_eq!(report.missing_by_dependency["/usr/lib/test.dylib"], (1, 40));
        assert_eq!(report.missing_by_reason["symbol unavailable"], (1, 40));
    }
}
