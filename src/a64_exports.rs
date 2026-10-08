/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bounded export parsing for ordinary ARM64 Mach-O images.
use std::collections::{HashMap, HashSet};

// Apple's mach-o/loader.h defines STUB_AND_RESOLVER as 0x10, and its
// terminal contains two ULEBs (stub offset followed by resolver offset).
// https://github.com/apple-oss-distributions/xnu/blob/main/EXTERNAL_HEADERS/mach-o/loader.h
const EXPORT_STUB_AND_RESOLVER: u64 = 0x10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    Address(u64),
    Reexport { ordinal: u64, name: String },
    StubAndResolver { stub: u64, resolver: u64 },
    ThreadLocal(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub name: String,
    pub flags: u64,
    pub target: ExportTarget,
}

/// Resolve an exported name through explicit ordinal aliases and inherited
/// LC_REEXPORT_DYLIB visibility. Ordinals are one-based dependency ordinals,
/// interpreted by the owning image, rather than indices in the global graph.
/// Resolver functions and TLS descriptors are never mistaken for code addresses.
pub fn resolve_symbol<L, O, I>(
    image: usize,
    name: &str,
    lookup: L,
    ordinal_image: O,
    inherited_images: I,
) -> Result<Option<u64>, String>
where
    L: FnMut(usize, &str) -> Option<ExportTarget>,
    O: FnMut(usize, u64) -> Result<usize, String>,
    I: FnMut(usize) -> Vec<usize>,
{
    resolve_symbol_with_resolver(
        image,
        name,
        lookup,
        ordinal_image,
        inherited_images,
        |_, name, _, _| Err(format!("symbol {name} requires an unsupported resolver")),
    )
}

/// The caller must initialize and validate a resolver's runtime dependencies.
/// This hook receives both addresses; the stub is never treated as its result.
/// The ordinary API above intentionally keeps rejecting resolver exports.
pub fn resolve_symbol_with_resolver<L, O, I, R>(
    image: usize,
    name: &str,
    mut lookup: L,
    mut ordinal_image: O,
    mut inherited_images: I,
    mut resolver: R,
) -> Result<Option<u64>, String>
where
    L: FnMut(usize, &str) -> Option<ExportTarget>,
    O: FnMut(usize, u64) -> Result<usize, String>,
    I: FnMut(usize) -> Vec<usize>,
    R: FnMut(usize, &str, u64, u64) -> Result<u64, String>,
{
    struct Search<L, O, I, R> {
        lookup: L,
        ordinal_image: O,
        inherited_images: I,
        resolver: R,
        active: HashSet<(usize, String)>,
        visits: usize,
    }
    impl<L, O, I, R> Search<L, O, I, R>
    where
        L: FnMut(usize, &str) -> Option<ExportTarget>,
        O: FnMut(usize, u64) -> Result<usize, String>,
        I: FnMut(usize) -> Vec<usize>,
        R: FnMut(usize, &str, u64, u64) -> Result<u64, String>,
    {
        fn visit(
            &mut self,
            image: usize,
            name: &str,
            depth: usize,
            alias: bool,
        ) -> Result<Option<u64>, String> {
            if name.is_empty() || name.len() > 4096 || name.contains('\0') {
                return Err("invalid reexport symbol name".into());
            }
            self.visits += 1;
            if depth > 128 || self.visits > 4096 {
                return Err("reexport traversal limit exceeded".into());
            }
            let key = (image, name.to_owned());
            if !self.active.insert(key.clone()) {
                // Inherited visibility can contain ordinary dependency cycles.
                // Skip that branch; an explicit alias cycle is malformed.
                return if alias {
                    Err("cycle in explicit symbol reexports".into())
                } else {
                    Ok(None)
                };
            }
            let result = match (self.lookup)(image, name) {
                Some(ExportTarget::Address(value)) => Ok(Some(value)),
                Some(ExportTarget::Reexport {
                    ordinal,
                    name: renamed,
                }) => {
                    if ordinal == 0 {
                        return Err("invalid reexport ordinal zero".into());
                    }
                    let dependency = (self.ordinal_image)(image, ordinal)?;
                    let target_name = if renamed.is_empty() { name } else { &renamed };
                    self.visit(dependency, target_name, depth + 1, true)
                }
                Some(ExportTarget::StubAndResolver { stub, resolver }) => {
                    let address = (self.resolver)(image, name, stub, resolver)?;
                    if address == 0 {
                        Err(format!("resolver for {name} returned null"))
                    } else {
                        Ok(Some(address))
                    }
                }
                Some(ExportTarget::ThreadLocal(_)) => Err(format!(
                    "symbol {name} requires unsupported thread-local storage"
                )),
                None => {
                    let dependencies = (self.inherited_images)(image);
                    if dependencies.len() > 128 {
                        return Err("too many inherited reexport images".into());
                    }
                    let mut found = None;
                    for dependency in dependencies {
                        if let Some(value) = self.visit(dependency, name, depth + 1, false)? {
                            found = Some(value);
                            break;
                        }
                    }
                    Ok(found)
                }
            };
            self.active.remove(&key);
            result
        }
    }
    Search {
        lookup: &mut lookup,
        ordinal_image: &mut ordinal_image,
        inherited_images: &mut inherited_images,
        resolver: &mut resolver,
        active: HashSet::new(),
        visits: 0,
    }
    .visit(image, name, 0, false)
}

fn range(bytes: &[u8], offset: usize, size: usize) -> Result<&[u8], String> {
    let end = offset.checked_add(size).ok_or("export range overflow")?;
    bytes
        .get(offset..end)
        .ok_or_else(|| "export range outside image".into())
}

#[test]
fn opt_in_resolver_receives_defining_image_and_both_addresses() {
    let result = resolve_symbol_with_resolver(
        0,
        "_facade",
        |image, name| match (image, name) {
            (0, "_facade") => Some(ExportTarget::Reexport {
                ordinal: 1,
                name: "_target".into(),
            }),
            (1, "_target") => Some(ExportTarget::StubAndResolver {
                stub: 0x1000,
                resolver: 0x2000,
            }),
            _ => None,
        },
        |_, _| Ok(1),
        |_| vec![],
        |image, name, stub, resolver| {
            assert_eq!(
                (image, name, stub, resolver),
                (1, "_target", 0x1000, 0x2000)
            );
            Ok(0x3000)
        },
    );
    assert_eq!(result, Ok(Some(0x3000)));
    assert!(resolve_symbol_with_resolver(
        0,
        "_target",
        |_, _| Some(ExportTarget::StubAndResolver {
            stub: 1,
            resolver: 2
        }),
        |_, _| Ok(0),
        |_| vec![],
        |_, _, _, _| Ok(0)
    )
    .unwrap_err()
    .contains("null"));
}
fn word(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        range(bytes, offset, 4)?.try_into().unwrap(),
    ))
}
fn quad(bytes: &[u8], offset: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        range(bytes, offset, 8)?.try_into().unwrap(),
    ))
}
fn uleb(bytes: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for index in 0..10 {
        let byte = *bytes.get(*cursor).ok_or("truncated export ULEB128")?;
        *cursor += 1;
        if index == 9 && byte > 1 {
            return Err("export ULEB128 overflow".into());
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("export ULEB128 overflow".into())
}
fn string(bytes: &[u8], cursor: &mut usize) -> Result<String, String> {
    let start = *cursor;
    let tail = bytes.get(start..).ok_or("export string outside image")?;
    let length = tail
        .iter()
        .take(4097)
        .position(|b| *b == 0)
        .ok_or("unterminated export string")?;
    if length > 4096 {
        return Err("export string too long".into());
    }
    *cursor = start + length + 1;
    String::from_utf8(tail[..length].to_vec()).map_err(|_| "non-UTF8 export name".into())
}
fn address(base: u64, offset: u64) -> Result<u64, String> {
    base.checked_add(offset)
        .ok_or_else(|| "export address overflow".into())
}

/// Resolve regular trie offsets against the mapped image base. Absolute exports
/// retain their original value. Unsupported executable export kinds stay explicit.
pub fn parse_trie(trie: &[u8], image_base: u64) -> Result<Vec<Export>, String> {
    if trie.is_empty() {
        return Ok(Vec::new());
    }
    enum Visit {
        Enter(usize, String),
        Exit(usize),
    }
    let mut stack = vec![Visit::Enter(0, String::new())];
    let mut active = HashSet::new();
    let mut names = HashSet::new();
    let mut exports = Vec::new();
    let mut visits = 0usize;
    let mut pending_name_bytes = 0usize;
    let mut exported_name_bytes = 0usize;
    let limit = trie.len().saturating_mul(2).min(1_000_000);
    while let Some(visit) = stack.pop() {
        let (offset, prefix) = match visit {
            Visit::Exit(offset) => {
                active.remove(&offset);
                continue;
            }
            Visit::Enter(offset, prefix) => {
                pending_name_bytes -= prefix.len();
                (offset, prefix)
            }
        };
        visits += 1;
        if visits > limit {
            return Err("export trie traversal limit exceeded".into());
        }
        if !active.insert(offset) {
            return Err("cycle in export trie".into());
        }
        let mut cursor = offset;
        let terminal_size = usize::try_from(uleb(trie, &mut cursor)?)
            .map_err(|_| "export terminal size overflow")?;
        let terminal = range(trie, cursor, terminal_size)?;
        cursor = cursor
            .checked_add(terminal_size)
            .ok_or("export terminal overflow")?;
        if terminal_size != 0 {
            exported_name_bytes += prefix.len();
            if exported_name_bytes > 16 * 1024 * 1024 {
                return Err("export name storage limit exceeded".into());
            }
            if prefix.is_empty() || !names.insert(prefix.clone()) {
                return Err("empty or duplicate export name".into());
            }
            let mut position = 0;
            let flags = uleb(terminal, &mut position)?;
            if flags & !0x1f != 0 || flags & 3 == 3 {
                return Err("unsupported export flags".into());
            }
            let target = if flags & 8 != 0 {
                if flags & (EXPORT_STUB_AND_RESOLVER | 3) != 0 {
                    return Err("invalid reexport flags".into());
                }
                let ordinal = uleb(terminal, &mut position)?;
                if ordinal == 0 {
                    return Err("invalid reexport ordinal".into());
                }
                let name = string(terminal, &mut position)?;
                ExportTarget::Reexport {
                    ordinal,
                    name: if name.is_empty() {
                        prefix.clone()
                    } else {
                        name
                    },
                }
            } else {
                let value = uleb(terminal, &mut position)?;
                if flags & EXPORT_STUB_AND_RESOLVER != 0 {
                    if flags & 3 != 0 {
                        return Err("invalid resolver export kind".into());
                    }
                    let resolver = uleb(terminal, &mut position)?;
                    ExportTarget::StubAndResolver {
                        stub: address(image_base, value)?,
                        resolver: address(image_base, resolver)?,
                    }
                } else {
                    match flags & 3 {
                        0 => ExportTarget::Address(address(image_base, value)?),
                        1 => ExportTarget::ThreadLocal(value),
                        2 => ExportTarget::Address(value),
                        _ => unreachable!(),
                    }
                }
            };
            if position != terminal.len() {
                return Err("trailing export terminal bytes".into());
            }
            exports.push(Export {
                name: prefix.clone(),
                flags,
                target,
            });
        }
        let children = usize::from(*trie.get(cursor).ok_or("missing export child count")?);
        cursor += 1;
        let mut edges = Vec::with_capacity(children);
        for _ in 0..children {
            let edge = string(trie, &mut cursor)?;
            // Some producers place a prefix's terminal on a separate child
            // reached by an empty edge. Apple's processExportNode traverses
            // that child without extending the name. Active-node cycle and
            // traversal/storage limits still bound zero-length edges.
            if prefix.len().saturating_add(edge.len()) > 4096 {
                return Err("export name too long".into());
            }
            let child = usize::try_from(uleb(trie, &mut cursor)?)
                .map_err(|_| "export child offset overflow")?;
            if child >= trie.len() {
                return Err("export child outside trie".into());
            }
            pending_name_bytes += prefix.len() + edge.len();
            if pending_name_bytes > 16 * 1024 * 1024 || stack.len() + edges.len() > 100_000 {
                return Err("export trie pending storage limit exceeded".into());
            }
            edges.push(Visit::Enter(child, format!("{prefix}{edge}")));
        }
        stack.push(Visit::Exit(offset));
        stack.extend(edges.into_iter().rev());
    }
    Ok(exports)
}

/// Parse a thin ARM64 image, preferring LC_DYLD_EXPORTS_TRIE / dyld-info exports.
/// LC_SYMTAB is a fallback for external defined symbols in ordinary dylibs.
pub fn parse_exports(bytes: &[u8], image_base: u64) -> Result<Vec<Export>, String> {
    range(bytes, 0, 32)?;
    if word(bytes, 0)? != 0xfeedfacf || word(bytes, 4)? != 0x0100000c {
        return Err("exports require a thin ARM64 Mach-O".into());
    }
    let count = word(bytes, 16)? as usize;
    let commands_size = word(bytes, 20)? as usize;
    range(bytes, 32, commands_size)?;
    if count > commands_size / 8 {
        return Err("invalid export load command count".into());
    }
    let commands_end = 32 + commands_size;
    let mut cursor = 32;
    let mut trie = None;
    let mut symtab = None;
    let mut preferred_base = None;
    for _ in 0..count {
        let command = word(bytes, cursor)?;
        let size = word(bytes, cursor + 4)? as usize;
        if size < 8
            || size % 8 != 0
            || cursor
                .checked_add(size)
                .is_none_or(|end| end > commands_end)
        {
            return Err("invalid export load command size".into());
        }
        let data = range(bytes, cursor, size)?;
        match command {
            0x80000033 => {
                if size < 16 {
                    return Err("truncated exports trie command".into());
                }
                let candidate = (word(data, 8)? as usize, word(data, 12)? as usize);
                if candidate.1 != 0 {
                    if trie.is_some_and(|old| old != candidate) {
                        return Err("conflicting export tries".into());
                    }
                    trie = Some(candidate);
                }
            }
            0x22 | 0x80000022 => {
                if size < 48 {
                    return Err("truncated dyld info command".into());
                }
                let candidate = (word(data, 40)? as usize, word(data, 44)? as usize);
                if candidate.1 != 0 {
                    if trie.is_some_and(|old| old != candidate) {
                        return Err("conflicting export tries".into());
                    }
                    trie = Some(candidate);
                }
            }
            2 => {
                if size < 24 || symtab.is_some() {
                    return Err("invalid symbol table command".into());
                }
                symtab = Some((
                    word(data, 8)? as usize,
                    word(data, 12)? as usize,
                    word(data, 16)? as usize,
                    word(data, 20)? as usize,
                ));
            }
            0x19 => {
                if size < 72 {
                    return Err("truncated export segment command".into());
                }
                if &data[8..24] == b"__TEXT\0\0\0\0\0\0\0\0\0\0" {
                    preferred_base = Some(quad(data, 24)?);
                }
            }
            _ => (),
        }
        cursor += size;
    }
    if cursor != commands_end {
        return Err("export command size mismatch".into());
    }
    if let Some((offset, size)) = trie {
        return parse_trie(range(bytes, offset, size)?, image_base);
    }
    let Some((offset, count, string_offset, string_size)) = symtab else {
        return Ok(Vec::new());
    };
    let symbols = range(
        bytes,
        offset,
        count.checked_mul(16).ok_or("symbol table size overflow")?,
    )?;
    let strings = range(bytes, string_offset, string_size)?;
    let mut result = HashMap::new();
    for symbol in symbols.chunks_exact(16) {
        let kind = symbol[4];
        if kind & 0xe0 != 0 || kind & 1 == 0 {
            continue;
        }
        let typ = kind & 0x0e;
        if typ != 2 && typ != 0x0e {
            continue;
        }
        let mut name_offset = word(symbol, 0)? as usize;
        let name = string(strings, &mut name_offset)?;
        if name.is_empty() {
            return Err("empty external symbol name".into());
        }
        let value = quad(symbol, 8)?;
        let target = if typ == 2 {
            value
        } else {
            if symbol[5] == 0 {
                return Err("defined export has no section".into());
            }
            let relative = value
                .checked_sub(preferred_base.ok_or("symbol exports need __TEXT base")?)
                .ok_or("symbol export before image base")?;
            address(image_base, relative)?
        };
        let export = Export {
            name: name.clone(),
            flags: if typ == 2 { 2 } else { 0 },
            target: ExportTarget::Address(target),
        };
        if result.insert(name, export).is_some() {
            return Err("duplicate symbol export".into());
        }
    }
    let mut result: Vec<_> = result.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_edge_preserves_prefix_terminal_and_remains_bounded() {
        // Minimized form of the injected dylib's _GADLogFormat node:
        // child "v" and child "" each lead to an ordinary terminal.
        let trie = [
            0, 1, b'x', 0, 5, 0, 2, b'v', 0, 12, 0, 16, 2, 0, 0x20, 0, 2, 0, 0x30, 0,
        ];
        let parsed = super::parse_trie(&trie, 0x1000).unwrap();
        assert_eq!(
            parsed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["xv", "x"]
        );
        assert!(super::parse_trie(&[0, 1, 0, 0], 0)
            .unwrap_err()
            .contains("cycle"));
        assert!(super::parse_trie(
            &[0, 1, b'x', 0, 5, 2, 0, 0x20, 1, 0, 12, 0, 2, 0, 0x30, 0],
            0
        )
        .unwrap_err()
        .contains("duplicate"));
    }

    #[test]
    #[ignore = "requires explicitly supplied thin injected Quest dylib"]
    fn actual_quest_injected_exports() {
        let path = std::env::var_os("A64_QUEST_DYLIB_TEST_PATH").expect("set thin dylib path");
        let data = std::fs::read(path).unwrap();
        let exports = super::parse_exports(&data, 0x100000000).unwrap();
        assert!(exports.iter().any(|export| export.name == "_GADLogFormat"));
        assert!(exports.iter().any(|export| export.name == "_GADLogFormatv"));
    }
    use super::*;

    #[test]
    fn reexports_use_owner_ordinals_and_renamed_targets() {
        let result = resolve_symbol(
            7,
            "_alias",
            |image, name| match (image, name) {
                (7, "_alias") => Some(ExportTarget::Reexport {
                    ordinal: 2,
                    name: "_real".into(),
                }),
                (42, "_real") => Some(ExportTarget::Address(0x1234)),
                _ => None,
            },
            |owner, ordinal| {
                assert_eq!((owner, ordinal), (7, 2));
                Ok(42)
            },
            |_| Vec::new(),
        )
        .unwrap();
        assert_eq!(result, Some(0x1234));
        let result = resolve_symbol(
            0,
            "_same",
            |image, name| match (image, name) {
                (0, "_same") => Some(ExportTarget::Reexport {
                    ordinal: 1,
                    name: String::new(),
                }),
                (1, "_same") => Some(ExportTarget::Address(9)),
                _ => None,
            },
            |_, _| Ok(1),
            |_| Vec::new(),
        )
        .unwrap();
        assert_eq!(result, Some(9));
    }

    #[test]
    fn inherited_visibility_skips_cycles_and_finds_siblings() {
        let result = resolve_symbol(
            0,
            "_f",
            |image, _| {
                if image == 2 {
                    Some(ExportTarget::Address(123))
                } else {
                    None
                }
            },
            |_, _| Err("ordinal unexpectedly queried".into()),
            |image| match image {
                0 => vec![1, 2],
                1 => vec![0],
                _ => Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(result, Some(123));
        assert_eq!(
            resolve_symbol(0, "_missing", |_, _| None, |_, _| Ok(0), |_| Vec::new()).unwrap(),
            None
        );
    }

    #[test]
    fn explicit_reexport_errors_and_unsupported_targets_are_rejected() {
        let cycle = resolve_symbol(
            0,
            "_f",
            |_, _| {
                Some(ExportTarget::Reexport {
                    ordinal: 1,
                    name: "_f".into(),
                })
            },
            |_, _| Ok(0),
            |_| Vec::new(),
        )
        .unwrap_err();
        assert!(cycle.contains("cycle"));
        let bad_ordinal = resolve_symbol(
            0,
            "_f",
            |_, _| {
                Some(ExportTarget::Reexport {
                    ordinal: 0,
                    name: "_f".into(),
                })
            },
            |_, _| Ok(0),
            |_| Vec::new(),
        )
        .unwrap_err();
        assert!(bad_ordinal.contains("ordinal"));
        assert!(resolve_symbol(
            0,
            "_f",
            |_, _| Some(ExportTarget::Reexport {
                ordinal: 5,
                name: "_f".into()
            }),
            |_, _| Err("ordinal outside dependencies".into()),
            |_| Vec::new()
        )
        .is_err());
        for target in [
            ExportTarget::ThreadLocal(1),
            ExportTarget::StubAndResolver {
                stub: 1,
                resolver: 2,
            },
        ] {
            assert!(resolve_symbol(
                0,
                "_f",
                |_, _| Some(target.clone()),
                |_, _| Ok(0),
                |_| Vec::new()
            )
            .is_err());
        }
    }

    #[test]
    fn reexport_graph_traversal_is_bounded() {
        let error = resolve_symbol(0, "_f", |_, _| None, |_, _| Ok(0), |image| vec![image + 1])
            .unwrap_err();
        assert!(error.contains("limit"));
        assert!(resolve_symbol(0, "_f", |_, _| None, |_, _| Ok(0), |_| vec![1; 129]).is_err());
    }

    fn leaf(flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0, 1, b'_', b'f', 0, 6, (1 + payload.len()) as u8, flags];
        bytes.extend_from_slice(payload);
        bytes.push(0);
        bytes
    }
    #[test]
    fn regular_absolute_and_weak_exports() {
        assert_eq!(
            parse_trie(&leaf(0, &[0x20]), 0x1000).unwrap()[0].target,
            ExportTarget::Address(0x1020)
        );
        assert_eq!(
            parse_trie(&leaf(2, &[0x20]), 0x1000).unwrap()[0].target,
            ExportTarget::Address(0x20)
        );
        assert_eq!(parse_trie(&leaf(4, &[0x20]), 0x1000).unwrap()[0].flags, 4);
    }
    #[test]
    fn nonordinary_exports_remain_explicit() {
        assert_eq!(
            parse_trie(&leaf(8, &[1, 0]), 0).unwrap()[0].target,
            ExportTarget::Reexport {
                ordinal: 1,
                name: "_f".into()
            }
        );
        assert_eq!(
            parse_trie(&leaf(0x10, &[0x10, 0x20]), 0x1000).unwrap()[0].target,
            ExportTarget::StubAndResolver {
                stub: 0x1010,
                resolver: 0x1020
            }
        );
        assert_eq!(
            parse_trie(&leaf(1, &[0x10]), 0).unwrap()[0].target,
            ExportTarget::ThreadLocal(0x10)
        );
    }
    #[test]
    fn apple_cached_resolver_terminal_consumes_both_offsets() {
        // Original iOS 16.7.16 CoreGraphics _CGBufIsConstantValue terminal:
        // 10 f4 b9 fc 01 9c ca c5 01. Offsets are stub=4136180,
        // resolver=3237148, not a regular address followed by junk bytes.
        let target = parse_trie(
            &leaf(0x10, &[0xf4, 0xb9, 0xfc, 1, 0x9c, 0xca, 0xc5, 1]),
            0x188786000,
        )
        .unwrap()[0]
            .target
            .clone();
        assert_eq!(
            target,
            ExportTarget::StubAndResolver {
                stub: 0x188786000 + 4136180,
                resolver: 0x188786000 + 3237148
            }
        );
        assert!(resolve_symbol(
            0,
            "_f",
            |_, _| Some(target.clone()),
            |_, _| Ok(0),
            |_| vec![]
        )
        .unwrap_err()
        .contains("unsupported resolver"));
        assert!(parse_trie(&leaf(0x10, &[0x10]), 0).is_err());
        assert!(parse_trie(&leaf(0x10, &[0x10, 0x20, 0x30]), 0)
            .unwrap_err()
            .contains("trailing export terminal bytes"));
        assert!(parse_trie(&leaf(0x18, &[1, 0]), 0)
            .unwrap_err()
            .contains("invalid reexport flags"));
        // The distinct/static resolver extension is not implemented and must
        // not be accepted as a callable regular address or as flag 0x10.
        assert!(parse_trie(&leaf(0x20, &[0x10, 0x20]), 0)
            .unwrap_err()
            .contains("unsupported export flags"));
    }
    #[test]
    fn malformed_trie_is_bounded() {
        assert!(parse_trie(&[0, 1, b'x', 0, 0], 0)
            .unwrap_err()
            .contains("cycle"));
        assert!(parse_trie(&[0, 1, b'x', 0, 99], 0).is_err());
        assert!(parse_trie(&[3, 0], 0).is_err());
        assert!(parse_trie(&[0, 1, b'x'], 0).is_err());
        assert!(parse_trie(&[0x80; 11], 0).unwrap_err().contains("overflow"));
        assert!(parse_trie(&leaf(0, &[1]), u64::MAX).is_err());
        assert!(parse_trie(&leaf(3, &[0]), 0).is_err());
    }
    #[test]
    fn image_command_bounds_are_checked() {
        let mut image = vec![0; 32];
        image[0..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        image[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        image[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_exports(&image, 0).is_err());
    }

    #[test]
    fn image_export_command_resolves_mapped_base() {
        let trie = leaf(0, &[0x20]);
        let mut image = vec![0; 48];
        image[0..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        image[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        image[16..20].copy_from_slice(&1u32.to_le_bytes());
        image[20..24].copy_from_slice(&16u32.to_le_bytes());
        image[32..36].copy_from_slice(&0x80000033u32.to_le_bytes());
        image[36..40].copy_from_slice(&16u32.to_le_bytes());
        image[40..44].copy_from_slice(&48u32.to_le_bytes());
        image[44..48].copy_from_slice(&(trie.len() as u32).to_le_bytes());
        image.extend_from_slice(&trie);
        assert_eq!(
            parse_exports(&image, 0x2000).unwrap()[0].target,
            ExportTarget::Address(0x2020)
        );
        image[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_exports(&image, 0).is_err());
    }

    #[test]
    fn symbol_table_fallback_preserves_absolute_and_slides_section() {
        let mut image = vec![0; 168];
        image[0..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        image[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        image[16..20].copy_from_slice(&2u32.to_le_bytes());
        image[20..24].copy_from_slice(&96u32.to_le_bytes());
        image[32..36].copy_from_slice(&0x19u32.to_le_bytes());
        image[36..40].copy_from_slice(&72u32.to_le_bytes());
        image[40..46].copy_from_slice(b"__TEXT");
        image[56..64].copy_from_slice(&0x1000u64.to_le_bytes());
        image[104..108].copy_from_slice(&2u32.to_le_bytes());
        image[108..112].copy_from_slice(&24u32.to_le_bytes());
        image[112..116].copy_from_slice(&128u32.to_le_bytes());
        image[116..120].copy_from_slice(&2u32.to_le_bytes());
        image[120..124].copy_from_slice(&160u32.to_le_bytes());
        image[124..128].copy_from_slice(&8u32.to_le_bytes());
        image[128..132].copy_from_slice(&1u32.to_le_bytes());
        image[132] = 0x0f;
        image[133] = 1;
        image[136..144].copy_from_slice(&0x1020u64.to_le_bytes());
        image[144..148].copy_from_slice(&4u32.to_le_bytes());
        image[148] = 3;
        image[152..160].copy_from_slice(&0x33u64.to_le_bytes());
        image[160..168].copy_from_slice(b"\0_f\0_a\0\0");
        let exports = parse_exports(&image, 0x2000).unwrap();
        assert_eq!(exports[0].name, "_a");
        assert_eq!(exports[0].target, ExportTarget::Address(0x33));
        assert_eq!(exports[1].target, ExportTarget::Address(0x2020));
    }
}
