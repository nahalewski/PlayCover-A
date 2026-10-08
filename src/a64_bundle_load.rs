/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Actual mapped-image registration and bounded constructor/+load receipts.
//! Cached runtime dependencies remain prerequisites, never mapping receipts.
use super::{
    bridge::GuestCall, bundle_services::BundleServices, linker::PreparedImage,
    objc_registration::MappedImage,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

/// Select only the requested image and its actual resolved ordinary providers.
/// Cache providers stay external metadata identities, not substituted classes.
fn dependency_scope(images: &[PreparedImage], requested: &str) -> Result<Vec<usize>, String> {
    if images.is_empty() || images.len() > 128 {
        return Err("bundle dependency scope image limit invalid".into());
    }
    let mut paths = BTreeMap::new();
    for (index, image) in images.iter().enumerate() {
        if paths.insert(image.path.as_str(), index).is_some() {
            return Err("bundle dependency scope image identity duplicated".into());
        }
    }
    let root = *paths
        .get(requested)
        .ok_or("bundle dependency scope requested image missing")?;
    fn visit(
        index: usize,
        images: &[PreparedImage],
        paths: &BTreeMap<&str, usize>,
        marks: &mut [u8],
        order: &mut Vec<usize>,
    ) -> Result<(), String> {
        match marks[index] {
            2 => return Ok(()),
            1 => {
                return Err(format!(
                    "cyclic bundle ordinary dependency scope at {}",
                    images[index].path
                ))
            }
            _ => (),
        }
        marks[index] = 1;
        for dependency in &images[index].required_dependencies {
            if let Some(&provider) = paths.get(dependency.as_str()) {
                visit(provider, images, paths, marks, order)?;
            }
        }
        marks[index] = 2;
        order.push(index);
        Ok(())
    }
    let mut order = Vec::new();
    visit(root, images, &paths, &mut vec![0; images.len()], &mut order)?;
    Ok(order)
}

pub(super) fn install(
    state: &Rc<RefCell<BundleServices>>,
    images: Vec<PreparedImage>,
    cached_regions: Vec<(u64, u64)>,
    cached_executable_regions: Vec<(u64, u64)>,
    cache_selector_context: super::objc_metadata::CacheSelectorContext,
    owned_executable_ranges: Vec<(u64, u64)>,
) -> Result<(), String> {
    if images.is_empty() || images.len() > 128 {
        return Err("bundle load mapped image count invalid".into());
    }
    let mut paths = BTreeSet::new();
    if images.iter().any(|image| !paths.insert(image.path.clone())) {
        return Err("bundle load mapped image identity duplicated".into());
    }
    let weak = Rc::downgrade(state);
    state.borrow_mut().set_load_coordinator(Rc::new(move |frame,id| {
        let state = weak.upgrade().ok_or("bundle loader state expired")?;
        let executable = state.try_borrow().map_err(|_| "reentrant bundle loader metadata")?.model.metadata(id)?.executable_path();
        let image = images.iter().find(|image| image.path == executable)
            .ok_or_else(|| format!("bundle load {executable}: actual selected image is not mapped"))?;
        let empty_weak = BTreeSet::new(); // Nil slots require real binding evidence; none is fabricated.
        let scope=dependency_scope(&images,&executable)?;
        echo!("[a64] actual bundle registration scope: {} ordinary image(s) in requested image/dependency closure; unrelated executable classes excluded",scope.len());
        let selected=scope.iter().map(|&index|&images[index]).collect::<Vec<_>>();
        let mapped = selected.iter().map(|image| MappedImage { identity:&image.path,file:&image.file,
            slide:image.slide,weak_class_ref_slots:&empty_weak }).collect::<Vec<_>>();
        let ranges = selected.iter().flat_map(|image| image.executable_ranges.iter().copied()).chain(owned_executable_ranges.iter().copied()).collect::<Vec<_>>();
        // A physically executable cached IMP can be valid metadata without
        // being authorized for execution. These extra ranges are ONLY passed
        // to read-only class/method registration, never the bridge policy or
        // constructor execution validation below.
        let metadata_ranges = ranges.iter().copied().chain(cached_executable_regions.iter().copied()).collect::<Vec<_>>();
        let loads = {
        let mut metadata_reader = frame.metadata_reader(super::bridge::MAX_METADATA_MEMORY)?;
        let result = super::objc_image_load::prepare_with_cache(&mapped,&cached_regions,Some(cache_selector_context),|address,length|metadata_reader.read(address,length),
            |address,length| if metadata_ranges.iter().any(|&(start,end)| address >= start && address.checked_add(length as u64).is_some_and(|next| next <= end)) {
                Ok(())
            } else { Err(format!("bundle method metadata {address:#x}+{length} is outside physically mapped RX ranges")) })
            ;
        let (charged_bytes,requests)=metadata_reader.statistics();
        echo!("[a64] bundle metadata registration: {charged_bytes} charged readable bytes, {requests} read requests; independent metadata-only limit={} bytes",super::bridge::MAX_METADATA_MEMORY);
        result.map_err(|error|format!("bundle load {executable}: genuine mapped-class registration failed after {charged_bytes} charged bytes/{requests} reads: {error}"))?
        };
        let methods = loads.ordered.iter().filter(|entry| entry.identity == executable).map(|entry|entry.method).collect::<Vec<_>>();
        // No scheduler has initialized the original cached frameworks yet.
        // Reject BEFORE entering any constructor or minting a load ticket.
        if !image.required_dependencies.is_empty() {
            return Err(format!("bundle load {executable}: {} dependency runtime initialization receipts are missing; first required provider {}; {} genuine constructors and {} +load methods remain unexecuted",
                image.required_dependencies.len(),image.required_dependencies[0],image.initializers.len(),methods.len()));
        }
        if loads.registered.images.iter().any(|registered| registered.identity == executable && !registered.classes.is_empty()) {
            return Err(format!("bundle load {executable}: genuine mapped classes validated but publication into the active Objective-C dispatch namespace is still required"));
        }
        let principal_name = state.borrow().model.metadata(id)?.principal_class.clone();
        let principal = principal_name.as_deref().map(|name|loads.registered.principal_class(&executable,name)).transpose()?;
        let calls = image.initializers.len().checked_add(methods.len()).ok_or("bundle continuation count overflow")?;
        if calls > 8 { return Err("bundle constructor/+load sequence exceeds current bounded bridge continuation capacity".into()); }
        state.borrow_mut().model.mark_mapped(id,&executable,&image.initializers,&methods,&[],|target| {
            if ranges.iter().any(|&(start,end)|target >= start && target.checked_add(4).is_some_and(|next|next <= end)) {Ok(())}
            else {Err("bundle initializer is outside actual mapped image RX ranges".into())}
        })?;
        let plan = state.borrow_mut().model.begin_load(id)?;
        let registered = Rc::new(loads.registered);
        if calls == 0 {
            return state.borrow_mut().model.finish_load(plan.ticket,principal,|_,address|registered.validate_image_class(&executable,address));
        }
        let mut sequence = image.initializers.iter().map(|&entry|(GuestCall{entry,..Default::default()},Some(entry),None)).collect::<Vec<_>>();
        sequence.extend(methods.iter().map(|&method|(GuestCall{entry:method.imp,integers:vec![method.receiver,method.selector],..Default::default()},None,Some(method))));
        for (index,(call,initializer,method)) in sequence.into_iter().enumerate() {
            let state = state.clone(); let registered = registered.clone(); let executable = executable.clone(); let ticket = plan.ticket;
            let admission_failure = state.clone();
            let admitted = frame.request_guest_call(call,move |result| {
                let mut state = state.try_borrow_mut().map_err(|_|"reentrant bundle completion")?;
                if let Err(error) = result {
                    let bounded = error.chars().take(2048).collect::<String>();
                    // A later canceled callback can encounter an already failed
                    // ticket. Preserve the first failure and never add receipts.
                    let _ = state.model.fail_load(ticket,&bounded);
                    return Err(error);
                }
                if let Some(address)=initializer {state.model.initializer_completed(ticket,address)?;}
                if let Some(method)=method {state.model.load_method_completed(ticket,method)?;}
                if index + 1 == calls {state.model.finish_load(ticket,principal,|_,address|registered.validate_image_class(&executable,address))?;}
                Ok(())
            });
            if let Err(error) = admitted {
                let bounded = error.chars().take(2048).collect::<String>();
                let _ = admission_failure.borrow_mut().model.fail_load(ticket,&bounded);
                return Err(error);
            }
        }
        Ok(())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn image(path: &str, dependencies: &[&str]) -> PreparedImage {
        PreparedImage {
            path: path.into(),
            file: vec![],
            slide: 0,
            initializers: vec![],
            required_dependencies: dependencies.iter().map(|path| (*path).into()).collect(),
            executable_ranges: vec![],
        }
    }
    #[test]
    fn requested_framework_scope_excludes_unrelated_main_and_keeps_transitive_providers() {
        let images = vec![
            image("/App.app/Main", &["/unrelated"]),
            image("/Unity", &["/Party", "/cached/Foundation"]),
            image("/Party", &["/Shared"]),
            image("/Shared", &[]),
            image("/unrelated", &[]),
        ];
        assert_eq!(dependency_scope(&images, "/Unity").unwrap(), vec![3, 2, 1]);
        assert!(dependency_scope(&images, "/missing").is_err());
    }
    #[test]
    fn scope_deduplicates_shared_dependencies_and_rejects_cycle_or_duplicate_identity() {
        let mut images = vec![
            image("/Unity", &["/A", "/B"]),
            image("/A", &["/Shared"]),
            image("/B", &["/Shared"]),
            image("/Shared", &[]),
        ];
        assert_eq!(
            dependency_scope(&images, "/Unity").unwrap(),
            vec![3, 1, 2, 0]
        );
        images[3].required_dependencies.push("/Unity".into());
        assert!(dependency_scope(&images, "/Unity")
            .unwrap_err()
            .contains("cyclic"));
        images[3] = image("/A", &[]);
        assert!(dependency_scope(&images, "/Unity")
            .unwrap_err()
            .contains("duplicated"));
    }
}
