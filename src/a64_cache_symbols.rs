/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Lazy symbol inspection at original shared-cache addresses; never executes code.
use super::{cache::CachePlan, exports};
use std::collections::{HashMap, HashSet, VecDeque};
use touchHLE_dynarmic_wrapper::a64::A64Cpu;

const MAX_COMMANDS: usize = 4 * 1024 * 1024;
const MAX_TRIE: usize = 16 * 1024 * 1024;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const MAX_RETAINED: usize = 64 * 1024 * 1024;

fn u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset
        .checked_add(4)
        .ok_or("cache symbol offset overflow")?;
    Ok(u32::from_le_bytes(
        data.get(offset..end)
            .ok_or("truncated cache image u32")?
            .try_into()
            .unwrap(),
    ))
}
fn u64_at(data: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or("cache symbol offset overflow")?;
    Ok(u64::from_le_bytes(
        data.get(offset..end)
            .ok_or("truncated cache image u64")?
            .try_into()
            .unwrap(),
    ))
}
fn read(cpu: &A64Cpu, address: u64, size: usize) -> Result<Vec<u8>, String> {
    address
        .checked_add(size as u64)
        .ok_or("cache image read overflow")?;
    let mut data = vec![0; size];
    cpu.read_into(address, &mut data)?;
    Ok(data)
}
fn command_name(data: &[u8], start: usize, end: usize) -> Result<String, String> {
    let relative = u32_at(data, start + 8)? as usize;
    if relative < 24 {
        return Err("invalid cached dylib name offset".into());
    }
    let offset = start
        .checked_add(relative)
        .filter(|offset| *offset < end)
        .ok_or("cached dylib name outside command")?;
    let raw = &data[offset..end];
    let length = raw
        .iter()
        .position(|&byte| byte == 0)
        .ok_or("unterminated cached dylib name")?;
    if length == 0 || length > 4096 {
        return Err("invalid cached dylib name length".into());
    }
    String::from_utf8(raw[..length].to_vec()).map_err(|_| "cached dylib name is not UTF-8".into())
}

struct Dependency {
    name: String,
    reexport: bool,
    weak: bool,
}
struct Metadata {
    dependencies: Vec<Dependency>,
    symbols: HashMap<String, (exports::ExportTarget, bool)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CacheDefinition {
    pub address: u64,
    pub weak: bool,
}

pub(super) struct CacheSymbols {
    images: Vec<(u64, String)>,
    paths: HashMap<String, Vec<usize>>,
    metadata: HashMap<usize, Metadata>,
    metadata_bytes: usize,
    retained_bytes: usize,
}

impl CacheSymbols {
    pub(super) fn new(plan: &CachePlan) -> Self {
        Self::from_images(
            plan.images
                .iter()
                .map(|image| (image.address, image.path.clone()))
                .collect(),
        )
    }

    fn from_images(images: Vec<(u64, String)>) -> Self {
        let mut paths: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, (_, path)) in images.iter().enumerate() {
            paths.entry(path.clone()).or_default().push(index);
        }
        // Include copied path strings and a conservative allowance for hash
        // buckets, index vectors and allocation overhead in the persistent
        // index. Expanded export names are charged separately as they load.
        let retained_bytes = images
            .capacity()
            .saturating_mul(std::mem::size_of::<(u64, String)>())
            .saturating_add(
                images
                    .iter()
                    .map(|(_, path)| path.capacity())
                    .sum::<usize>(),
            )
            .saturating_add(
                paths
                    .iter()
                    .map(|(path, indices)| {
                        path.capacity()
                            .saturating_add(
                                indices
                                    .capacity()
                                    .saturating_mul(std::mem::size_of::<usize>()),
                            )
                            .saturating_add(2 * std::mem::size_of::<(String, Vec<usize>)>() + 32)
                    })
                    .sum::<usize>(),
            );
        Self {
            images,
            paths,
            metadata: HashMap::new(),
            metadata_bytes: 0,
            retained_bytes,
        }
    }

    pub(super) fn loaded_images(&self) -> usize {
        self.metadata.len()
    }

    fn find(&self, path: &str) -> Result<Option<usize>, String> {
        match self.paths.get(path) {
            None => Ok(None),
            Some(indices) if indices.len() == 1 => Ok(Some(indices[0])),
            Some(_) => Err(format!("ambiguous cached image path {path}")),
        }
    }

    fn ensure_metadata(&mut self, cpu: &A64Cpu, index: usize) -> Result<(), String> {
        if self.metadata.contains_key(&index) {
            return Ok(());
        }
        if self.retained_bytes > MAX_RETAINED {
            return Err("retained cached symbol index exceeds 64 MiB".into());
        }
        let (base, path) = &self.images[index];
        let base = *base;
        let header = read(cpu, base, 32).map_err(|e| format!("{path}: {e}"))?;
        if u32_at(&header, 0)? != 0xfeedfacf
            || u32_at(&header, 4)? != 0x0100000c
            || u32_at(&header, 12)? != 6
        {
            return Err(format!("{path}: cached image is not an ARM64 MH_DYLIB"));
        }
        let count = u32_at(&header, 16)? as usize;
        let size = u32_at(&header, 20)? as usize;
        if size > MAX_COMMANDS || count > size / 8 {
            return Err("cached image load-command limit exceeded".into());
        }
        let commands = read(
            cpu,
            base.checked_add(32).ok_or("cached header overflow")?,
            size,
        )?;
        let mut offset = 0usize;
        let mut dependencies = Vec::new();
        let mut identity = None;
        let mut linkedit = None;
        let mut export_range = None;
        for _ in 0..count {
            let command = u32_at(&commands, offset)?;
            let length = u32_at(&commands, offset + 4)? as usize;
            let end = offset
                .checked_add(length)
                .filter(|end| *end <= size)
                .ok_or("cached command outside declared area")?;
            if length < 8 {
                return Err("invalid cached load-command size".into());
            }
            match command {
                0x19 => {
                    if length < 72 {
                        return Err("truncated cached LC_SEGMENT_64".into());
                    }
                    if commands[offset + 8..offset + 24].starts_with(b"__LINKEDIT\0") {
                        if linkedit.is_some() {
                            return Err("duplicate cached __LINKEDIT".into());
                        }
                        linkedit = Some((
                            u64_at(&commands, offset + 24)?,
                            u64_at(&commands, offset + 40)?,
                            u64_at(&commands, offset + 48)?,
                        ));
                    }
                }
                0xd | 0xc | 0x18 | 0x80000018 | 0x8000001f | 0x80000023 => {
                    if length < 24 {
                        return Err("truncated cached dylib load command".into());
                    }
                    let name = command_name(&commands, offset, end)?;
                    if command == 0xd {
                        if identity.replace(name).is_some() {
                            return Err("duplicate cached LC_ID_DYLIB".into());
                        }
                    } else {
                        if dependencies.len() >= 128 {
                            return Err("cached image dependency limit exceeded".into());
                        }
                        dependencies.push(Dependency {
                            name,
                            reexport: command == 0x8000001f,
                            weak: matches!(command, 0x18 | 0x80000018),
                        });
                    }
                }
                0x80000033 => {
                    if length < 16 {
                        return Err("truncated cached export trie command".into());
                    }
                    let range = (
                        u32_at(&commands, offset + 8)? as u64,
                        u32_at(&commands, offset + 12)? as usize,
                    );
                    if export_range.replace(range).is_some_and(|old| old != range) {
                        return Err("conflicting cached export metadata".into());
                    }
                }
                0x22 | 0x80000022 => {
                    if length < 48 {
                        return Err("truncated cached dyld info command".into());
                    }
                    let range = (
                        u32_at(&commands, offset + 40)? as u64,
                        u32_at(&commands, offset + 44)? as usize,
                    );
                    if range.1 != 0 && export_range.replace(range).is_some_and(|old| old != range) {
                        return Err("conflicting cached export metadata".into());
                    }
                }
                _ => (),
            }
            offset = end;
        }
        // Monolithic caches retain filesystem aliases as separate image-table
        // entries at the identical header address. Accept only that explicit
        // cache evidence, never a basename or inferred filesystem alias.
        let verified_alias = identity
            .as_ref()
            .and_then(|name| self.paths.get(name))
            .is_some_and(|indices| indices.len() == 1 && self.images[indices[0]].0 == base);
        if identity.as_deref() != Some(path.as_str()) && !verified_alias {
            return Err(format!(
                "{path}: cached LC_ID_DYLIB identity mismatch: {identity:?}"
            ));
        }
        let mut symbols = HashMap::new();
        let mut cost = size;
        let mut retained_cost = std::mem::size_of::<Metadata>()
            + 64
            + dependencies.capacity() * std::mem::size_of::<Dependency>()
            + dependencies
                .iter()
                .map(|dependency| dependency.name.capacity())
                .sum::<usize>();
        if let Some((file_offset, trie_size)) = export_range.filter(|range| range.1 != 0) {
            if trie_size > MAX_TRIE {
                return Err("cached export trie exceeds 16 MiB".into());
            }
            let (vmaddr, link_file_offset, link_file_size) =
                linkedit.ok_or("cached exports missing __LINKEDIT")?;
            let relative = file_offset
                .checked_sub(link_file_offset)
                .ok_or("cached exports precede __LINKEDIT")?;
            if relative
                .checked_add(trie_size as u64)
                .filter(|end| *end <= link_file_size)
                .is_none()
            {
                return Err("cached exports outside __LINKEDIT".into());
            }
            cost = cost
                .checked_add(trie_size)
                .ok_or("cached metadata size overflow")?;
            if self
                .metadata_bytes
                .checked_add(cost)
                .filter(|total| *total <= MAX_METADATA)
                .is_none()
            {
                return Err("cached symbol metadata exceeds 64 MiB".into());
            }
            let address = vmaddr
                .checked_add(relative)
                .ok_or("cached export address overflow")?;
            let trie = read(cpu, address, trie_size)?;
            for export in exports::parse_trie(&trie, base).map_err(|e| format!("{path}: {e}"))? {
                let alias_bytes = match &export.target {
                    exports::ExportTarget::Reexport { name, .. } => name.capacity(),
                    _ => 0,
                };
                // HashMap capacity grows geometrically. Charging twice each
                // bucket plus allocator/control bytes bounds persistent
                // storage rather than merely the compressed trie input.
                let entry_cost = export
                    .name
                    .capacity()
                    .checked_add(alias_bytes)
                    .and_then(|cost| {
                        cost.checked_add(
                            2 * (std::mem::size_of::<(String, (exports::ExportTarget, bool))>()
                                + std::mem::size_of::<usize>())
                                + 32,
                        )
                    })
                    .ok_or("retained cache export size overflow")?;
                retained_cost = retained_cost
                    .checked_add(entry_cost)
                    .ok_or("retained cache metadata size overflow")?;
                if self
                    .retained_bytes
                    .checked_add(retained_cost)
                    .filter(|total| *total <= MAX_RETAINED)
                    .is_none()
                {
                    return Err("retained cached symbol index exceeds 64 MiB".into());
                }
                if symbols
                    .insert(export.name, (export.target, export.flags & 4 != 0))
                    .is_some()
                {
                    return Err("duplicate cached export name".into());
                }
            }
        }
        let metadata_bytes = self
            .metadata_bytes
            .checked_add(cost)
            .filter(|total| *total <= MAX_METADATA)
            .ok_or("cached symbol metadata exceeds 64 MiB")?;
        let retained_bytes = self
            .retained_bytes
            .checked_add(retained_cost)
            .filter(|total| *total <= MAX_RETAINED)
            .ok_or("retained cached symbol index exceeds 64 MiB")?;
        self.metadata_bytes = metadata_bytes;
        self.retained_bytes = retained_bytes;
        self.metadata.insert(
            index,
            Metadata {
                dependencies,
                symbols,
            },
        );
        Ok(())
    }

    fn dependency_image(&self, owner: usize, ordinal: u64) -> Result<usize, String> {
        let ordinal: usize = ordinal
            .checked_sub(1)
            .ok_or("cached reexport ordinal zero")?
            .try_into()
            .map_err(|_| "cached reexport ordinal overflow")?;
        let dependency = self.metadata[&owner]
            .dependencies
            .get(ordinal)
            .ok_or("cached reexport ordinal outside dependency list")?;
        self.find(&dependency.name)?.ok_or_else(|| {
            format!(
                "cached reexport dependency is unavailable: {}",
                dependency.name
            )
        })
    }

    pub(super) fn resolve(
        &mut self,
        cpu: &A64Cpu,
        path: &str,
        name: &str,
    ) -> Result<Option<u64>, String> {
        self.resolve_definition(cpu, path, name)
            .map(|definition| definition.map(|d| d.address))
    }

    /// Follow reexports to the defining terminal. Strength comes from that
    /// actual export, rather than the facade's reexport flags.
    pub(super) fn resolve_definition(
        &mut self,
        cpu: &A64Cpu,
        path: &str,
        name: &str,
    ) -> Result<Option<CacheDefinition>, String> {
        let Some(index) = self.prepare_symbol_metadata(cpu, path, name)? else {
            return Ok(None);
        };
        self.resolve_prepared(index, name, |_, name, _, _| {
            Err(format!("symbol {name} requires an unsupported resolver"))
        })
    }

    /// Opt-in path for initialized, bounded resolver services. Read-only
    /// availability audits above retain their original no-execution behavior.
    pub(super) fn resolve_definition_with_resolver<R>(
        &mut self,
        cpu: &mut A64Cpu,
        path: &str,
        name: &str,
        mut resolver: R,
    ) -> Result<Option<CacheDefinition>, String>
    where
        R: FnMut(&mut A64Cpu, usize, &str, u64, u64) -> Result<u64, String>,
    {
        let Some(index) = self.prepare_symbol_metadata(cpu, path, name)? else {
            return Ok(None);
        };
        self.resolve_prepared(index, name, |owner, name, stub, entry| {
            resolver(cpu, owner, name, stub, entry)
        })
    }

    fn prepare_symbol_metadata(
        &mut self,
        cpu: &A64Cpu,
        path: &str,
        name: &str,
    ) -> Result<Option<usize>, String> {
        if name.is_empty() || name.len() > 4096 || name.contains('\0') {
            return Err("invalid cached symbol name".into());
        }
        let Some(index) = self.find(path)? else {
            return Ok(None);
        };
        // Load only metadata reachable through this queried symbol. Closures
        // used by the shared resolver then borrow an immutable prepared index.
        let mut queue = VecDeque::from([(index, name.to_owned())]);
        let mut visited = HashSet::new();
        while let Some((owner, symbol)) = queue.pop_front() {
            if !visited.insert((owner, symbol.clone())) {
                continue;
            }
            if visited.len() > 4096 {
                return Err("cached symbol traversal limit exceeded".into());
            }
            self.ensure_metadata(cpu, owner)?;
            let metadata = &self.metadata[&owner];
            match metadata.symbols.get(&symbol).map(|(target, _)| target) {
                Some(exports::ExportTarget::Reexport {
                    ordinal,
                    name: alias,
                }) => {
                    queue.push_back((
                        self.dependency_image(owner, *ordinal)?,
                        if alias.is_empty() {
                            symbol
                        } else {
                            alias.clone()
                        },
                    ));
                }
                None => {
                    for dependency in metadata
                        .dependencies
                        .iter()
                        .filter(|dependency| dependency.reexport)
                    {
                        match self.find(&dependency.name)? {
                            Some(target) => queue.push_back((target, symbol.clone())),
                            None if dependency.weak => (),
                            None => {
                                return Err(format!(
                                    "cached reexport dependency is unavailable: {}",
                                    dependency.name
                                ))
                            }
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(Some(index))
    }

    fn resolve_prepared<R>(
        &self,
        index: usize,
        name: &str,
        mut resolver: R,
    ) -> Result<Option<CacheDefinition>, String>
    where
        R: FnMut(usize, &str, u64, u64) -> Result<u64, String>,
    {
        // Tokenize only concrete addresses so the bounded shared traversal
        // returns the exact defining export, even when two images share an
        // address but have different weak flags. TLS remains unsupported.
        let definitions = std::cell::RefCell::new(Vec::new());
        let token = exports::resolve_symbol_with_resolver(
            index,
            name,
            |owner, symbol| {
                self.metadata[&owner]
                    .symbols
                    .get(symbol)
                    .map(|(target, weak)| match target {
                        exports::ExportTarget::Address(address) => {
                            let mut definitions = definitions.borrow_mut();
                            definitions.push(CacheDefinition {
                                address: *address,
                                weak: *weak,
                            });
                            exports::ExportTarget::Address(definitions.len() as u64)
                        }
                        _ => target.clone(),
                    })
            },
            |owner, ordinal| self.dependency_image(owner, ordinal),
            |owner| {
                self.metadata[&owner]
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.reexport)
                    .filter_map(|dependency| {
                        self.paths.get(&dependency.name).and_then(|indices| {
                            if indices.len() == 1 {
                                Some(indices[0])
                            } else {
                                None
                            }
                        })
                    })
                    .collect()
            },
            |owner, symbol, stub, entry| {
                let address = resolver(owner, symbol, stub, entry)?;
                if address == 0 {
                    return Err(format!("cached resolver for {symbol} returned null"));
                }
                let weak = self.metadata[&owner]
                    .symbols
                    .get(symbol)
                    .ok_or("missing defining resolver metadata")?
                    .1;
                let mut definitions = definitions.borrow_mut();
                definitions.push(CacheDefinition { address, weak });
                Ok(definitions.len() as u64)
            },
        )?;
        token
            .map(|token| {
                definitions
                    .borrow()
                    .get(token as usize - 1)
                    .copied()
                    .ok_or("invalid cached definition token".to_owned())
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(name: &str, vmaddr: u64, fileoff: u64, filesize: u64) -> Vec<u8> {
        let mut command = vec![0; 72];
        command[..4].copy_from_slice(&0x19u32.to_le_bytes());
        command[4..8].copy_from_slice(&72u32.to_le_bytes());
        command[8..8 + name.len()].copy_from_slice(name.as_bytes());
        command[24..32].copy_from_slice(&vmaddr.to_le_bytes());
        command[32..40].copy_from_slice(&filesize.to_le_bytes());
        command[40..48].copy_from_slice(&fileoff.to_le_bytes());
        command[48..56].copy_from_slice(&filesize.to_le_bytes());
        command
    }
    fn dylib(command: u32, name: &str) -> Vec<u8> {
        let mut bytes = vec![0; 24 + name.len() + 1];
        let length = bytes.len() as u32;
        bytes[..4].copy_from_slice(&command.to_le_bytes());
        bytes[4..8].copy_from_slice(&length.to_le_bytes());
        bytes[8..12].copy_from_slice(&24u32.to_le_bytes());
        bytes[24..24 + name.len()].copy_from_slice(name.as_bytes());
        bytes
    }
    fn image(cpu: &mut A64Cpu, base: u64, path: &str, reexport: Option<&str>, with_export: bool) {
        let linkedit = base + 0x100000;
        let trie = [0, 1, b'_', b'f', b'o', b'o', 0, 8, 2, 0, 0x20, 0];
        let mut commands = vec![
            segment("__TEXT", base, 0, 0x1000),
            segment("__LINKEDIT", linkedit, 0x8000, 0x1000),
            dylib(0xd, path),
        ];
        if let Some(dependency) = reexport {
            commands.push(dylib(0x8000001f, dependency));
        }
        if with_export {
            let mut export = vec![0; 16];
            export[..4].copy_from_slice(&0x80000033u32.to_le_bytes());
            export[4..8].copy_from_slice(&16u32.to_le_bytes());
            export[8..12].copy_from_slice(&0x8008u32.to_le_bytes());
            export[12..16].copy_from_slice(&(trie.len() as u32).to_le_bytes());
            commands.push(export);
        }
        let mut header = vec![0; 32];
        header[..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        header[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        header[12..16].copy_from_slice(&6u32.to_le_bytes());
        header[16..20].copy_from_slice(&(commands.len() as u32).to_le_bytes());
        header[20..24]
            .copy_from_slice(&(commands.iter().map(Vec::len).sum::<usize>() as u32).to_le_bytes());
        for command in commands {
            header.extend(command);
        }
        cpu.map_zeroed(base, 0x1000, 1).unwrap();
        cpu.map_zeroed(linkedit, 0x1000, 1).unwrap();
        cpu.write_bytes(base, &header);
        cpu.write_bytes(linkedit + 8, &trie);
    }

    #[test]
    fn mutable_resolver_preserves_defining_strength_and_readonly_audit_refusal() {
        let mut cpu = A64Cpu::new_sparse();
        image(
            &mut cpu,
            0x180000000,
            "/usr/lib/front.dylib",
            Some("/usr/lib/back.dylib"),
            false,
        );
        image(&mut cpu, 0x190000000, "/usr/lib/back.dylib", None, true);
        let mut symbols = CacheSymbols::from_images(vec![
            (0x180000000, "/usr/lib/front.dylib".into()),
            (0x190000000, "/usr/lib/back.dylib".into()),
        ]);
        symbols.ensure_metadata(&cpu, 1).unwrap();
        symbols.metadata.get_mut(&1).unwrap().symbols.insert(
            "_foo".into(),
            (
                exports::ExportTarget::StubAndResolver {
                    stub: 0x190000020,
                    resolver: 0x190000040,
                },
                true,
            ),
        );
        assert!(symbols
            .resolve_definition(&cpu, "/usr/lib/front.dylib", "_foo")
            .unwrap_err()
            .contains("unsupported resolver"));
        let resolved = symbols.resolve_definition_with_resolver(
            &mut cpu,
            "/usr/lib/front.dylib",
            "_foo",
            |_, owner, name, stub, resolver| {
                assert_eq!(
                    (owner, name, stub, resolver),
                    (1, "_foo", 0x190000020, 0x190000040)
                );
                Ok(0x190000080)
            },
        );
        assert_eq!(
            resolved,
            Ok(Some(CacheDefinition {
                address: 0x190000080,
                weak: true
            }))
        );
        assert!(symbols
            .resolve_definition_with_resolver(
                &mut cpu,
                "/usr/lib/front.dylib",
                "_foo",
                |_, _, _, _, _| Ok(0)
            )
            .unwrap_err()
            .contains("null"));
    }
    #[test]
    fn cached_exports_translate_linkedit_file_offsets_to_vm() {
        let mut cpu = A64Cpu::new_sparse();
        image(&mut cpu, 0x180000000, "/usr/lib/test.dylib", None, true);
        let mut symbols =
            CacheSymbols::from_images(vec![(0x180000000, "/usr/lib/test.dylib".into())]);
        assert_eq!(
            symbols.resolve(&cpu, "/usr/lib/test.dylib", "_foo"),
            Ok(Some(0x180000020))
        );
        assert_eq!(
            symbols.resolve(&cpu, "/usr/lib/test.dylib", "_missing"),
            Ok(None)
        );
        assert_eq!(symbols.resolve(&cpu, "/missing", "_foo"), Ok(None));
        assert_eq!(symbols.loaded_images(), 1);
    }

    #[test]
    fn cached_image_table_alias_requires_identical_header_address() {
        let mut cpu = A64Cpu::new_sparse();
        image(
            &mut cpu,
            0x180000000,
            "/usr/lib/libstdc++.6.0.9.dylib",
            None,
            true,
        );
        let mut symbols = CacheSymbols::from_images(vec![
            (0x180000000, "/usr/lib/libstdc++.6.0.9.dylib".into()),
            (0x180000000, "/usr/lib/libstdc++.6.dylib".into()),
        ]);
        assert_eq!(
            symbols.resolve(&cpu, "/usr/lib/libstdc++.6.dylib", "_foo"),
            Ok(Some(0x180000020))
        );
        let mut mismatched = CacheSymbols::from_images(vec![
            (0x190000000, "/usr/lib/libstdc++.6.0.9.dylib".into()),
            (0x180000000, "/usr/lib/libstdc++.6.dylib".into()),
        ]);
        assert!(mismatched
            .resolve(&cpu, "/usr/lib/libstdc++.6.dylib", "_foo")
            .unwrap_err()
            .contains("identity mismatch"));
    }

    #[test]
    fn resolved_definition_tracks_terminal_weak_strength_through_reexports() {
        let mut cpu = A64Cpu::new_sparse();
        image(
            &mut cpu,
            0x180000000,
            "/usr/lib/front.dylib",
            Some("/usr/lib/back.dylib"),
            false,
        );
        image(&mut cpu, 0x190000000, "/usr/lib/back.dylib", None, true);
        cpu.write_bytes(0x190100011, &[4]); // Actual trie WEAK_DEFINITION flag.
        let mut symbols = CacheSymbols::from_images(vec![
            (0x180000000, "/usr/lib/front.dylib".into()),
            (0x190000000, "/usr/lib/back.dylib".into()),
            (0x190000000, "/usr/lib/back-alias.dylib".into()),
        ]);
        symbols.ensure_metadata(&cpu, 0).unwrap();
        symbols.ensure_metadata(&cpu, 1).unwrap();
        let weak = CacheDefinition {
            address: 0x190000020,
            weak: true,
        };
        assert_eq!(
            symbols.resolve_definition(&cpu, "/usr/lib/front.dylib", "_foo"),
            Ok(Some(weak))
        );
        // An explicit weak facade reexport must expose a strong terminal as
        // strong. Neither facade flags nor shared addresses determine it.
        symbols.metadata.get_mut(&0).unwrap().symbols.insert(
            "_foo".into(),
            (
                exports::ExportTarget::Reexport {
                    ordinal: 1,
                    name: "_foo".into(),
                },
                true,
            ),
        );
        symbols
            .metadata
            .get_mut(&1)
            .unwrap()
            .symbols
            .get_mut("_foo")
            .unwrap()
            .1 = false;
        assert_eq!(
            symbols.resolve_definition(&cpu, "/usr/lib/front.dylib", "_foo"),
            Ok(Some(CacheDefinition {
                address: 0x190000020,
                weak: false
            }))
        );
        assert_eq!(
            symbols.resolve_definition(&cpu, "/usr/lib/back-alias.dylib", "_foo"),
            Ok(Some(weak))
        );
    }

    #[test]
    fn cached_inherited_reexports_are_resolved_lazily() {
        let mut cpu = A64Cpu::new_sparse();
        image(
            &mut cpu,
            0x180000000,
            "/usr/lib/front.dylib",
            Some("/usr/lib/back.dylib"),
            false,
        );
        image(&mut cpu, 0x190000000, "/usr/lib/back.dylib", None, true);
        let mut symbols = CacheSymbols::from_images(vec![
            (0x180000000, "/usr/lib/front.dylib".into()),
            (0x190000000, "/usr/lib/back.dylib".into()),
        ]);
        assert_eq!(
            symbols.resolve(&cpu, "/usr/lib/front.dylib", "_foo"),
            Ok(Some(0x190000020))
        );
        assert_eq!(symbols.loaded_images(), 2);
    }

    #[test]
    fn cached_exports_reject_malformed_metadata_and_unmapped_trie() {
        let mut cpu = A64Cpu::new_sparse();
        image(&mut cpu, 0x180000000, "/usr/lib/test.dylib", None, true);
        cpu.write_bytes(0x180000000 + 20, &u32::MAX.to_le_bytes());
        let mut symbols =
            CacheSymbols::from_images(vec![(0x180000000, "/usr/lib/test.dylib".into())]);
        assert!(symbols
            .resolve(&cpu, "/usr/lib/test.dylib", "_foo")
            .unwrap_err()
            .contains("load-command limit"));
        let mut cpu = A64Cpu::new_sparse();
        image(&mut cpu, 0x180000000, "/usr/lib/test.dylib", None, true);
        let header = read(&cpu, 0x180000000, 32).unwrap();
        let export = 0x180000000 + 32 + u32_at(&header, 20).unwrap() as u64 - 16;
        cpu.write_bytes(export + 8, &0x9000u32.to_le_bytes());
        assert!(symbols
            .resolve(&cpu, "/usr/lib/test.dylib", "_foo")
            .is_err());
    }

    #[test]
    fn cached_exports_charge_retained_names_and_hash_buckets() {
        let mut cpu = A64Cpu::new_sparse();
        image(&mut cpu, 0x180000000, "/usr/lib/test.dylib", None, true);
        let mut symbols =
            CacheSymbols::from_images(vec![(0x180000000, "/usr/lib/test.dylib".into())]);
        symbols.retained_bytes = MAX_RETAINED - 10;
        let before_raw = symbols.metadata_bytes;
        let before_retained = symbols.retained_bytes;
        assert!(symbols
            .resolve(&cpu, "/usr/lib/test.dylib", "_foo")
            .unwrap_err()
            .contains("retained cached symbol index"));
        assert_eq!(symbols.loaded_images(), 0);
        assert_eq!(symbols.metadata_bytes, before_raw);
        assert_eq!(symbols.retained_bytes, before_retained);
    }
}
