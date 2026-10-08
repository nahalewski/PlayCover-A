/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Bind ordinary app images against the original cache; defer runtime/app startup.
//! Only the explicit UUID/byte-audited capability resolvers may execute.
use super::{cache, cache_map, cache_symbols::CacheSymbols, linker, A64Cpu};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const IOSURFACE_PRIVATE: &str = "/System/Library/PrivateFrameworks/IOSurface.framework/IOSurface";
const IOSURFACE_PUBLIC: &str = "/System/Library/Frameworks/IOSurface.framework/IOSurface";
const PROVEN_IOS16_CACHE_UUID: [u8; 16] = [
    0x32, 0x03, 0x55, 0x64, 0x85, 0x3b, 0x38, 0x8b, 0xa1, 0xef, 0x2e, 0xa3, 0x54, 0xc1, 0x96, 0xe1,
];
const PROVEN_IOS11_CACHE_UUID: [u8; 16] = [
    0x73, 0x36, 0xd7, 0x5f, 0x30, 0x14, 0x33, 0xe7, 0x84, 0x3f, 0xe1, 0xf3, 0x52, 0x2f, 0xc5, 0x2f,
];

fn aliases_for_uuid(uuid: &[u8; 16], paths: &HashSet<String>) -> HashMap<String, String> {
    // Proven filesystem link in firmware 20H392, recorded with its OS image
    // SHA256 in ios-runtime/iosurface-alias-proof.json. This rule is scoped to
    // that original shared cache, not arbitrary extracted files or basenames.
    // The same link was independently verified in 15G77; its evidence is in
    // ios-runtime/legacy-iosurface-alias-proof.json.
    if matches!(uuid, &PROVEN_IOS16_CACHE_UUID | &PROVEN_IOS11_CACHE_UUID)
        && paths.contains(IOSURFACE_PUBLIC)
    {
        HashMap::from([(IOSURFACE_PRIVATE.into(), IOSURFACE_PUBLIC.into())])
    } else {
        HashMap::new()
    }
}

fn verified_cache_aliases(
    plan: &cache::CachePlan,
    paths: &HashSet<String>,
) -> Result<HashMap<String, String>, String> {
    let Some(main) = plan.files.first() else {
        return Ok(HashMap::new());
    };
    let mut file =
        File::open(main).map_err(|error| format!("cannot verify cache alias UUID: {error}"))?;
    file.seek(SeekFrom::Start(0x58))
        .map_err(|error| error.to_string())?;
    let mut uuid = [0; 16];
    file.read_exact(&mut uuid)
        .map_err(|error| format!("cannot read cache alias UUID: {error}"))?;
    Ok(aliases_for_uuid(&uuid, paths))
}

pub(super) struct PreparedCacheApp {
    // Kept private deliberately: a successful binding pass is not permission
    // to enter uninitialized Apple frameworks or the application's main.
    pub(super) link: linker::PreparedLink,
    pub(super) _plan: cache::CachePlan,
}

impl PreparedCacheApp {
    pub(super) fn execution_gate(&self) -> Result<(), String> {
        Err(format!("Cache preparation does not execute: {} cached dependencies have not undergone Apple runtime initialization; resolver/TLS semantics, Darwin/Mach services, Objective-C/Swift startup and framework integration remain required", self.link.cached_dependencies.len()))
    }

    pub(super) fn read_pointer(&self, address: u64) -> Option<u64> {
        self.link.loaded.cpu.read_u64(address)
    }
}

/// Execute only the original libSystem initializer in an isolated diagnostic
/// CPU. No dependency or framework-loaded receipt is issued.
pub(super) fn initializer_test(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    bundle_initializer_test(bytes, executable_path, cache_path, reader, true)
}
/// Generic actual-bundle diagnostic. Additional Unity mapping is explicit;
/// non-Unity applications acquire no synthetic Unity dependency.
pub(super) fn bundle_initializer_test(
    bytes: &[u8], executable_path: &str, cache_path: &Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>, with_unity: bool,
) -> Result<(), String> {
    let (mut prepared,inputs)=prepare_bundle_initialization(bytes,executable_path,cache_path,reader,with_unity)?;
    let services=prepared.link.host_services.as_mut().ok_or("selected services absent")?;
    let super::cached_session::Profile::Modern(routes)=inputs.profile else{
        return Err("original iOS11 startup requires the explicitly retained legacy session diagnostic".into());
    };
    super::cache_init_probe::execute(&mut prepared.link.loaded.cpu,&prepared._plan,services,
        inputs.arguments,&prepared.link.main_stack,routes.slide_route,routes.restricted_entry,
        routes.immutable_route,routes.tlv_images,routes.sdk_query,routes.objc_callbacks,routes.cache_range)
}
/// Shared actual mapping/argument provenance for the disposable diagnostic and
/// persistent owner. Neither path receives an initialization receipt here.
pub(super) fn prepare_bundle_initialization(
    bytes:&[u8],executable_path:&str,cache_path:&Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,
) -> Result<(PreparedCacheApp,super::cached_session::Inputs),String> {
    prepare_bundle_initialization_inner(bytes,executable_path,cache_path,reader,with_unity,false)
}
pub(super) fn prepare_bundle_initialization_image_infos(
    bytes:&[u8],executable_path:&str,cache_path:&Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,
) -> Result<(PreparedCacheApp,super::cached_session::Inputs),String> {
    prepare_bundle_initialization_inner(bytes,executable_path,cache_path,reader,with_unity,true)
}
fn prepare_bundle_initialization_inner(
    bytes:&[u8],executable_path:&str,cache_path:&Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,with_unity:bool,image_info_compatibility:bool,
) -> Result<(PreparedCacheApp,super::cached_session::Inputs),String> {
    let file = super::thin_arm64_slice(bytes)?;
    let metadata = super::MachO64::parse_metadata(file)?;
    let main_header = metadata
        .segments
        .iter()
        .find(|segment| segment.fileoff == 0 && segment.filesize >= 32)
        .ok_or("actual main Mach-O header segment missing")?
        .vmaddr;
    let bundle = executable_path
        .rsplit_once('/')
        .ok_or("main bundle path missing")?
        .0;
    let additional = if with_unity { vec![format!(
        "{bundle}/Frameworks/UnityFramework.framework/UnityFramework"
    )] } else { Vec::new() };
    let mut prepared = prepare_with_reader_selected_additional(
        bytes,
        executable_path,
        cache_path,
        super::host_services::Selection {
            core_foundation: true,
            objc_lifetime: true,
        },
        reader,
        &additional,
        image_info_compatibility,
    )?;
    let main_cache=prepared._plan.files.first().ok_or("original cache file identity absent")?;
    let cache_header=prepared._plan.regions.iter().find(|region|&region.file==main_cache&&region.file_offset==0)
        .ok_or("original cache header mapping absent")?.vmaddr;
    let mut uuid=[0;16];prepared.link.loaded.cpu.read_guest_into(cache_header.checked_add(88).ok_or("cache UUID address overflow")?,&mut uuid)?;
    if uuid==[0x73,0x36,0xd7,0x5f,0x30,0x14,0x33,0xe7,0x84,0x3f,0xe1,0xf3,0x52,0x2f,0xc5,0x2f]{
        let catalogue=prepared._plan.images.iter().map(|image|(image.path.clone(),image.address)).collect();
        let selected=super::image_infos::cached_closure(&prepared.link.loaded.cpu,&catalogue,prepared.link.cached_dependencies.iter().cloned())?;
        let mut entries=Vec::new();let mut objc_candidates=Vec::new();
        for image in &prepared.link.images{
            let metadata=super::MachO64::parse_metadata(&image.file)?;
            let header=metadata.segments.iter().find(|segment|segment.fileoff==0&&segment.filesize>=32).ok_or("legacy selected image header missing")?.vmaddr.checked_add(image.slide).ok_or("legacy selected image slide overflow")?;
            entries.push((header,i64::try_from(image.slide).map_err(|_|"legacy image slide exceeds intptr_t")?));
            objc_candidates.push((header,image.path.clone(),false));
        }
        entries.extend(selected.iter().map(|image|(image.header,0i64)));
        objc_candidates.extend(selected.iter().map(|image|(image.header,image.path.clone(),true)));
        let mut objc_seen=HashSet::new();objc_candidates.retain(|(header,_,_)|objc_seen.insert(*header));
        let objc_candidates=super::legacy_objc_notify::bottom_up(&prepared.link.loaded.cpu,objc_candidates)?;
        let mut objc_images=Vec::new();for(header,path,cached)in objc_candidates{if let Some(image)=super::legacy_objc_notify::read_image(&prepared.link.loaded.cpu,header,path,cached)?{objc_images.push(image);}}
        let mut seen=HashSet::new();entries.retain(|&(header,_)|seen.insert(header));
        let notifications=entries.iter().map(|&(header,slide)|super::legacy_add_images::Image::read(&prepared.link.loaded.cpu,header,slide)).collect::<Result<Vec<_>,String>>()?;
        let legacy_budget=if objc_images.is_empty(){None}else{Some(super::legacy_initializer_budget::LegacyBudget::verified(&prepared.link.loaded.cpu,&prepared._plan,&objc_images)?)};
        let slides=super::dyld_slide::ImageSlides::new(&entries)?;
        let lookup=prepared.link.host_services.as_mut().ok_or("legacy selected services absent")?
            .install_legacy_dyld_lookup(&mut prepared.link.loaded.cpu,&prepared._plan,slides,notifications,objc_images)?;
        echo!("[a64] original15G77 dyld2 lookup callback published={lookup:#x}; genuine mapped image-slide ledger, unsupported services remain explicit");
        let arguments=initialization_arguments(&mut prepared,main_header,executable_path)?;
        return Ok((prepared,super::cached_session::Inputs{arguments,profile:super::cached_session::Profile::Legacy(legacy_budget)}));
    }
    let program_sdk=super::dyld_sdk_query::ProgramSdk::parse(file)?;
    let mut entries: Vec<(u64,i64)> = prepared._plan.images.iter().map(|image| (image.address,0)).collect();
    for image in &prepared.link.images {
        let metadata=super::MachO64::parse_metadata(&image.file)?;
        let header=metadata.segments.iter().find(|s| s.fileoff==0 && s.filesize>=32)
            .ok_or("loaded image lacks genuine header segment")?.vmaddr.checked_add(image.slide)
            .ok_or("loaded slide header overflow")?;
        entries.push((header,i64::try_from(image.slide).map_err(|_| "loaded slide exceeds intptr_t")?));
    }
    let slides=super::dyld_slide::ImageSlides::new(&entries)?;
    let mut tlv_images=entries[prepared._plan.images.len()..].iter().map(|&(header,slide)|(header,slide as u64,false)).collect::<Vec<_>>();
    for path in &prepared.link.cached_dependencies {
        let image=prepared._plan.images.iter().find(|image|&image.path==path)
            .ok_or_else(||format!("loaded cached TLV image identity missing: {path}"))?;
        tlv_images.push((image.address,0,true));
    }
    let mut immutable_ranges=Vec::new();
    let mut retain_range=|start:u64,size:u64,cache:bool|->Result<(),String> {
        let end=start.checked_add(size).ok_or("permanent image range overflow")?;
        let mut cursor=start;
        while cursor<end {
            let region=prepared.link.loaded.cpu.protection_region(cursor).ok_or("permanent image region missing")?;
            let limit=region.base.checked_add(region.len).ok_or("protection region overflow")?.min(end);
            if limit<=cursor {return Err("invalid protection extent in image ledger".into());}
            immutable_ranges.push((cursor,limit,region.max_protection,cache));cursor=limit;
        }
        Ok(())
    };
    for region in &prepared._plan.regions {retain_range(region.vmaddr,region.size,true)?;}
    for image in &prepared.link.images {
        let metadata=super::MachO64::parse_metadata(&image.file)?;
        for segment in &metadata.segments {
            if segment.vmsize!=0 && segment.initprot!=0 {
                retain_range(segment.vmaddr.checked_add(image.slide).ok_or("image segment slide overflow")?,segment.vmsize,false)?;
            }
        }
    }
    let immutable_ranges=super::dyld_slide::ImmutableRanges::new(immutable_ranges)?;
    let mut symbols=super::cache_symbols::CacheSymbols::new(&prepared._plan);
    let slide_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","__dyld_get_image_slide")?
        .ok_or("genuine libdyld slide export missing")?;
    if slide_definition.weak {return Err("dyld slide export is unexpectedly weak".into());}
    let restricted_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","_dyld_process_is_restricted")?
        .ok_or("genuine libdyld restricted-query export missing")?;
    if restricted_definition.weak {return Err("dyld restricted-query export is unexpectedly weak".into());}
    let immutable_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","__dyld_is_memory_immutable")?
        .ok_or("genuine libdyld immutable-query export missing")?;
    if immutable_definition.weak {return Err("dyld immutable-query export unexpectedly weak".into());}
    let sdk_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","_dyld_program_sdk_at_least")?
        .ok_or("genuine libdyld program SDK query export missing")?;
    if sdk_definition.weak {return Err("dyld SDK-query export unexpectedly weak".into());}
    let callbacks_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","__dyld_objc_register_callbacks")?
        .ok_or("genuine libdyld ObjC callback export missing")?;
    if callbacks_definition.weak {return Err("dyld ObjC callback export unexpectedly weak".into());}
    let selectors_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","__dyld_get_objc_selector")?
        .ok_or("genuine libdyld selector-query export missing")?;
    if selectors_definition.weak{return Err("dyld selector-query export unexpectedly weak".into());}
    let selectors=super::dyld_selectors::SelectorTable::read(&prepared.link.loaded.cpu,&prepared._plan)?;
    let selector_target=prepared.link.host_services.as_mut().ok_or("selected services absent")?
        .install_dyld_selectors(&mut prepared.link.loaded.cpu,selectors_definition.address,selectors)?;
    echo!("[a64] genuine original-cache selector lookup entry={:#x} target={selector_target:#x}; cache-table canonical identities, actual misses only",selectors_definition.address);
    let main_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","__dyld_get_prog_image_header")?
        .ok_or("genuine libdyld main-header export missing")?;
    if main_definition.weak{return Err("dyld main-header export unexpectedly weak".into());}
    let actual_main=super::dyld_main_header::MainHeader::read(&prepared.link.loaded.cpu,main_header)?;
    let main_target=prepared.link.host_services.as_mut().ok_or("selected services absent")?
        .install_dyld_main_header(&mut prepared.link.loaded.cpu,main_definition.address,actual_main)?;
    echo!("[a64] genuine selected main-header query entry={:#x} target={main_target:#x} actual_main={main_header:#x}; no initialization receipt",main_definition.address);
    let range_definition=symbols.resolve_definition(&prepared.link.loaded.cpu,
        "/usr/lib/system/libdyld.dylib","__dyld_get_shared_cache_range")?
        .ok_or("genuine libdyld cache range export missing")?;
    if range_definition.weak{return Err("dyld cache range export unexpectedly weak".into());}
    let cache_range=super::dyld_cache_range::CacheRange::read(&prepared.link.loaded.cpu,&prepared._plan)?;
    let catalogue=prepared._plan.images.iter().map(|image|(image.path.clone(),image.address)).collect();
    let cached=super::image_infos::cached_closure(&prepared.link.loaded.cpu,&catalogue,
        prepared.link.cached_dependencies.iter().cloned())?;
    let cached_candidates=cached.len();
    let mut objc_images=Vec::new();let mut seen=std::collections::HashSet::new();
    for (index,image) in prepared.link.images.iter().enumerate(){
        let header=entries[prepared._plan.images.len()+index].0;
        if seen.insert(header){if let Some(image)=super::dyld_objc_callbacks::ObjcImage::read(
            &prepared.link.loaded.cpu,header,image.path.clone(),false)
            .map_err(|error|format!("ordinary ObjC notification image {} at {header:#x}: {error}",image.path))?{objc_images.push(image);}}
    }
    for image in cached {
        if seen.insert(image.header){if let Some(image)=super::dyld_objc_callbacks::ObjcImage::read(
            &prepared.link.loaded.cpu,image.header,image.path.clone(),true)
            .map_err(|error|format!("cached ObjC notification image {} at {:#x}: {error}",image.path,image.header))?{objc_images.push(image);}}
    }
    echo!("[a64] ObjC notification provenance: {} selected ordinary images, {} direct cache dependencies, {cached_candidates} transitive cached candidates, {} unique headers, {} actual ObjC image-info sections; whole-cache catalogue has {} images and is not notified wholesale",
        prepared.link.images.len(),prepared.link.cached_dependencies.len(),seen.len(),objc_images.len(),prepared._plan.images.len());
    let arguments=initialization_arguments(&mut prepared,main_header,executable_path)?;
    let inputs=super::cached_session::Inputs{arguments,profile:super::cached_session::Profile::Modern(super::cached_session::ModernInputs{
        slide_route:(slide_definition.address,slides),restricted_entry:restricted_definition.address,
        immutable_route:(immutable_definition.address,immutable_ranges),tlv_images,
        sdk_query:Some((sdk_definition.address,program_sdk)),
        objc_callbacks:Some((callbacks_definition.address,objc_images)),
        cache_range:Some((range_definition.address,cache_range))})};
    Ok((prepared,inputs))
}
fn initialization_arguments(prepared:&mut PreparedCacheApp,header:u64,path:&str)->Result<Vec<u64>,String>{
    let services=prepared.link.host_services.as_mut().ok_or("selected services absent")?;
    let mut base=services.scratch_end().checked_add(4095).ok_or("argument arena overflow")?&!4095;
    // Explicit compatibility metadata may already occupy the first pages
    // after bridge scratch. Preserve its RX getter and immutable snapshot.
    let limit=base.checked_add(16*1024*1024).ok_or("argument arena search overflow")?;
    loop{
        let end=base.checked_add(4096).ok_or("argument arena overflow")?;
        if end>limit{return Err("no free bounded argument arena after service scratch".into());}
        if (base..end).all(|address|prepared.link.loaded.cpu.mapped_permissions(address).is_none()){break;}
        base=end;
    }
    let token=super::cache_init_probe::random_munge_token()?;
    let apple=[prepared.link.main_stack.apple_entry(),format!("ptr_munge={token:#x}")];
    super::cache_init_probe::arguments(&mut prepared.link.loaded.cpu,base,header,path,&apple)
}

pub(super) fn prepare_with_reader(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PreparedCacheApp, String> {
    let (plan, cpu) = cache_map::map(cache_path)?;
    prepare_mapped(bytes, executable_path, plan, cpu, reader)
}
/// Explicit loader-owned compatibility ABI. Strict cache export lookup remains
/// the default; this does not authorize execution of uninitialized frameworks.
pub(super) fn prepare_with_reader_image_infos(
    bytes:&[u8],executable_path:&str,cache_path:&Path,
    reader:impl FnMut(&str)->Result<Vec<u8>,String>,
) -> Result<PreparedCacheApp,String> {
    let (plan,cpu)=cache_map::map(cache_path)?;
    prepare_mapped_selection_additional(bytes,executable_path,plan,cpu,None,reader,&[],true)
}

/// Explicit function-subset routing only; original cache providers still must
/// exist and the full cached-runtime execution gate remains unchanged.
pub(super) fn prepare_with_reader_selected(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    selection: super::host_services::Selection,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PreparedCacheApp, String> {
    selection.validate()?;
    let (plan, cpu) = cache_map::map(cache_path)?;
    prepare_mapped_selection(bytes, executable_path, plan, cpu, Some(selection), reader)
}

fn prepare_with_reader_selected_additional(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    selection: super::host_services::Selection,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
    additional: &[String],
    image_info_compatibility:bool,
) -> Result<PreparedCacheApp, String> {
    selection.validate()?;
    let (plan, cpu) = cache_map::map(cache_path)?;
    prepare_mapped_selection_additional(
        bytes,
        executable_path,
        plan,
        cpu,
        Some(selection),
        reader,
        additional,
        image_info_compatibility,
    )
}

/// Diagnostic calls are deliberately limited to registered emulator services.
/// The application's main and all app/cached framework initializers stay gated.
pub(super) fn selected_service_test(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    use super::{
        bridge::GuestCall,
        host_services::{Selection, CORE_FOUNDATION, OBJC},
    };
    let mut prepared = prepare_with_reader_selected(
        bytes,
        executable_path,
        cache_path,
        Selection {
            core_foundation: true,
            objc_lifetime: true,
        },
        reader,
    )?;
    if prepared.link.selected_imports_bound == 0 {
        return Err(
            "selected service diagnostic requires actual app imports routed to services".into(),
        );
    }
    let cpu = &mut prepared.link.loaded.cpu;
    // An isolated diagnostic input mapping. Sparse map overlap refusal is
    // mandatory; this never writes into app, cache, bridge stack or CF arena.
    let input = 0x7000000000;
    cpu.map_zeroed(input, 4096, 3)?;
    let text = b"Terraria selected-service bridge";
    cpu.write_guest_into(input, text)?;
    let services = prepared
        .link
        .host_services
        .as_mut()
        .ok_or("selected bridge missing")?;
    // The fixed probe services may not all appear in this particular app's
    // import table. Independently verify each genuine provider export before
    // permitting any probe call, using the same production route validation.
    let mut originals = CacheSymbols::new(&prepared._plan);
    for (provider, name) in [
        (CORE_FOUNDATION, "_CFStringCreateWithBytes"),
        (CORE_FOUNDATION, "_CFStringGetLength"),
        (CORE_FOUNDATION, "_CFRelease"),
        (OBJC, "_objc_retain"),
    ] {
        let original = originals
            .resolve_definition(cpu, provider, name)?
            .ok_or_else(|| {
                format!("selected diagnostic requires genuine export {provider}:{name}")
            })?;
        let routed = services
            .route(cpu, provider, name, original)?
            .ok_or_else(|| format!("selected diagnostic service not registered: {name}"))?;
        if services.registered_address(provider, name) != Some(routed) {
            return Err("selected diagnostic routing identity mismatch".into());
        }
    }
    let entry = services
        .registered_address(CORE_FOUNDATION, "_CFStringCreateWithBytes")
        .ok_or("CF create service missing")?;
    let string = services
        .call(
            cpu,
            &GuestCall {
                entry,
                integers: vec![0, input, text.len() as u64, 0x08000100, 0],
                ..Default::default()
            },
            1000,
        )?
        .integers[0];
    let entry = services
        .registered_address(CORE_FOUNDATION, "_CFStringGetLength")
        .ok_or("CF length service missing")?;
    let length = services
        .call(
            cpu,
            &GuestCall {
                entry,
                integers: vec![string],
                ..Default::default()
            },
            1000,
        )?
        .integers[0];
    if length != text.len() as u64 {
        return Err("selected CFString length mismatch".into());
    }
    let entry = services
        .registered_address(CORE_FOUNDATION, "_CFRelease")
        .ok_or("CF release service missing")?;
    services.call(
        cpu,
        &GuestCall {
            entry,
            integers: vec![string],
            ..Default::default()
        },
        1000,
    )?;
    let entry = services
        .registered_address(OBJC, "_objc_retain")
        .ok_or("ObjC retain service missing")?;
    if services
        .call(
            cpu,
            &GuestCall {
                entry,
                integers: vec![0],
                ..Default::default()
            },
            1000,
        )?
        .integers[0]
        != 0
    {
        return Err("selected Objective-C nil retain mismatch".into());
    }
    echo!("[a64] selected-service diagnostic passed: {} app import records routed; owned CFString create/length/release and Objective-C nil retain executed through guest trampolines; no app/Apple initializer/main executed", prepared.link.selected_imports_bound);
    prepared
        .execution_gate()
        .expect_err("Apple runtime execution must remain gated");
    Ok(())
}

fn prepare_mapped(
    bytes: &[u8],
    executable_path: &str,
    plan: cache::CachePlan,
    cpu: A64Cpu,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PreparedCacheApp, String> {
    prepare_mapped_selection(bytes, executable_path, plan, cpu, None, reader)
}

/// Execute only a checked diagnostic prefix of LC_MAIN, not normal startup.
pub(super) fn entry_prefix_test(
    bytes: &[u8],
    executable_path: &str,
    cache_path: &Path,
    mut reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    use super::{
        bridge::GuestCall,
        host_services::Selection,
        startup::{DeferredInitialization, PrefixOutcome},
    };
    let file = super::thin_arm64_slice(bytes)?;
    let metadata = super::MachO64::parse_metadata(file)?;
    let bundle_path = executable_path
        .rsplit_once('/')
        .ok_or("main bundle path missing")?
        .0;
    let main_plist = reader(&format!("{bundle_path}/Info.plist"))?;
    let unity_plist = reader(&format!(
        "{bundle_path}/Frameworks/UnityFramework.framework/Info.plist"
    ))?;
    if !matches!(metadata.entry, super::EntryPoint::Main { .. }) {
        return Err("entry prefix requires LC_MAIN ABI".into());
    }
    let ranges = metadata
        .segments
        .iter()
        .filter(|s| s.initprot & 4 != 0 && s.filesize != 0)
        .map(|s| {
            s.vmaddr
                .checked_add(s.filesize)
                .map(|end| (s.vmaddr, end))
                .ok_or("entry executable range overflow")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let requested_frameworks = [format!(
        "{bundle_path}/Frameworks/UnityFramework.framework/UnityFramework"
    )];
    let mut prepared = prepare_with_reader_selected_additional(
        bytes,
        executable_path,
        cache_path,
        Selection {
            core_foundation: true,
            objc_lifetime: true,
        },
        reader,
        &requested_frameworks,
        false,
    )?;
    echo!("[a64] explicitly requested Unity framework mapped/bound into the same CPU/cache graph before main; no framework initialization or bundle-loaded receipt issued");
    let policy = DeferredInitialization {
        app_functions: prepared.link.pending_initializers,
        cached_dependencies: prepared.link.cached_dependencies.len(),
    };
    // Resolve original identities directly, bypassing all selected-service routing.
    let mut original_symbols = super::cache_symbols::CacheSymbols::new(&prepared._plan);
    let foundation_path = "/System/Library/Frameworks/Foundation.framework/Foundation";
    let objc_path = "/usr/lib/libobjc.A.dylib";
    let cf_path = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
    let bundle_definition = original_symbols
        .resolve_definition(
            &prepared.link.loaded.cpu,
            foundation_path,
            "_OBJC_CLASS_$_NSBundle",
        )?
        .ok_or("original NSBundle class export missing")?;
    let message_definition = original_symbols
        .resolve_definition(&prepared.link.loaded.cpu, objc_path, "_objc_msgSend")?
        .ok_or("original objc_msgSend export missing")?;
    let constant_definition = original_symbols
        .resolve_definition(
            &prepared.link.loaded.cpu,
            cf_path,
            "___CFConstantStringClassReference",
        )?
        .ok_or("original constant-string class export missing")?;
    if bundle_definition.weak || message_definition.weak || constant_definition.weak {
        return Err("startup routing requires exact strong original providers".into());
    }
    let constants =
        super::foundation_startup::constants_for_main(file, 0, constant_definition.address)?;
    let imports = if let Some(streams) = metadata.legacy_fixups {
        super::legacy::imports(file, streams, &metadata.segments)?
    } else if metadata.chained_fixups.is_some() {
        super::fixups::imports(file)?
    } else {
        return Err("startup routing requires actual import metadata".into());
    };
    let dependencies = metadata
        .dependencies
        .iter()
        .map(|dependency| dependency.name.clone())
        .collect::<Vec<_>>();
    let cpu = &mut prepared.link.loaded.cpu;
    let call = GuestCall {
        entry: cpu.pc(),
        integers: (0..4).map(|i| cpu.reg(i)).collect(),
        ..Default::default()
    };
    let services = prepared
        .link
        .host_services
        .as_mut()
        .ok_or("entry prefix requires selected bridge")?;
    services.enable_foundation(
        cpu,
        super::foundation_startup::Inputs {
            executable_path: executable_path.into(),
            main_plist,
            unity_plist,
            constants: Some(constants),
        },
    )?;
    let owned = services
        .foundation
        .as_ref()
        .ok_or("owned Foundation absent")?;
    let exports = owned.exports()?;
    let originals = [super::objc_slots::VerifiedClassExport {
        provider: foundation_path,
        name: "_OBJC_CLASS_$_NSBundle",
        original_address: bundle_definition.address,
        weak_definition: bundle_definition.weak,
    }];
    let transaction = super::objc_slots::SlotTransaction::prepare(
        file,
        0,
        &imports,
        &dependencies,
        &originals,
        &exports,
        |name| owned.namespace.selector(name),
        cpu,
    )?;
    let (class_slots, selector_slots) = transaction.commit(cpu)?;
    if class_slots != 1 {
        return Err(format!(
            "expected one proven main NSBundle classref, found {class_slots}"
        ));
    }
    let message_target = services
        .registered_address(objc_path, "_objc_msgSend")
        .ok_or("owned objc_msgSend service missing")?;
    let message_slots = super::objc_slots::patch_message_got(
        file,
        0,
        &dependencies,
        &imports,
        message_definition.address,
        message_target,
        cpu,
    )?;
    let bundle_state = services
        .foundation
        .as_ref()
        .ok_or("owned Foundation absent")?
        .bundles
        .clone();
    let cached_regions = prepared
        ._plan
        .regions
        .iter()
        .filter(|region| region.init_prot & 1 != 0)
        .map(|region| {
            Ok((
                region.vmaddr,
                region
                    .vmaddr
                    .checked_add(region.size)
                    .ok_or("bundle cache range overflow")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let cached_executable_regions = prepared
        ._plan
        .regions
        .iter()
        .filter(|region| region.init_prot & 4 != 0)
        .map(|region| {
            Ok((
                region.vmaddr,
                region
                    .vmaddr
                    .checked_add(region.size)
                    .ok_or("bundle cache RX range overflow")?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let cache_selector_context = super::cache_objc_context::read(cpu, &prepared._plan)?;
    echo!("[a64] original cached libobjc selector optimization v16 validated; relative selector base={:#x}",cache_selector_context.base);
    super::bundle_load::install(
        &bundle_state,
        std::mem::take(&mut prepared.link.images),
        cached_regions,
        cached_executable_regions,
        cache_selector_context,
        services.instruction_ranges(),
    )?;
    echo!("[a64] owned Foundation routing: {class_slots} verified NSBundle classrefs, {selector_slots} canonical selectors, {message_slots} verified objc_msgSend GOT slots; genuine owned NSString initialization completed; cached/app initialization remains deferred");
    match super::startup::run_prefix(cpu,services,&call,&ranges,&policy,10_000)? {
        PrefixOutcome::Boundary{pc,instructions,reason}=>echo!("[a64] entry prefix stopped before runtime boundary at {pc:#x} after {instructions} instructions: {reason}; app startup/gameplay not achieved"),
        PrefixOutcome::Returned{values,instructions}=>echo!("[a64] entry prefix returned {} after {instructions} instructions; initialization policy remained deferred, normal app startup/gameplay not established",values.integers[0]),
    }
    prepared
        .execution_gate()
        .expect_err("full Apple initialization gate must remain closed");
    Ok(())
}
fn prepare_mapped_selection(
    bytes: &[u8],
    executable_path: &str,
    plan: cache::CachePlan,
    cpu: A64Cpu,
    selection: Option<super::host_services::Selection>,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PreparedCacheApp, String> {
    prepare_mapped_selection_additional(bytes, executable_path, plan, cpu, selection, reader, &[],false)
}
fn prepare_mapped_selection_additional(
    bytes: &[u8],
    executable_path: &str,
    plan: cache::CachePlan,
    cpu: A64Cpu,
    selection: Option<super::host_services::Selection>,
    reader: impl FnMut(&str) -> Result<Vec<u8>, String>,
    additional: &[String],
    image_info_compatibility: bool,
) -> Result<PreparedCacheApp, String> {
    let mapping_top = plan
        .regions
        .iter()
        .map(|region| {
            region
                .vmaddr
                .checked_add(region.size)
                .ok_or("cache mapping end overflow")
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    let paths = plan
        .images
        .iter()
        .map(|image| image.path.clone())
        .collect::<HashSet<_>>();
    let symbols = CacheSymbols::new(&plan);
    let aliases = verified_cache_aliases(&plan, &paths)?;
    // Fileless plans are synthetic metadata-only fixtures; they cannot opt in
    // to cache resolver execution. Real caches require an audited UUID.
    let resolvers = if plan.files.is_empty() {
        None
    } else {
        Some(super::cache_resolver::AuditedResolvers::for_plan(&plan)?)
    };
    let link = linker::prepare_with_cache_additional(
        bytes,
        executable_path,
        reader,
        cpu,
        linker::CacheContext {
            symbols,
            paths,
            aliases,
            mapping_top,
            resolvers,
            selected: selection,
            host_services: None,
            image_info_catalogue: image_info_compatibility.then(||plan.images.iter().map(|image|(image.path.clone(),image.address)).collect()),
            image_infos: None,
        },
        additional,
    )?;
    echo!("[a64] cache preparation passed: {} ordinary images, {} original-cache dependencies, {} resolved import records ({} against cache); application binding slots written", link.image_count, link.cached_dependencies.len(), link.imports_bound, link.cached_imports_bound);
    echo!(
        "  mapped bytes: {}; app initializer functions deferred: {}; audited capability resolvers executed: {}; no app or Apple runtime initializer/main executed",
        link.loaded.cpu.mapped_bytes(),
        link.pending_initializers,
        link.audited_resolvers_executed
    );
    if selection.is_some() {
        echo!("  selected host-service import records: {}; full Apple runtime initialization still deferred",link.selected_imports_bound);
    }
    let prepared = PreparedCacheApp { link, _plan: plan };
    echo!(
        "  execution gate: {}",
        prepared.execution_gate().unwrap_err()
    );
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iosurface_alias_requires_proven_uuid_and_canonical_cached_image() {
        let paths = HashSet::from([IOSURFACE_PUBLIC.to_owned()]);
        assert_eq!(
            aliases_for_uuid(&PROVEN_IOS16_CACHE_UUID, &paths)
                .get(IOSURFACE_PRIVATE)
                .map(String::as_str),
            Some(IOSURFACE_PUBLIC)
        );
        let mut different = PROVEN_IOS16_CACHE_UUID;
        different[0] ^= 1;
        assert!(aliases_for_uuid(&different, &paths).is_empty());
        assert_eq!(
            aliases_for_uuid(&PROVEN_IOS11_CACHE_UUID, &paths)
                .get(IOSURFACE_PRIVATE)
                .map(String::as_str),
            Some(IOSURFACE_PUBLIC)
        );
        assert!(aliases_for_uuid(&PROVEN_IOS16_CACHE_UUID, &HashSet::new()).is_empty());
    }

    // Build an in-memory cached image with the original header plus export
    // metadata at a separate LINKEDIT VM address. It supplies addresses only;
    // its code is never executed by the cache-preparation path.
    fn cached_answer(cpu: &mut A64Cpu, base: u64, name: &str) {
        let linkedit = base + 0x100000;
        let mut commands = Vec::new();
        let mut segments = Vec::new();
        for (segment_name, address, fileoff) in
            [("__TEXT", base, 0), ("__LINKEDIT", linkedit, 0x8000)]
        {
            let mut command = vec![0u8; 72];
            command[..4].copy_from_slice(&0x19u32.to_le_bytes());
            command[4..8].copy_from_slice(&72u32.to_le_bytes());
            command[8..8 + segment_name.len()].copy_from_slice(segment_name.as_bytes());
            command[24..32].copy_from_slice(&address.to_le_bytes());
            command[32..40].copy_from_slice(&0x1000u64.to_le_bytes());
            command[40..48].copy_from_slice(&(fileoff as u64).to_le_bytes());
            command[48..56].copy_from_slice(&0x1000u64.to_le_bytes());
            commands.extend(command);
            segments.push(address);
        }
        let length = 24 + name.len() + 1;
        let mut identity = vec![0; length];
        identity[..4].copy_from_slice(&0xdu32.to_le_bytes());
        identity[4..8].copy_from_slice(&(length as u32).to_le_bytes());
        identity[8..12].copy_from_slice(&24u32.to_le_bytes());
        identity[24..24 + name.len()].copy_from_slice(name.as_bytes());
        commands.extend(identity);
        // Root edge _answer, terminal regular offset 0x100.
        let trie = [
            0, 1, b'_', b'a', b'n', b's', b'w', b'e', b'r', 0, 11, 3, 0, 0x80, 2, 0,
        ];
        let mut export = vec![0; 16];
        export[..4].copy_from_slice(&0x80000033u32.to_le_bytes());
        export[4..8].copy_from_slice(&16u32.to_le_bytes());
        export[8..12].copy_from_slice(&0x8008u32.to_le_bytes());
        export[12..16].copy_from_slice(&(trie.len() as u32).to_le_bytes());
        commands.extend(export);
        let mut header = vec![0; 32];
        header[..4].copy_from_slice(&0xfeedfacfu32.to_le_bytes());
        header[4..8].copy_from_slice(&0x0100000cu32.to_le_bytes());
        header[12..16].copy_from_slice(&6u32.to_le_bytes());
        header[16..20].copy_from_slice(&4u32.to_le_bytes());
        header[20..24].copy_from_slice(&(commands.len() as u32).to_le_bytes());
        header.extend(commands);
        for address in segments {
            cpu.map_zeroed(address, 0x1000, 1).unwrap();
        }
        cpu.write_bytes(base, &header);
        cpu.write_bytes(linkedit + 8, &trie);
    }

    #[test]
    fn cached_export_is_written_to_application_got_without_execution() {
        let name = "/Test.app/Frameworks/libAnswer.dylib";
        let base = 0x180000000;
        let mut cpu = A64Cpu::new_sparse();
        cached_answer(&mut cpu, base, name);
        let plan = cache::CachePlan {
            files: vec![],
            image_count: 1,
            images: vec![cache::CacheImage {
                address: base,
                path: name.into(),
            }],
            mapped_bytes: 0x2000,
            mapped_span: 0x101000,
            regions: vec![],
        };
        let prepared = prepare_mapped(
            include_bytes!("../tests/a64/import_client.macho"),
            "/Test.app/Test",
            plan,
            cpu,
            |_| Err("cache-backed dependency must not be read as a file".into()),
        )
        .unwrap();
        assert_eq!(prepared.link.cached_imports_bound, 1);
        assert_eq!(prepared.read_pointer(0x100008000), Some(base + 0x100));
        assert!(prepared
            .execution_gate()
            .unwrap_err()
            .contains("have not undergone Apple runtime initialization"));
        assert_eq!(prepared.link.loaded.cpu.pc(), 0x100004000);
    }

    #[test]
    fn proven_private_iosurface_request_binds_canonical_cached_provider() {
        let original = include_bytes!("../tests/a64/import_client.macho");
        let mut bytes = original.to_vec();
        let mut offset = 32;
        let old_end = 32 + super::super::rd_u32(original, 20).unwrap() as usize;
        for _ in 0..super::super::rd_u32(original, 16).unwrap() {
            let length = super::super::rd_u32(original, offset + 4).unwrap() as usize;
            if super::super::rd_u32(original, offset).unwrap() == 0xc {
                let new_length = (24 + IOSURFACE_PRIVATE.len() + 1 + 7) & !7;
                let mut replacement = vec![0; new_length];
                replacement[..24].copy_from_slice(&original[offset..offset + 24]);
                replacement[4..8].copy_from_slice(&(new_length as u32).to_le_bytes());
                replacement[8..12].copy_from_slice(&24u32.to_le_bytes());
                replacement[24..24 + IOSURFACE_PRIVATE.len()]
                    .copy_from_slice(IOSURFACE_PRIVATE.as_bytes());
                let mut commands = original[32..offset].to_vec();
                commands.extend(replacement);
                commands.extend_from_slice(&original[offset + length..old_end]);
                assert!(32 + commands.len() < 0x4000);
                bytes[20..24].copy_from_slice(&(commands.len() as u32).to_le_bytes());
                bytes[32..32 + commands.len()].copy_from_slice(&commands);
                break;
            }
            offset += length;
        }
        let base = 0x180000000;
        let mut cpu = A64Cpu::new_sparse();
        cached_answer(&mut cpu, base, IOSURFACE_PUBLIC);
        let plan = cache::CachePlan {
            files: vec![],
            image_count: 1,
            images: vec![cache::CacheImage {
                address: base,
                path: IOSURFACE_PUBLIC.into(),
            }],
            mapped_bytes: 0x2000,
            mapped_span: 0x101000,
            regions: vec![],
        };
        let paths = HashSet::from([IOSURFACE_PUBLIC.to_owned()]);
        let aliases = aliases_for_uuid(&PROVEN_IOS16_CACHE_UUID, &paths);
        let prepared = linker::prepare_with_cache(
            &bytes,
            "/Test.app/Test",
            |_| Err("alias must resolve only through the canonical cache image".into()),
            cpu,
            linker::CacheContext {
                symbols: CacheSymbols::new(&plan),
                paths,
                aliases,
                mapping_top: 0,
                resolvers: None,
                selected: None,
                host_services: None,
                image_info_catalogue: None,
                image_infos: None,
            },
        )
        .unwrap();
        assert_eq!(
            prepared.loaded.cpu.read_u64(0x100008000),
            Some(base + 0x100)
        );
        assert_eq!(prepared.cached_dependencies, [IOSURFACE_PUBLIC]);
    }
}
