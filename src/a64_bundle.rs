/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Real bundle metadata and explicit loading receipts. BundleId is a host
//! identity, never an Objective-C pointer or a cached class realization claim.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};
const MAX_BUNDLES: usize = 64;
const MAX_PLIST: usize = 1024 * 1024;
const MAX_INITIALIZERS: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct BundleId(u32);
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Metadata {
    pub path: String,
    pub identifier: Option<String>,
    pub executable: String,
    pub principal_class: Option<String>,
    pub package_type: String,
}
impl Metadata {
    pub(super) fn executable_path(&self) -> String {
        format!("{}/{}", self.path, self.executable)
    }
    pub(super) fn parse(path: &str, bytes: &[u8]) -> Result<Self, String> {
        let path = canonical_path(path)?;
        if bytes.is_empty() || bytes.len() > MAX_PLIST {
            return Err("bundle Info.plist exceeds bounded input size".into());
        }
        let value = plist::Value::from_reader(Cursor::new(bytes))
            .map_err(|e| format!("bundle Info.plist: {e}"))?;
        let dictionary = value
            .as_dictionary()
            .ok_or("bundle Info.plist root is not a dictionary")?;
        if dictionary.len() > 1024 {
            return Err("bundle Info.plist key limit exceeded".into());
        }
        let field = |key: &str| -> Result<Option<String>, String> {
            let Some(value) = dictionary.get(key) else {
                return Ok(None);
            };
            let string = value
                .as_string()
                .ok_or_else(|| format!("bundle {key} is not a string"))?;
            if string.is_empty() || string.len() > 4096 || string.chars().any(|c| c.is_control()) {
                return Err(format!("bundle {key} string invalid"));
            }
            Ok(Some(string.into()))
        };
        let executable =
            field("CFBundleExecutable")?.ok_or("bundle executable metadata missing")?;
        if executable.contains('/') || executable == "." || executable == ".." {
            return Err("bundle executable must be one filename".into());
        }
        let package_type =
            field("CFBundlePackageType")?.ok_or("bundle package type metadata missing")?;
        if package_type != "APPL" && package_type != "FMWK" {
            return Err("unsupported executable bundle package type".into());
        }
        Ok(Self {
            path,
            identifier: field("CFBundleIdentifier")?,
            executable,
            principal_class: field("NSPrincipalClass")?,
            package_type,
        })
    }
}
fn canonical_path(path: &str) -> Result<String, String> {
    if !path.starts_with('/') || path.len() > 4096 || path.chars().any(|c| c.is_control()) {
        return Err("bundle path must be a bounded absolute guest path".into());
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => (),
            ".." => return Err("bundle path traversal rejected".into()),
            _ => components.push(component),
        }
    }
    if components.is_empty() {
        return Err("guest root is not a bundle".into());
    }
    Ok(format!("/{}", components.join("/")))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadTicket {
    bundle: BundleId,
    generation: u64,
}
/// Distinct Objective-C +load ABI, never a void() image initializer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadMethod {
    pub receiver: u64,
    pub selector: u64,
    pub imp: u64,
}
pub(super) struct LoadPlan {
    pub ticket: LoadTicket,
    pub executable_path: String,
    pub initializers: Vec<u64>,
    pub principal_class: Option<String>,
    pub required_dependencies: Vec<String>,
    pub load_methods: Vec<LoadMethod>,
}
enum State {
    Discovered,
    Mapped(Vec<u64>),
    Loading {
        generation: u64,
        expected: Vec<u64>,
        completed: usize,
    },
    Loaded {
        principal_class: Option<u64>,
    },
    Failed(String),
}
struct Record {
    metadata: Metadata,
    state: State,
    required_dependencies: BTreeSet<String>,
    completed_dependencies: BTreeSet<String>,
    load_methods: Vec<LoadMethod>,
    completed_load_methods: usize,
}
pub(super) struct Bundles {
    records: Vec<Record>,
    paths: BTreeMap<String, BundleId>,
    main: BundleId,
    next_generation: u64,
}
impl Bundles {
    pub(super) fn from_main(executable_path: &str, plist: &[u8]) -> Result<Self, String> {
        let executable_path = canonical_path(executable_path)?;
        let (path, _) = executable_path
            .rsplit_once('/')
            .ok_or("main executable path invalid")?;
        let metadata = Metadata::parse(path, plist)?;
        if metadata.package_type != "APPL" || metadata.executable_path() != executable_path {
            return Err("main executable disagrees with actual bundle Info.plist".into());
        }
        let main = BundleId(0);
        Ok(Self {
            paths: BTreeMap::from([(metadata.path.clone(), main)]),
            records: vec![Record {
                metadata,
                state: State::Discovered,
                required_dependencies: BTreeSet::new(),
                completed_dependencies: BTreeSet::new(),
                load_methods: Vec::new(),
                completed_load_methods: 0,
            }],
            main,
            next_generation: 1,
        })
    }
    pub(super) fn main_bundle(&self) -> BundleId {
        self.main
    }
    fn record(&self, id: BundleId) -> Result<&Record, String> {
        self.records
            .get(id.0 as usize)
            .ok_or_else(|| "unknown bundle identity".into())
    }
    fn record_mut(&mut self, id: BundleId) -> Result<&mut Record, String> {
        self.records
            .get_mut(id.0 as usize)
            .ok_or_else(|| "unknown bundle identity".into())
    }
    pub(super) fn metadata(&self, id: BundleId) -> Result<&Metadata, String> {
        Ok(&self.record(id)?.metadata)
    }
    pub(super) fn bundle_with_path(
        &mut self,
        path: &str,
        mut reader: impl FnMut(&str) -> Result<Option<Vec<u8>>, String>,
    ) -> Result<Option<BundleId>, String> {
        let path = canonical_path(path)?;
        if let Some(&id) = self.paths.get(&path) {
            return Ok(Some(id));
        }
        if self.records.len() >= MAX_BUNDLES {
            return Err("bundle identity limit exceeded".into());
        }
        let Some(bytes) = reader(&format!("{path}/Info.plist"))? else {
            return Ok(None);
        };
        let metadata = Metadata::parse(&path, &bytes)?;
        let id = BundleId(self.records.len() as u32);
        self.records.push(Record {
            metadata,
            state: State::Discovered,
            required_dependencies: BTreeSet::new(),
            completed_dependencies: BTreeSet::new(),
            load_methods: Vec::new(),
            completed_load_methods: 0,
        });
        self.paths.insert(path, id);
        Ok(Some(id))
    }
    /// Only a loader with the actual mapped image identity and an executable
    /// validator supplies this dependency-ordered initializer list.
    pub(super) fn mark_mapped(
        &mut self,
        id: BundleId,
        executable_path: &str,
        initializers: &[u64],
        load_methods: &[LoadMethod],
        required_dependencies: &[String],
        mut validate: impl FnMut(u64) -> Result<(), String>,
    ) -> Result<(), String> {
        if initializers.len() > MAX_INITIALIZERS {
            return Err("bundle initializer limit exceeded".into());
        }
        let record = self.record(id)?;
        if !matches!(record.state, State::Discovered)
            || record.metadata.executable_path() != canonical_path(executable_path)?
        {
            return Err("bundle mapped identity/state mismatch".into());
        }
        let mut unique = BTreeSet::new();
        for &address in initializers {
            if address == 0 || address & 3 != 0 || !unique.insert(address) {
                return Err("bundle initializer identity invalid or duplicated".into());
            }
            validate(address)?;
        }
        if required_dependencies.len() > 256 {
            return Err("bundle runtime dependency limit exceeded".into());
        }
        let dependencies = required_dependencies
            .iter()
            .map(|path| canonical_path(path))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if dependencies.len() != required_dependencies.len() {
            return Err("bundle runtime dependency duplicated".into());
        }
        if load_methods.len() > MAX_INITIALIZERS {
            return Err("bundle +load method limit exceeded".into());
        }
        let mut unique_load = BTreeSet::new();
        for &method in load_methods {
            if method.receiver == 0
                || method.receiver & 7 != 0
                || method.selector == 0
                || method.imp == 0
                || method.imp & 3 != 0
                || !unique_load.insert((method.receiver, method.selector, method.imp))
            {
                return Err("bundle +load method identity invalid or duplicated".into());
            }
            validate(method.imp)?;
        }
        self.record_mut(id)?.load_methods = load_methods.into();
        self.record_mut(id)?.required_dependencies = dependencies;
        self.record_mut(id)?.state = State::Mapped(initializers.into());
        Ok(())
    }
    pub(super) fn begin_load(&mut self, id: BundleId) -> Result<LoadPlan, String> {
        let record = self.record(id)?;
        let State::Mapped(expected) = &record.state else {
            return Err(
                "bundle load requires a mapped image and cannot retry partial initialization"
                    .into(),
            );
        };
        let expected = expected.clone();
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or("bundle load generation exhausted")?;
        let ticket = LoadTicket {
            bundle: id,
            generation,
        };
        let record = self.record_mut(id)?;
        let plan = LoadPlan {
            ticket,
            executable_path: record.metadata.executable_path(),
            initializers: expected.clone(),
            principal_class: record.metadata.principal_class.clone(),
            required_dependencies: record.required_dependencies.iter().cloned().collect(),
            load_methods: record.load_methods.clone(),
        };
        record.state = State::Loading {
            generation,
            expected,
            completed: 0,
        };
        Ok(plan)
    }
    /// Called only after the real guest initializer returned successfully.
    /// A mapped address alone is never a completion receipt.
    pub(super) fn initializer_completed(
        &mut self,
        ticket: LoadTicket,
        address: u64,
    ) -> Result<(), String> {
        match &mut self.record_mut(ticket.bundle)?.state {
            State::Loading {
                generation,
                expected,
                completed,
            } if *generation == ticket.generation && expected.get(*completed) == Some(&address) => {
                *completed += 1;
                Ok(())
            }
            _ => Err("bundle initializer completion is stale or out of execution order".into()),
        }
    }
    /// Only the startup scheduler records this after actual dependency runtime
    /// initialization completed. File mapping or export resolution is not it.
    pub(super) fn dependency_initialized(
        &mut self,
        ticket: LoadTicket,
        path: &str,
    ) -> Result<(), String> {
        let path = canonical_path(path)?;
        let record = self.record_mut(ticket.bundle)?;
        match record.state {
            State::Loading { generation, .. } if generation == ticket.generation => (),
            _ => return Err("bundle dependency completion ticket stale".into()),
        }
        if !record.required_dependencies.contains(&path)
            || !record.completed_dependencies.insert(path)
        {
            return Err("bundle dependency completion unknown or duplicated".into());
        }
        Ok(())
    }
    /// Recorded only after calling the real IMP with its receiver and SEL.
    pub(super) fn load_method_completed(
        &mut self,
        ticket: LoadTicket,
        method: LoadMethod,
    ) -> Result<(), String> {
        let record = self.record_mut(ticket.bundle)?;
        match record.state {
            State::Loading { generation, .. } if generation == ticket.generation => (),
            _ => return Err("bundle +load completion ticket stale".into()),
        }
        if record.load_methods.get(record.completed_load_methods) != Some(&method) {
            return Err("bundle +load completion out of order or wrong ABI identity".into());
        }
        record.completed_load_methods += 1;
        Ok(())
    }
    /// The scheduler must validate registered real class identity and its
    /// implemented startup policy; metadata text alone does not create a class.
    pub(super) fn finish_load(
        &mut self,
        ticket: LoadTicket,
        principal_class: Option<u64>,
        mut validate_class: impl FnMut(&str, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        let record = self.record(ticket.bundle)?;
        if record.completed_load_methods != record.load_methods.len() {
            return Err("bundle load lacks Objective-C +load execution receipts".into());
        }
        if record.required_dependencies != record.completed_dependencies {
            return Err("bundle load lacks dependency runtime initialization receipts".into());
        }
        match &record.state {
            State::Loading {
                generation,
                expected,
                completed,
            } if *generation == ticket.generation && *completed == expected.len() => (),
            _ => return Err("bundle load lacks completed initializer receipts".into()),
        }
        match (record.metadata.principal_class.as_deref(), principal_class) {
            (Some(name), Some(address)) if address != 0 && address & 7 == 0 => {
                validate_class(name, address)?
            }
            (None, None) => (),
            _ => return Err("bundle principal class has not been genuinely registered".into()),
        }
        self.record_mut(ticket.bundle)?.state = State::Loaded { principal_class };
        Ok(())
    }
    pub(super) fn fail_load(&mut self, ticket: LoadTicket, error: &str) -> Result<(), String> {
        if error.is_empty() || error.len() > 4096 {
            return Err("bundle load failure description invalid".into());
        }
        match self.record(ticket.bundle)?.state {
            State::Loading { generation, .. } if generation == ticket.generation => (),
            _ => return Err("bundle load failure ticket stale".into()),
        }
        self.record_mut(ticket.bundle)?.state = State::Failed(error.into());
        Ok(())
    }
    pub(super) fn is_loaded(&self, id: BundleId) -> Result<bool, String> {
        Ok(matches!(self.record(id)?.state, State::Loaded { .. }))
    }
    pub(super) fn principal_class(&self, id: BundleId) -> Result<Option<u64>, String> {
        match &self.record(id)?.state {
            State::Loaded { principal_class } => Ok(*principal_class),
            State::Failed(error) => Err(format!("bundle initialization failed: {error}")),
            _ => Err("principalClass requires genuine bundle loading/registration".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plist(executable: &str, package: &str, principal: Option<&str>) -> Vec<u8> {
        format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>{executable}</string><key>CFBundlePackageType</key><string>{package}</string>{}</dict></plist>",principal.map(|s|format!("<key>NSPrincipalClass</key><string>{s}</string>")).unwrap_or_default()).into_bytes()
    }
    #[test]
    fn exact_metadata_singletons_and_path_validation() {
        let mut bundles =
            Bundles::from_main("/Terraria.app/Terraria", &plist("Terraria", "APPL", None)).unwrap();
        assert_eq!(
            bundles
                .bundle_with_path("/Terraria.app/", |_| panic!())
                .unwrap(),
            Some(bundles.main_bundle())
        );
        let id = bundles
            .bundle_with_path(
                "/Terraria.app/Frameworks/UnityFramework.framework",
                |path| {
                    assert!(path.ends_with("/Info.plist"));
                    Ok(Some(plist(
                        "UnityFramework",
                        "FMWK",
                        Some("UnityFramework"),
                    )))
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            bundles.metadata(id).unwrap().principal_class.as_deref(),
            Some("UnityFramework")
        );
        assert!(!bundles.is_loaded(id).unwrap());
        assert!(bundles
            .bundle_with_path("/Terraria.app/../Other.app", |_| panic!())
            .is_err());
        assert!(
            Bundles::from_main("/Terraria.app/Other", &plist("Terraria", "APPL", None)).is_err()
        );
        assert_eq!(
            bundles
                .bundle_with_path("/Missing.framework", |_| Ok(None))
                .unwrap(),
            None
        );
    }
    #[test]
    fn mapped_does_not_mean_loaded_or_registered() {
        let mut bundles =
            Bundles::from_main("/Test.app/Test", &plist("Test", "APPL", Some("RealClass")))
                .unwrap();
        let id = bundles.main_bundle();
        bundles
            .mark_mapped(
                id,
                "/Test.app/Test",
                &[0x1000, 0x2000],
                &[LoadMethod {
                    receiver: 0x3000,
                    selector: 0x4000,
                    imp: 0x5000,
                }],
                &["/System/Framework".into()],
                |_| Ok(()),
            )
            .unwrap();
        let plan = bundles.begin_load(id).unwrap();
        assert_eq!(plan.initializers, vec![0x1000, 0x2000]);
        assert_eq!(plan.required_dependencies, vec!["/System/Framework"]);
        assert_eq!(plan.executable_path, "/Test.app/Test");
        assert_eq!(plan.principal_class.as_deref(), Some("RealClass"));
        assert!(bundles
            .finish_load(plan.ticket, Some(0x3000), |_, _| Ok(()))
            .is_err());
        assert!(bundles.initializer_completed(plan.ticket, 0x2000).is_err());
        bundles.initializer_completed(plan.ticket, 0x1000).unwrap();
        assert!(!bundles.is_loaded(id).unwrap());
        bundles.initializer_completed(plan.ticket, 0x2000).unwrap();
        assert!(bundles
            .finish_load(plan.ticket, Some(0x3000), |_, _| Ok(()))
            .unwrap_err()
            .contains("+load"));
        assert!(bundles
            .load_method_completed(
                plan.ticket,
                LoadMethod {
                    receiver: 0x3010,
                    selector: 0x4000,
                    imp: 0x5000
                }
            )
            .is_err());
        bundles
            .load_method_completed(plan.ticket, plan.load_methods[0])
            .unwrap();
        assert!(bundles
            .finish_load(plan.ticket, Some(0x3000), |_, _| Ok(()))
            .unwrap_err()
            .contains("dependency"));
        bundles
            .dependency_initialized(plan.ticket, "/System/Framework")
            .unwrap();
        assert!(bundles
            .dependency_initialized(plan.ticket, "/System/Framework")
            .is_err());
        assert!(bundles
            .finish_load(plan.ticket, Some(0x3000), |_, _| Err(
                "cached class uninitialized".into()
            ))
            .is_err());
        assert!(!bundles.is_loaded(id).unwrap());
        bundles
            .finish_load(plan.ticket, Some(0x3000), |name, address| {
                assert_eq!(name, "RealClass");
                assert_eq!(address, 0x3000);
                Ok(())
            })
            .unwrap();
        assert!(bundles.is_loaded(id).unwrap());
        assert_eq!(bundles.principal_class(id).unwrap(), Some(0x3000));
        assert!(bundles.initializer_completed(plan.ticket, 0x2000).is_err());
    }
    #[test]
    fn failed_guest_initialization_is_not_silently_retried() {
        let mut bundles =
            Bundles::from_main("/Test.app/Test", &plist("Test", "APPL", None)).unwrap();
        let id = bundles.main_bundle();
        bundles
            .mark_mapped(id, "/Test.app/Test", &[], &[], &[], |_| Ok(()))
            .unwrap();
        let plan = bundles.begin_load(id).unwrap();
        bundles
            .fail_load(plan.ticket, "guest initializer faulted")
            .unwrap();
        assert!(!bundles.is_loaded(id).unwrap());
        assert!(bundles.begin_load(id).is_err());
        assert!(bundles.principal_class(id).unwrap_err().contains("faulted"));
        assert!(bundles
            .finish_load(plan.ticket, None, |_, _| Ok(()))
            .is_err());
    }
}
