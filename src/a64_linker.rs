/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Small bounded linker for independently loadable experimental ARM64 images.
use super::*;
use std::collections::{HashMap, HashSet};

const PAGE: u64 = 0x4000;
const MAX_MAPPED: u64 = A64Cpu::MAX_MAPPED_BYTES as u64;
const MAX_IMAGES: usize = 128;

struct Image {
    path: String,
    file: Vec<u8>,
    macho: MachO64,
    slide: u64,
    dependency_images: Vec<Option<usize>>,
    cached_dependencies: HashMap<usize, String>,
    symbols: HashMap<String, exports::ExportTarget>,
    symbol_flags: HashMap<String, u64>,
}

enum DependencySource {
    Image(String, Vec<u8>),
    Cache(String),
}

pub(super) struct CacheContext {
    pub symbols: cache_symbols::CacheSymbols,
    pub paths: HashSet<String>,
    pub aliases: HashMap<String, String>,
    pub mapping_top: u64,
    pub resolvers: Option<cache_resolver::AuditedResolvers>,
    pub selected: Option<host_services::Selection>,
    pub host_services: Option<host_services::SelectedServices>,
    pub image_info_catalogue: Option<HashMap<String,u64>>,
    pub image_infos: Option<image_infos::ImageInfos>,
}

impl CacheContext {
    fn definition(
        &mut self,
        cpu: &mut A64Cpu,
        path: &str,
        name: &str,
    ) -> Result<Option<cache_symbols::CacheDefinition>, String> {
        let service = &self.resolvers;
        let definition = self.symbols.resolve_definition_with_resolver(
            cpu,
            path,
            name,
            |cpu, _, name, stub, resolver| {
                service
                    .as_ref()
                    .ok_or_else(|| format!("symbol {name} requires an unsupported resolver"))?
                    .resolve(cpu, stub, resolver)
            },
        )?;
        if definition.is_none() {
            if let Some(infos)=&self.image_infos {
                if let Some(address)=infos.compatibility_symbol(path,name) {
                    echo!("[a64] explicit loader-owned compatibility binding {name}; not a cached provider export");
                    return Ok(Some(cache_symbols::CacheDefinition{address,weak:false}));
                }
            }
        }
        if let (Some(services), Some(definition)) = (&self.host_services, definition) {
            if let Some(address) = services.route(cpu, path, name, definition)? {
                return Ok(Some(cache_symbols::CacheDefinition {
                    address,
                    weak: false,
                }));
            }
        }
        Ok(definition)
    }
}

pub(super) struct PreparedLink {
    pub main_stack: super::main_stack::MainStackDescriptor,
    pub images: Vec<PreparedImage>,
    pub loaded: LoadedA64,
    pub cached_dependencies: Vec<String>,
    pub imports_bound: usize,
    pub cached_imports_bound: usize,
    pub image_count: usize,
    pub pending_initializers: usize,
    pub audited_resolvers_executed: usize,
    pub selected_imports_bound: usize,
    pub host_services: Option<host_services::SelectedServices>,
}

/// Original selected image provenance retained after binding. Initializer
/// targets are read from relocated guest memory, never unrelocated file data.
pub(super) struct PreparedImage {
    pub path: String,
    pub file: Vec<u8>,
    pub slide: u64,
    pub initializers: Vec<u64>,
    pub required_dependencies: Vec<String>,
    pub executable_ranges: Vec<(u64, u64)>,
}

fn retained_images(images: Vec<Image>, cpu: &A64Cpu) -> Result<Vec<PreparedImage>, String> {
    let mut result = Vec::new();
    for image in &images {
        let mut total = 0u64;
        let base = image
            .macho
            .segments
            .iter()
            .find(|s| s.fileoff == 0 && s.filesize != 0)
            .ok_or("retained image missing mapped header")?
            .vmaddr
            .checked_add(image.slide)
            .ok_or("retained image base overflow")?;
        let mut initializers = Vec::new();
        for &(address, count, width) in &image.macho.initializer_sections {
            for index in 0..count {
                total += 1;
                if total > super::MAX_INITIALIZERS_PER_IMAGE {
                    return Err("retained initializer per-image budget exceeds 65536".into());
                }
                let slot = index
                    .checked_mul(width)
                    .and_then(|v| address.checked_add(v))
                    .and_then(|v| v.checked_add(image.slide))
                    .ok_or("retained initializer slot overflow")?;
                let target = if width == 4 {
                    let mut raw = [0; 4];
                    cpu.read_into(slot, &mut raw)?;
                    base.checked_add(u32::from_le_bytes(raw) as u64)
                        .ok_or("retained initializer offset overflow")?
                } else if width == 8 {
                    cpu.read_u64(slot)
                        .ok_or("retained initializer pointer unmapped")?
                } else {
                    return Err("retained initializer width unsupported".into());
                };
                if target == 0
                    || target & 3 != 0
                    || !(0..4).all(|offset| {
                        target
                            .checked_add(offset)
                            .and_then(|address| cpu.mapped_permissions(address))
                            .is_some_and(|p| p & 4 != 0)
                    })
                {
                    return Err(format!(
                        "{}: retained initializer {target:#x} not mapped executable",
                        image.path
                    ));
                }
                initializers.push(target);
            }
        }
        let mut dependencies = Vec::new();
        for (ordinal, dependency) in image.dependency_images.iter().enumerate() {
            if let Some(index) = dependency {
                dependencies.push(images[*index].path.clone());
            } else if let Some(path) = image.cached_dependencies.get(&ordinal) {
                dependencies.push(path.clone());
            }
        }
        dependencies.sort();
        dependencies.dedup();
        let ranges = image
            .macho
            .mapped_segments()
            .filter(|s| s.initprot & 4 != 0)
            .map(|s| {
                let start = s
                    .vmaddr
                    .checked_add(image.slide)
                    .ok_or("retained RX range overflow")?;
                Ok((
                    start,
                    start
                        .checked_add(s.vmsize)
                        .ok_or("retained RX range overflow")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        result.push((initializers, dependencies, ranges));
    }
    Ok(images
        .into_iter()
        .zip(result)
        .map(
            |(image, (initializers, required_dependencies, executable_ranges))| PreparedImage {
                path: image.path,
                file: image.file,
                slide: image.slide,
                initializers,
                required_dependencies,
                executable_ranges,
            },
        )
        .collect())
}

fn normalize(path: &str) -> Result<String, String> {
    if !path.starts_with('/') {
        return Err(format!("dependency path must be absolute: {path}"));
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop().ok_or("dependency path escapes guest root")?;
            }
            other => parts.push(other),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn directory(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        .unwrap_or("/")
}

fn expand(path: &str, loader: &str, executable: &str) -> Result<String, String> {
    let expanded = if let Some(tail) = path.strip_prefix("@loader_path/") {
        format!("{}/{tail}", directory(loader))
    } else if let Some(tail) = path.strip_prefix("@executable_path/") {
        format!("{}/{tail}", directory(executable))
    } else if path == "@loader_path" {
        directory(loader).into()
    } else if path == "@executable_path" {
        directory(executable).into()
    } else if path.starts_with('@') {
        return Err(format!("unsupported dependency path token: {path}"));
    } else {
        path.into()
    };
    normalize(&expanded)
}

fn bounds(macho: &MachO64) -> Result<(u64, u64), String> {
    let low = macho
        .mapped_segments()
        .map(|s| s.vmaddr)
        .min()
        .ok_or("no mapped segments")?;
    let high = macho
        .mapped_segments()
        .map(|s| {
            s.vmaddr
                .checked_add(s.vmsize)
                .ok_or("segment address overflow")
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap();
    mapped_size(macho)?;
    let mut ranges: Vec<_> = macho
        .mapped_segments()
        .map(|s| (s.vmaddr, s.vmaddr + s.vmsize))
        .collect();
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("overlapping ARM64 segments are unsupported".into());
    }
    Ok((low, high))
}

fn mapped_size(macho: &MachO64) -> Result<u64, String> {
    let size = macho.mapped_segments().try_fold(0u64, |size, segment| {
        size.checked_add(segment.vmsize)
            .ok_or("mapped-byte count overflow")
    })?;
    if size > MAX_MAPPED {
        return Err("ARM64 image exceeds mapped-byte limit: 512 MiB".into());
    }
    Ok(size)
}

fn align(value: u64) -> Result<u64, String> {
    Ok(value
        .checked_add(PAGE - 1)
        .ok_or("image address overflow")?
        & !(PAGE - 1))
}

fn read_dependency(
    name: &str,
    path: &str,
    executable: &str,
    rpaths: &[String],
    cached_paths: &HashSet<String>,
    cached_aliases: &HashMap<String, String>,
    reader: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<DependencySource, String> {
    let candidates = if let Some(tail) = name.strip_prefix("@rpath/") {
        rpaths
            .iter()
            .map(|root| normalize(&format!("{root}/{tail}")))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![expand(name, path, executable)?]
    };
    let mut errors = Vec::new();
    for candidate in candidates {
        if let Some(canonical) = cached_aliases
            .get(&candidate)
            .filter(|canonical| cached_paths.contains(*canonical))
        {
            return Ok(DependencySource::Cache(canonical.clone()));
        }
        if cached_paths.contains(&candidate) {
            return Ok(DependencySource::Cache(candidate));
        }
        match reader(&candidate) {
            Ok(bytes) => return Ok(DependencySource::Image(candidate, bytes)),
            Err(error) => errors.push(format!("{candidate}: {error}")),
        }
    }
    Err(format!(
        "cannot load dynamic library {name}: {}",
        if errors.is_empty() {
            "no usable LC_RPATH".into()
        } else {
            errors.join("; ")
        }
    ))
}

fn discover(
    index: usize,
    executable: &str,
    inherited_rpaths: &[String],
    images: &mut Vec<Image>,
    indexes: &mut HashMap<String, usize>,
    cached_paths: &HashSet<String>,
    cached_aliases: &HashMap<String, String>,
    reader: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let path = images[index].path.clone();
    let mut rpaths = images[index]
        .macho
        .rpaths
        .iter()
        .map(|p| expand(p, &path, executable))
        .collect::<Result<Vec<_>, _>>()?;
    rpaths.extend_from_slice(inherited_rpaths);
    let dependencies = images[index].macho.dependencies.clone();
    for dependency in dependencies {
        let (dependency_path, bytes) = match read_dependency(
            &dependency.name,
            &path,
            executable,
            &rpaths,
            cached_paths,
            cached_aliases,
            reader,
        ) {
            Ok(DependencySource::Image(path, bytes)) => (path, bytes),
            Ok(DependencySource::Cache(path)) => {
                let ordinal = images[index].dependency_images.len();
                images[index].cached_dependencies.insert(ordinal, path);
                images[index].dependency_images.push(None);
                continue;
            }
            Err(_) if dependency.weak => {
                images[index].dependency_images.push(None);
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(&existing) = indexes.get(&dependency_path) {
            images[index].dependency_images.push(Some(existing));
            continue;
        }
        if images.len() >= MAX_IMAGES {
            return Err("ARM64 dynamic-library count exceeds 128".into());
        }
        let file = thin_arm64_slice(&bytes)?.to_vec();
        let macho = MachO64::parse_image(&file).map_err(|e| format!("{dependency_path}: {e}"))?;
        if macho.file_type != MH_DYLIB {
            return Err(format!("{dependency_path}: dependency is not MH_DYLIB"));
        }
        if let Some(identity) = &macho.install_name {
            // A relocated @rpath provider may keep its original absolute
            // install identity. Permit that narrow relocation case by exact
            // filename; absolute requested libraries still require identity.
            let relocated_rpath_identity = dependency.name.starts_with("@rpath/")
                && identity.starts_with('/')
                && !dependency.name.ends_with('/')
                && !identity.ends_with('/')
                && identity.rsplit('/').next() == dependency.name.rsplit('/').next();
            if identity != &dependency.name
                && identity != &dependency_path
                && !relocated_rpath_identity
            {
                return Err(format!(
                    "{dependency_path}: LC_ID_DYLIB {identity} does not match requested {}",
                    dependency.name
                ));
            }
        } else {
            return Err(format!("{dependency_path}: missing LC_ID_DYLIB"));
        }
        bounds(&macho)?;
        let next = images.len();
        indexes.insert(dependency_path.clone(), next);
        images.push(Image {
            path: dependency_path,
            file,
            macho,
            slide: 0,
            dependency_images: Vec::new(),
            cached_dependencies: HashMap::new(),
            symbols: HashMap::new(),
            symbol_flags: HashMap::new(),
        });
        images[index].dependency_images.push(Some(next));
        discover(
            next,
            executable,
            &rpaths,
            images,
            indexes,
            cached_paths,
            cached_aliases,
            reader,
        )?;
    }
    Ok(())
}

fn symbol(images: &[Image], image: usize, name: &str) -> Result<Option<u64>, String> {
    exports::resolve_symbol(
        image,
        name,
        |owner, symbol| images[owner].symbols.get(symbol).cloned(),
        |owner, ordinal| {
            let dependency = ordinal.checked_sub(1).ok_or("invalid reexport ordinal 0")?;
            let dependency: usize = dependency
                .try_into()
                .map_err(|_| "reexport ordinal too large")?;
            images[owner]
                .dependency_images
                .get(dependency)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    format!(
                        "invalid or unavailable reexport ordinal {ordinal} in {}",
                        images[owner].path
                    )
                })
        },
        |owner| {
            images[owner]
                .macho
                .dependencies
                .iter()
                .enumerate()
                .filter(|(_, dependency)| dependency.reexport)
                .filter_map(|(index, _)| images[owner].dependency_images[index])
                .collect()
        },
    )
}

fn initializer_order(images: &[Image]) -> Result<Vec<usize>, String> {
    fn visit(
        index: usize,
        images: &[Image],
        marks: &mut [u8],
        order: &mut Vec<usize>,
    ) -> Result<(), String> {
        match marks[index] {
            1 => {
                return Err(format!(
                    "initializer execution requires acyclic dependencies: {}",
                    images[index].path
                ))
            }
            2 => return Ok(()),
            _ => (),
        }
        marks[index] = 1;
        for &dependency in images[index].dependency_images.iter().flatten() {
            visit(dependency, images, marks, order)?;
        }
        marks[index] = 2;
        order.push(index);
        Ok(())
    }
    if !images.iter().any(|image| image.macho.has_initializers) {
        return Ok(Vec::new());
    }
    let mut order = Vec::new();
    visit(0, images, &mut vec![0; images.len()], &mut order)?;
    Ok(order)
}

fn run_initializers(
    images: &[Image],
    loaded: &mut LoadedA64,
    return_trap: u64,
) -> Result<(), String> {
    let order = initializer_order(images)?;
    let mut ticks = 1_000_000;
    let saved_context = loaded.cpu.save_context();
    let saved_sp = loaded.cpu.sp();
    let entry_arguments: [u64; 4] = std::array::from_fn(|index| loaded.cpu.reg(index));
    for index in order {
        let mut count = 0u64;
        let image = &images[index];
        let mapped_header = image
            .macho
            .segments
            .iter()
            .find(|segment| segment.fileoff == 0 && segment.filesize != 0)
            .ok_or("initializer image has no mapped Mach-O header")?
            .vmaddr
            .checked_add(image.slide)
            .ok_or("initializer image base overflow")?;
        for &(preferred_address, length, width) in &image.macho.initializer_sections {
            for item in 0..length {
                count += 1;
                if count > super::MAX_INITIALIZERS_PER_IMAGE {
                    return Err("initializer per-image budget exceeds 65536".into());
                }
                let slot = preferred_address
                    .checked_add(image.slide)
                    .and_then(|address| address.checked_add(item * width))
                    .ok_or("initializer slot overflow")?;
                let function = if width == 4 {
                    let raw = loaded
                        .cpu
                        .read_bytes(slot, 4)
                        .ok_or("initializer offset array outside mapped memory")?;
                    mapped_header
                        .checked_add(u32::from_le_bytes(raw.try_into().unwrap()) as u64)
                        .ok_or("initializer function offset overflow")?
                } else {
                    loaded
                        .cpu
                        .read_u64(slot)
                        .ok_or("initializer array outside mapped memory")?
                };
                let executable = images.iter().any(|owner| {
                    owner.macho.mapped_segments().any(|segment| {
                        if segment.initprot & 4 == 0 {
                            return false;
                        }
                        let start = segment.vmaddr + owner.slide;
                        function >= start
                            && function
                                .checked_add(4)
                                .is_some_and(|end| end <= start + segment.vmsize)
                    })
                });
                if function & 3 != 0 || !executable {
                    return Err(format!(
                        "{}: initializer target {function:#x} is not mapped executable code",
                        image.path
                    ));
                }
                loaded.cpu.set_pc(function);
                loaded.cpu.set_sp(saved_sp);
                loaded.cpu.set_reg(A64Cpu::LR, return_trap);
                // Startup argv/envp/apple vectors are still unavailable.
                for register in 0..4 {
                    loaded.cpu.set_reg(register, entry_arguments[register]);
                }
                loop {
                    match loaded.cpu.run_or_step(Some(&mut ticks)) {
                        A64State::Svc(0x7f) if loaded.cpu.pc() == return_trap + 4 => break,
                        A64State::Svc(0x80) => {
                            if let Some(code) = loaded.handle_svc()? {
                                return Err(format!(
                                    "{}: initializer exited process with code {code}",
                                    image.path
                                ));
                            }
                        }
                        A64State::Normal if ticks != 0 => continue,
                        A64State::Normal => {
                            return Err(format!(
                                "{}: initializer tick budget exhausted",
                                image.path
                            ))
                        }
                        state => {
                            return Err(format!(
                                "{}: initializer stopped unexpectedly: {state:?} at {:#x}",
                                image.path,
                                loaded.cpu.pc()
                            ))
                        }
                    }
                }
                loaded.cpu.restore_context(&saved_context);
            }
        }
    }
    Ok(())
}

fn resolve(
    images: &[Image],
    requester: usize,
    import: &fixups::ChainedImport,
) -> Result<u64, String> {
    let owner = &images[requester];
    let (targets, missing_weak_library) = match import.library_ordinal {
        0 => (vec![requester], false),
        -1 => (vec![0], false),
        -2 | -3 => ((0..images.len()).collect(), false),
        ordinal if ordinal > 0 => {
            let dependency = (ordinal - 1) as usize;
            let target = owner.dependency_images.get(dependency).ok_or_else(|| {
                format!(
                    "import {} has invalid library ordinal {ordinal}",
                    import.name
                )
            })?;
            (
                target.iter().copied().collect(),
                target.is_none() && owner.macho.dependencies[dependency].weak,
            )
        }
        ordinal => return Err(format!("unsupported library ordinal {ordinal}")),
    };
    // Follow image load order: keep the first weak definition until a strong
    // definition appears, then keep the first strong definition.
    if import.library_ordinal == -3 {
        let mut address = None;
        for image in targets {
            if let Some(candidate) = symbol(images, image, &import.name)? {
                let flags = images[image].symbol_flags.get(&import.name).ok_or(
                    "weak coalescing through inherited exports needs definition strength metadata",
                )?;
                if flags & 8 != 0 {
                    return Err("weak coalescing through explicit reexports needs definition strength metadata".into());
                }
                if flags & 4 == 0 {
                    return Ok(candidate);
                }
                if address.is_none() {
                    address = Some(candidate);
                }
            }
        }
        if let Some(address) = address {
            return Ok(address);
        }
        if import.weak {
            return Ok(0);
        }
        return Err(format!("missing required weak definition {}", import.name));
    }
    for image in targets {
        if let Some(address) = symbol(images, image, &import.name)? {
            if images[image]
                .symbol_flags
                .get(&import.name)
                .is_some_and(|flags| flags & 4 != 0)
            {
                return resolve(
                    images,
                    requester,
                    &fixups::ChainedImport {
                        library_ordinal: -3,
                        weak: import.weak,
                        name: import.name.clone(),
                        addend: import.addend,
                    },
                );
            }
            return Ok(address);
        }
    }
    if import.weak || missing_weak_library {
        return Ok(0);
    }
    Err(format!(
        "unresolved required symbol {} (library ordinal {}) in {}",
        import.name, import.library_ordinal, owner.path
    ))
}

/// Ordinary images already have an established loader order. Mixed local/cache
/// order is not established: retain the unique winning-address requirement
/// whenever a cached candidate participates.
fn unique_coalesced_definition(
    candidates: impl IntoIterator<Item = (u64, bool, bool)>,
) -> Result<Option<(u64, bool)>, String> {
    let candidates: Vec<_> = candidates.into_iter().collect();
    if candidates.iter().all(|candidate| !candidate.2) {
        let mut first_weak = None;
        for &(address, is_weak, _) in &candidates {
            if !is_weak {
                return Ok(Some((address, false)));
            }
            first_weak.get_or_insert((address, false));
        }
        return Ok(first_weak);
    }
    let mut strong = HashMap::new();
    let mut weak = HashMap::new();
    for (address, is_weak, cached) in candidates {
        let definitions = if is_weak { &mut weak } else { &mut strong };
        definitions
            .entry(address)
            .and_modify(|any_cached| *any_cached |= cached)
            .or_insert(cached);
    }
    let definitions = if strong.is_empty() { weak } else { strong };
    if definitions.len() > 1 {
        return Err(
            "ambiguous mixed local/cache coalescing requires verified global load order".into(),
        );
    }
    Ok(definitions.into_iter().next())
}

fn resolve_cached(
    images: &[Image],
    requester: usize,
    import: &fixups::ChainedImport,
    cpu: &mut A64Cpu,
    context: &mut CacheContext,
) -> Result<(u64, bool), String> {
    if import.library_ordinal > 0 {
        if let Some(path) = images[requester]
            .cached_dependencies
            .get(&((import.library_ordinal - 1) as usize))
        {
            let definition = context.definition(cpu, path, &import.name)?;
            return match definition {
                Some(definition) if definition.weak => resolve_cached(
                    images,
                    requester,
                    &fixups::ChainedImport {
                        library_ordinal: -3,
                        ..import.clone()
                    },
                    cpu,
                    context,
                ),
                Some(definition) => Ok((definition.address, true)),
                None if import.weak => Ok((0, true)),
                None => Err(format!(
                    "unresolved required cached symbol {} in {path}",
                    import.name
                )),
            };
        }
    }
    if import.library_ordinal == -3 {
        let mut candidates = Vec::new();
        for (index, image) in images.iter().enumerate() {
            if let Some(address) = symbol(images, index, &import.name)? {
                let flags = image
                    .symbol_flags
                    .get(&import.name)
                    .ok_or("ordinary inherited weak lookup needs terminal definition strength")?;
                if flags & 8 != 0 {
                    return Err(
                        "ordinary reexport weak lookup needs terminal definition strength".into(),
                    );
                }
                candidates.push((address, flags & 4 != 0, false));
            }
        }
        let mut visited = HashSet::new();
        for image in images {
            for ordinal in 0..image.macho.dependencies.len() {
                if let Some(path) = image.cached_dependencies.get(&ordinal) {
                    if visited.insert(path) {
                        if let Some(definition) = context.definition(cpu, path, &import.name)? {
                            candidates.push((definition.address, definition.weak, true));
                        }
                    }
                }
            }
        }
        return unique_coalesced_definition(candidates.iter().copied())
            .map_err(|error| format!("{error}: symbol {} requested by {}; candidates (address,weak,cached): {candidates:?}", import.name, images[requester].path))?
            .map(Ok)
            .unwrap_or_else(|| {
                if import.weak {
                    Ok((0, false))
                } else {
                    Err(format!("missing required weak definition {}", import.name))
                }
            });
    }
    if import.library_ordinal == -2 {
        for index in 0..images.len() {
            if let Some(address) = symbol(images, index, &import.name)? {
                if images[index]
                    .symbol_flags
                    .get(&import.name)
                    .is_some_and(|flags| flags & 4 != 0)
                {
                    return resolve_cached(
                        images,
                        requester,
                        &fixups::ChainedImport {
                            library_ordinal: -3,
                            ..import.clone()
                        },
                        cpu,
                        context,
                    );
                }
                return Ok((address, false));
            }
        }
        let mut visited = HashSet::new();
        for image in images {
            for ordinal in 0..image.macho.dependencies.len() {
                if let Some(path) = image.cached_dependencies.get(&ordinal) {
                    if visited.insert(path) {
                        if let Some(definition) = context.definition(cpu, path, &import.name)? {
                            if definition.weak {
                                return resolve_cached(
                                    images,
                                    requester,
                                    &fixups::ChainedImport {
                                        library_ordinal: -3,
                                        ..import.clone()
                                    },
                                    cpu,
                                    context,
                                );
                            }
                            return Ok((definition.address, true));
                        }
                    }
                }
            }
        }
    }
    let local_target = match import.library_ordinal {
        0 => Some(requester),
        -1 => Some(0),
        ordinal if ordinal > 0 => images[requester]
            .dependency_images
            .get((ordinal - 1) as usize)
            .copied()
            .flatten(),
        _ => None,
    };
    if let Some(index) = local_target {
        if symbol(images, index, &import.name)?.is_some()
            && images[index]
                .symbol_flags
                .get(&import.name)
                .is_some_and(|flags| flags & 4 != 0)
        {
            return resolve_cached(
                images,
                requester,
                &fixups::ChainedImport {
                    library_ordinal: -3,
                    ..import.clone()
                },
                cpu,
                context,
            );
        }
    }
    resolve(images, requester, import).map(|address| (address, false))
}

pub(super) fn load_with_reader(
    bytes: &[u8],
    executable_path: &str,
    mut reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<LoadedA64, String> {
    Ok(load_core(bytes, executable_path, &mut reader, None, None, true, &[])?.loaded)
}

pub(super) fn prepare_with_cache(
    bytes: &[u8],
    executable_path: &str,
    mut reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
    cpu: A64Cpu,
    context: CacheContext,
) -> Result<PreparedLink, String> {
    load_core(
        bytes,
        executable_path,
        &mut reader,
        Some(cpu),
        Some(context),
        false,
        &[],
    )
}

/// Explicitly requested framework images can be mapped/bound ahead of their
/// NSBundle load request. This adds graph roots, not invented dependency
/// commands or runtime initialization receipts.
pub(super) fn prepare_with_cache_additional(
    bytes: &[u8],
    executable_path: &str,
    mut reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
    cpu: A64Cpu,
    context: CacheContext,
    additional: &[String],
) -> Result<PreparedLink, String> {
    load_core(
        bytes,
        executable_path,
        &mut reader,
        Some(cpu),
        Some(context),
        false,
        additional,
    )
}

fn load_core(
    bytes: &[u8],
    executable_path: &str,
    reader: &mut impl FnMut(&str) -> Result<Vec<u8>, String>,
    existing_cpu: Option<A64Cpu>,
    mut context: Option<CacheContext>,
    execute_initializers: bool,
    additional: &[String],
) -> Result<PreparedLink, String> {
    let executable = normalize(executable_path)?;
    let file = thin_arm64_slice(bytes)?.to_vec();
    let macho = MachO64::parse_image(&file)?;
    if macho.file_type != MH_EXECUTE {
        return Err("main image is not MH_EXECUTE".into());
    }
    let mut images = vec![Image {
        path: executable.clone(),
        file,
        macho,
        slide: 0,
        dependency_images: Vec::new(),
        cached_dependencies: HashMap::new(),
        symbols: HashMap::new(),
        symbol_flags: HashMap::new(),
    }];
    let mut indexes = HashMap::from([(executable.clone(), 0)]);
    let empty_cache = HashSet::new();
    let paths = context
        .as_ref()
        .map(|context| &context.paths)
        .unwrap_or(&empty_cache);
    let empty_aliases = HashMap::new();
    let aliases = context
        .as_ref()
        .map(|context| &context.aliases)
        .unwrap_or(&empty_aliases);
    discover(
        0,
        &executable,
        &[],
        &mut images,
        &mut indexes,
        paths,
        aliases,
        reader,
    )?;
    if additional.len() > 16 || (execute_initializers && !additional.is_empty()) {
        return Err("additional framework root count/execution policy invalid".into());
    }
    let mut requested = HashSet::new();
    for path in additional {
        let path = normalize(path)?;
        if !requested.insert(path.clone()) {
            return Err("duplicate additional framework root".into());
        }
        if indexes.contains_key(&path) {
            continue;
        }
        if images.len() >= MAX_IMAGES {
            return Err("additional framework image limit exceeded".into());
        }
        let file = thin_arm64_slice(&reader(&path)?)?.to_vec();
        let macho = MachO64::parse_image(&file)?;
        if macho.file_type != super::MH_DYLIB {
            return Err(format!(
                "additional requested framework {path} is not MH_DYLIB"
            ));
        }
        let identity = macho
            .install_name
            .as_deref()
            .ok_or("additional framework has no dylib identity")?;
        if identity.rsplit('/').next() != path.rsplit('/').next() {
            return Err(format!(
                "additional framework install identity {identity} does not match {path}"
            ));
        }
        let index = images.len();
        indexes.insert(path.clone(), index);
        images.push(Image {
            path,
            file,
            macho,
            slide: 0,
            dependency_images: Vec::new(),
            cached_dependencies: HashMap::new(),
            symbols: HashMap::new(),
            symbol_flags: HashMap::new(),
        });
        discover(
            index,
            &executable,
            &[],
            &mut images,
            &mut indexes,
            paths,
            aliases,
            reader,
        )?;
    }
    let (_, main_high) = bounds(&images[0].macho)?;
    let mut top = align(main_high)?;
    if let Some(context) = &context {
        top = top.max(align(context.mapping_top)?);
    }
    for (index, image) in images.iter_mut().enumerate() {
        let (low, high) = bounds(&image.macho)?;
        if index != 0 {
            image.slide = top
                .checked_sub(low)
                .ok_or("negative image slides are unsupported")?;
            // Without relocation metadata, moving pointers is unsafe even if
            // exported symbols can be read from an extracted cache image.
            if image.slide != 0
                && image.macho.chained_fixups.is_none()
                && image.macho.legacy_fixups.is_none()
                && image
                    .macho
                    .mapped_segments()
                    .any(|s| s.initprot & 2 != 0 && s.filesize != 0)
            {
                return Err(format!(
                    "{}: relocated writable image lacks supported relocation metadata",
                    image.path
                ));
            }
            top = align(high.checked_add(image.slide).ok_or("slid image overflow")?)?;
        }
        let image_base = low
            .checked_add(image.slide)
            .ok_or("slid image base overflow")?;
        for export in exports::parse_exports(&image.file, image_base)
            .map_err(|e| format!("{}: {e}", image.path))?
        {
            image.symbol_flags.insert(export.name.clone(), export.flags);
            if image.symbols.insert(export.name, export.target).is_some() {
                return Err("duplicate exported symbol".into());
            }
        }
    }
    let trampoline = top;
    let stack_top = top
        .checked_add(PAGE + STACK_SIZE)
        .ok_or("stack address overflow")?;
    let resolver_memory = if context.as_ref().is_some_and(|c| c.resolvers.is_some()) {
        0x3000
    } else {
        0
    };
    let host_memory = context
        .as_ref()
        .and_then(|c| c.selected)
        .map_or(0, |selection| selection.mapped_bytes());
    let mapped = images.iter().try_fold(
        PAGE + STACK_SIZE + resolver_memory + host_memory,
        |total, image| {
            total
                .checked_add(mapped_size(&image.macho)?)
                .ok_or_else(|| "mapped-byte count overflow".to_string())
        },
    )?;
    if mapped > MAX_MAPPED {
        return Err("ARM64 images exceed mapped-byte limit: 512 MiB".into());
    }
    let mut cpu = existing_cpu.unwrap_or_else(A64Cpu::new_sparse);
    for image in &images {
        for segment in image.macho.mapped_segments() {
            if segment.filesize > segment.vmsize {
                return Err("segment file size exceeds virtual size".into());
            }
            let start: usize = segment
                .fileoff
                .try_into()
                .map_err(|_| "file offset too large")?;
            let length: usize = segment
                .filesize
                .try_into()
                .map_err(|_| "segment too large")?;
            let end = start
                .checked_add(length)
                .ok_or("segment file range overflow")?;
            let data = image
                .file
                .get(start..end)
                .ok_or("segment outside image file")?;
            let address = segment
                .vmaddr
                .checked_add(image.slide)
                .ok_or("segment slide overflow")?;
            cpu.map_zeroed(
                address,
                segment.vmsize.try_into().map_err(|_| "segment too large")?,
                segment.initprot,
            )?;
            cpu.write_bytes(address, data);
        }
    }
    cpu.map_zeroed(trampoline, PAGE as usize, 5)?;
    cpu.map_zeroed(trampoline + PAGE, STACK_SIZE as usize, 3)?;
    if let Some(service) = context
        .as_mut()
        .and_then(|context| context.resolvers.as_mut())
    {
        // Every app/embedded image and its normal stack is placed first.
        // The sparse mapper independently refuses overlap with any mapping.
        let scratch = align(stack_top)?;
        service.map_scratch(&mut cpu, scratch)?;
    }
    if let Some(context) = context.as_mut() {
        if let Some(selection) = context.selected {
            selection.validate()?;
            let constants = if selection.core_foundation {
                host_services::cf_constants(&cpu, &mut context.symbols)?
            } else {
                cf_terraria_services::KnownConstants::default()
            };
            let start = align(stack_top)?
                .checked_add(if context.resolvers.is_some() { PAGE } else { 0 })
                .ok_or("host service scratch address overflow")?;
            context.host_services = Some(host_services::SelectedServices::install(
                &mut cpu, start, selection, constants,
            )?);
        }
    }
    let mut resolved_records = 0usize;
    if let Some(context)=context.as_mut() {
        if let Some(catalogue)=&context.image_info_catalogue {
            let mut records=Vec::new();
            for image in &images {
                let segment=image.macho.segments.iter().find(|s|s.fileoff==0&&s.filesize>=32).ok_or("image-info main/header segment missing")?;
                records.push(image_infos::Record{header:segment.vmaddr.checked_add(image.slide).ok_or("image-info header slide overflow")?,path:image.path.clone()});
            }
            let roots=images.iter().flat_map(|i|i.cached_dependencies.values().cloned()).collect::<Vec<_>>();
            records.extend(image_infos::cached_closure(&cpu,catalogue,roots)?);
            let start=context.host_services.as_ref().map(|s|s.scratch_end()).unwrap_or(align(stack_top)?.checked_add(if context.resolvers.is_some(){PAGE}else{0}).ok_or("image-info scratch overflow")?);
            let infos=image_infos::ImageInfos::map(&mut cpu,align(start)?,&records)?;
            echo!("[a64] explicit loader-owned image-info compatibility snapshot: {} actual mapped images; version1, no runtime-init claim",infos.count);
            context.image_infos=Some(infos);
        }
    }
    let mut cached_records = 0usize;
    let mut selected_records = 0usize;
    let mut declarations = HashSet::new();
    for image in &mut images {
        if let Some(streams) = image.macho.legacy_fixups {
            for name in legacy::weak_declarations(&image.file, streams, &image.macho.segments)? {
                // Addressless declarations remain metadata, never definitions.
                if image.symbols.contains_key(&name) {
                    image
                        .symbol_flags
                        .entry(name.clone())
                        .and_modify(|flags| *flags &= !4);
                }
                declarations.insert(name);
            }
        }
    }
    for (index, image) in images.iter().enumerate() {
        let mut records = Vec::new();
        if let Some((start, end)) = image.macho.chained_fixups {
            records.extend(fixups::imports(&image.file[start..end])?);
        }
        if let Some(streams) = image.macho.legacy_fixups {
            records.extend(
                legacy::imports(&image.file, streams, &image.macho.segments)
                    .map_err(|e| format!("{}: {e}", image.path))?,
            );
        }
        let mut addresses = HashMap::new();
        for import in records {
            let (address, cached) = if let Some(context) = &mut context {
                resolve_cached(&images, index, &import, &mut cpu, context)?
            } else {
                (resolve(&images, index, &import)?, false)
            };
            resolved_records += 1;
            cached_records += usize::from(cached);
            selected_records += usize::from(
                context
                    .as_ref()
                    .and_then(|c| c.host_services.as_ref())
                    .is_some_and(|services| services.owns_address(address)),
            );
            addresses.insert((import.library_ordinal, import.name, import.weak), address);
        }
        let mut resolver = |import: &fixups::ChainedImport| {
            addresses
                .get(&(import.library_ordinal, import.name.clone(), import.weak))
                .copied()
                .ok_or_else(|| format!("unprepared binding record {}", import.name))
        };
        if let Some(streams) = image.macho.legacy_fixups {
            legacy::apply_with_weak_notifications(
                &image.file,
                streams,
                &image.macho.segments,
                image.slide,
                &mut cpu,
                &mut resolver,
                |name| {
                    if declarations.contains(name) {
                        Ok(())
                    } else {
                        Err("unprepared weak definition announcement".into())
                    }
                },
            )
            .map_err(|e| format!("{}: {e}", image.path))?;
        }
        if let Some((start, end)) = image.macho.chained_fixups {
            fixups::apply_with_resolver(
                &image.file[start..end],
                &image.macho,
                &mut cpu,
                image.slide,
                &mut resolver,
            )?;
        }
    }
    cpu.write_bytes(
        trampoline,
        &[0x30, 0x00, 0x80, 0xd2, 0x01, 0x10, 0x00, 0xd4],
    );
    let initializer_return = trampoline + 8;
    cpu.write_bytes(initializer_return, &0xd4000fe1u32.to_le_bytes());
    match images[0].macho.entry {
        EntryPoint::Main { entryoff, .. } => {
            let text = images[0]
                .macho
                .segments
                .iter()
                .find(|s| s.fileoff == 0 && s.filesize != 0)
                .ok_or("missing executable text")?;
            if entryoff >= text.filesize {
                return Err("entry point outside text segment".into());
            }
            cpu.set_pc(
                text.vmaddr
                    .checked_add(entryoff)
                    .ok_or("entry point overflow")?,
            );
            for register in 0..4 {
                cpu.set_reg(register, 0);
            }
        }
        EntryPoint::UnixThread { pc, .. } => cpu.set_pc(pc),
        EntryPoint::Dylib => unreachable!(),
    }
    cpu.set_sp(stack_top - 16);
    if matches!(images[0].macho.entry, EntryPoint::Main { .. }) {
        setup_main_arguments(&mut cpu, stack_top, executable_path)?;
    }
    cpu.set_reg(A64Cpu::LR, trampoline);
    let mut loaded = LoadedA64::new(cpu)?;
    if execute_initializers {
        run_initializers(&images, &mut loaded, initializer_return)?;
    }
    let mut cached_dependencies: Vec<_> = images
        .iter()
        .flat_map(|image| image.cached_dependencies.values().cloned())
        .collect();
    cached_dependencies.sort();
    cached_dependencies.dedup();
    let image_count = images.len();
    let pending_initializers = images
        .iter()
        .map(|image| {
            image
                .macho
                .initializer_sections
                .iter()
                .map(|(_, count, _)| *count as usize)
                .sum::<usize>()
        })
        .sum();
    let retained = retained_images(images, &loaded.cpu)?;
    let main_stack = super::main_stack::MainStackDescriptor::from_loader(
        &loaded.cpu,
        1,
        trampoline + PAGE,
        STACK_SIZE,
        trampoline + PAGE,
        STACK_SIZE,
        loaded.cpu.sp(),
    )?;
    Ok(PreparedLink {
        main_stack,
        images: retained,
        selected_imports_bound: selected_records,
        host_services: context
            .as_mut()
            .and_then(|context| context.host_services.take()),
        audited_resolvers_executed: context
            .as_ref()
            .and_then(|c| c.resolvers.as_ref())
            .map_or(0, |service| service.executions()),
        loaded,
        cached_dependencies,
        imports_bound: resolved_records,
        cached_imports_bound: cached_records,
        image_count,
        pending_initializers,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn ordinary_mapping_budget_accepts_512mib_and_rejects_excess_without_allocating() {
        let mut metadata=MachO64::parse_image(include_bytes!("../tests/a64/libInitialized.dylib")).unwrap();
        let mapped=metadata.segments.iter().position(|segment|segment.initprot!=0&&segment.vmsize!=0).unwrap();
        let mut segment=metadata.segments.remove(mapped);
        segment.vmsize=MAX_MAPPED;metadata.segments=vec![segment];
        assert_eq!(mapped_size(&metadata).unwrap(),512*1024*1024);
        metadata.segments[0].vmsize+=1;
        assert!(mapped_size(&metadata).unwrap_err().contains("512 MiB"));
        metadata.segments[0].vmsize=u64::MAX;
        let mut second=metadata.segments[0].clone();second.vmsize=1;second.vmaddr=0x800000000;
        metadata.segments.push(second);
        assert!(mapped_size(&metadata).unwrap_err().contains("overflow"));
        let mut cpu=A64Cpu::new_sparse();
        assert!(cpu.map_zeroed(0x10000,A64Cpu::MAX_MAPPED_BYTES+1,3).unwrap_err().contains("512 MiB"));
        assert!(cpu.read_bytes(0x10000,1).is_none());
    }
    use super::*;

    #[test]
    fn requested_framework_is_bound_in_same_graph_without_running_constructors() {
        let additional = "/App.app/Frameworks/libInitialized.dylib".to_string();
        let mut reader = |path: &str| -> Result<Vec<u8>, String> {
            match path {
                "/App.app/Frameworks/libAnswer.dylib" => {
                    Ok(include_bytes!("../tests/a64/libAnswer.dylib").to_vec())
                }
                "/App.app/Frameworks/libInitialized.dylib" => {
                    Ok(include_bytes!("../tests/a64/libInitialized.dylib").to_vec())
                }
                _ => Err(format!("fixture missing {path}")),
            }
        };
        let prepared = load_core(
            include_bytes!("../tests/a64/import_client.macho"),
            "/App.app/main",
            &mut reader,
            None,
            None,
            false,
            &[additional.clone()],
        )
        .unwrap();
        assert_eq!(prepared.image_count, 3);
        assert_eq!(prepared.pending_initializers, 2);
        assert!(prepared.imports_bound > 0);
        let image = prepared
            .images
            .iter()
            .find(|image| image.path == additional)
            .unwrap();
        let metadata = MachO64::parse_image(&image.file).unwrap();
        let data = metadata
            .segments
            .iter()
            .find(|segment| segment.name == "__DATA")
            .unwrap();
        assert_eq!(
            prepared.loaded.cpu.read_u64(data.vmaddr + image.slide),
            Some(0)
        );
        assert_eq!(image.initializers.len(), 2);
        let mut ranges = prepared
            .images
            .iter()
            .flat_map(|image| image.executable_ranges.iter().copied())
            .collect::<Vec<_>>();
        ranges.sort();
        assert!(ranges.windows(2).all(|pair| pair[0].1 <= pair[1].0));
    }

    #[test]
    fn retained_initializer_evidence_uses_mapped_offsets_and_rejects_unmapped_targets() {
        let file = include_bytes!("../tests/a64/libInitialized.dylib");
        let make = || Image {
            path: "/App.app/Frameworks/libInitialized.dylib".into(),
            file: file.to_vec(),
            macho: MachO64::parse_image(file).unwrap(),
            slide: 0x100000000,
            dependency_images: vec![],
            cached_dependencies: HashMap::new(),
            symbols: HashMap::new(),
            symbol_flags: HashMap::new(),
        };
        let image = make();
        let mut cpu = A64Cpu::new_sparse();
        for segment in image.macho.mapped_segments() {
            cpu.map_zeroed(
                segment.vmaddr + image.slide,
                segment.vmsize as usize,
                segment.initprot,
            )
            .unwrap();
            cpu.try_write_bytes(
                segment.vmaddr + image.slide,
                &file[segment.fileoff as usize..(segment.fileoff + segment.filesize) as usize],
            )
            .unwrap();
        }
        let evidence = retained_images(vec![make()], &cpu).unwrap();
        assert_eq!(evidence[0].initializers.len(), 2);
        assert!(evidence[0].initializers.iter().all(|&entry| evidence[0]
            .executable_ranges
            .iter()
            .any(|&(start, end)| entry >= start && entry < end)));
        let (slot, _, width) = image.macho.initializer_sections[0];
        assert_eq!(width, 4);
        cpu.try_write_bytes(slot + image.slide, &u32::MAX.to_le_bytes())
            .unwrap();
        assert!(retained_images(vec![make()], &cpu)
            .err()
            .unwrap()
            .contains("not mapped executable"));
    }

    fn rewrite_dylib_command(file: &[u8], command: u32, name: &str) -> Vec<u8> {
        let mut bytes = file.to_vec();
        let command_end = 32 + rd_u32(file, 20).unwrap() as usize;
        let mut offset = 32;
        for _ in 0..rd_u32(file, 16).unwrap() {
            let size = rd_u32(file, offset + 4).unwrap() as usize;
            if rd_u32(file, offset).unwrap() == command {
                let length = (24 + name.len() + 1 + 7) & !7;
                let mut replacement = vec![0; length];
                replacement[..24].copy_from_slice(&file[offset..offset + 24]);
                replacement[4..8].copy_from_slice(&(length as u32).to_le_bytes());
                replacement[8..12].copy_from_slice(&24u32.to_le_bytes());
                replacement[24..24 + name.len()].copy_from_slice(name.as_bytes());
                let mut commands = file[32..offset].to_vec();
                commands.extend(replacement);
                commands.extend_from_slice(&file[offset + size..command_end]);
                assert!(
                    32 + commands.len() < 0x4000,
                    "fixture header must fit before code"
                );
                bytes[20..24].copy_from_slice(&(commands.len() as u32).to_le_bytes());
                bytes[32..32 + commands.len()].copy_from_slice(&commands);
                return bytes;
            }
            offset += size;
        }
        panic!("fixture dylib command not found");
    }

    #[test]
    fn relocated_rpath_provider_may_keep_absolute_install_identity() {
        let provider = rewrite_dylib_command(
            include_bytes!("../tests/a64/libAnswer.dylib"),
            0xd,
            "/Library/MobileSubstrate/DynamicLibraries/libAnswer.dylib",
        );
        let mut loaded = load_with_reader(
            include_bytes!("../tests/a64/import_client.macho"),
            "/Test.app/Test",
            |path| {
                if path == "/Test.app/Frameworks/libAnswer.dylib" {
                    Ok(provider.clone())
                } else {
                    Err("unexpected provider path".into())
                }
            },
        )
        .unwrap();
        assert_eq!(loaded.run(100_000), Ok(42));
        let different = rewrite_dylib_command(
            include_bytes!("../tests/a64/libAnswer.dylib"),
            0xd,
            "/Library/MobileSubstrate/DynamicLibraries/libDifferent.dylib",
        );
        assert!(load_with_reader(
            include_bytes!("../tests/a64/import_client.macho"),
            "/Test.app/Test",
            |_| Ok(different.clone())
        )
        .err()
        .unwrap()
        .contains("does not match requested"));
        let absolute_client = rewrite_dylib_command(
            include_bytes!("../tests/a64/import_client.macho"),
            0xc,
            "/usr/lib/libAnswer.dylib",
        );
        assert!(
            load_with_reader(&absolute_client, "/Test.app/Test", |_| Ok(provider.clone()))
                .err()
                .unwrap()
                .contains("does not match requested")
        );
    }

    #[test]
    fn weak_lookup_prefers_first_strong_and_requires_nonweak_missing_definition() {
        fn image(path: &str, address: u64) -> Image {
            let file = include_bytes!("../tests/a64/libAnswer.dylib").to_vec();
            Image {
                path: path.into(),
                macho: MachO64::parse_image(&file).unwrap(),
                file,
                slide: 0,
                dependency_images: Vec::new(),
                cached_dependencies: HashMap::new(),
                symbols: HashMap::from([(
                    "_coalesced".into(),
                    exports::ExportTarget::Address(address),
                )]),
                symbol_flags: HashMap::from([("_coalesced".into(), 4)]),
            }
        }
        let mut images = vec![
            image("/first.dylib", 0x1000),
            image("/second.dylib", 0x2000),
        ];
        let mut import = fixups::ChainedImport {
            library_ordinal: -3,
            weak: true,
            name: "_coalesced".into(),
            addend: 0,
        };
        assert_eq!(resolve(&images, 0, &import), Ok(0x1000));
        images[1].symbol_flags.insert("_coalesced".into(), 0);
        assert_eq!(resolve(&images, 0, &import), Ok(0x2000));
        images[1]
            .symbols
            .insert("_coalesced".into(), exports::ExportTarget::Address(0x1000));
        assert_eq!(resolve(&images, 0, &import), Ok(0x1000));
        import.name = "_absent".into();
        assert_eq!(resolve(&images, 0, &import), Ok(0));
        import.weak = false;
        assert!(resolve(&images, 0, &import)
            .unwrap_err()
            .contains("missing required weak definition"));
    }

    #[test]
    fn mixed_coalescing_accepts_unique_winning_strength_only() {
        assert_eq!(
            unique_coalesced_definition([(10, true, false), (20, true, false)]),
            Ok(Some((10, false)))
        );
        assert_eq!(
            unique_coalesced_definition([
                (10, true, false),
                (20, false, false),
                (30, false, false)
            ]),
            Ok(Some((20, false)))
        );
        assert_eq!(
            unique_coalesced_definition([(10, true, false), (20, true, true), (30, false, true)]),
            Ok(Some((30, true)))
        );
        assert!(
            unique_coalesced_definition([(10, false, false), (20, false, true)])
                .unwrap_err()
                .contains("global load order")
        );
        assert!(unique_coalesced_definition([(10, true, false), (20, true, true)]).is_err());
        assert_eq!(
            unique_coalesced_definition([(10, true, false), (10, true, true)]),
            Ok(Some((10, true)))
        );
        assert_eq!(
            unique_coalesced_definition([(10, true, true), (10, false, false)]),
            Ok(Some((10, false)))
        );
        assert_eq!(unique_coalesced_definition([]), Ok(None));
    }

    #[test]
    fn initializer_order_is_dependency_first_and_rejects_cycles() {
        fn image(path: &str, file: &[u8], dependencies: Vec<Option<usize>>) -> Image {
            Image {
                path: path.into(),
                file: file.to_vec(),
                macho: MachO64::parse_image(file).unwrap(),
                slide: 0,
                dependency_images: dependencies,
                cached_dependencies: HashMap::new(),
                symbols: HashMap::new(),
                symbol_flags: HashMap::new(),
            }
        }
        let mut images = vec![
            image(
                "/main",
                include_bytes!("../tests/a64/initializer_client.macho"),
                vec![Some(1)],
            ),
            image(
                "/library",
                include_bytes!("../tests/a64/libInitialized.dylib"),
                vec![],
            ),
        ];
        assert_eq!(initializer_order(&images).unwrap(), [1, 0]);
        images[1].dependency_images.push(Some(0));
        assert!(initializer_order(&images)
            .unwrap_err()
            .contains("acyclic dependencies"));
    }
}
