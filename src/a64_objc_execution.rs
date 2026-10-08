/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Guest +initialize ordering and ABI-preserving ordinary method dispatch.
//! Single guest thread only. Classes start uninitialized; decoding class_ro
//! never constitutes initialization. Dynamic method mutation and forwarding
//! must remain rejected by the caller until the registry supports them.
use super::bridge::{GuestCall, ServiceFrame};
use super::objc_metadata::{Class, Invocation, MessagePlan, Registry};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Uninitialized,
    Initializing,
    Initialized,
    Failed,
}
#[derive(Debug, Clone)]
struct ClassState {
    superclass: u64,
    state: State,
}
#[derive(Debug, Clone)]
pub(super) struct Initialization {
    classes: BTreeMap<u64, ClassState>,
}
pub(super) struct InitializationCall {
    class: u64,
    invocation: Invocation,
}

impl Initialization {
    /// Validate a publication snapshot without changing any live lifecycle
    /// state. A currently executing +initialize prevents namespace replacement.
    pub(super) fn extended_with(
        &self,
        classes: impl IntoIterator<Item = Class>,
    ) -> Result<Self, String> {
        if self
            .classes
            .values()
            .any(|s| s.state == State::Initializing)
        {
            return Err("cannot publish Objective-C classes during +initialize execution".into());
        }
        let mut extended = self.clone();
        for class in classes {
            if class.flags & 1 != 0 {
                continue;
            }
            if let Some(previous) = extended.classes.get(&class.address) {
                if previous.superclass != class.superclass {
                    return Err("publication changes live initialization superclass".into());
                }
                continue;
            }
            if extended.classes.len() >= 4096 || class.address == 0 || class.address & 7 != 0 {
                return Err("invalid/excessive publication initialization class".into());
            }
            extended.classes.insert(
                class.address,
                ClassState {
                    superclass: class.superclass,
                    state: State::Uninitialized,
                },
            );
        }
        for &class in extended.classes.keys() {
            extended.chain(class)?;
        }
        Ok(extended)
    }
    /// Classes must come from the already validated Registry, including its
    /// complete superclass graph. Metaclasses are not separately initialized.
    pub(super) fn new(classes: impl IntoIterator<Item = Class>) -> Result<Self, String> {
        let mut states = BTreeMap::new();
        for class in classes {
            if class.flags & 1 != 0 {
                continue;
            }
            if states.len() >= 4096
                || class.address == 0
                || class.address & 7 != 0
                || states
                    .insert(
                        class.address,
                        ClassState {
                            superclass: class.superclass,
                            state: State::Uninitialized,
                        },
                    )
                    .is_some()
            {
                return Err("invalid or excessive Objective-C initialization class graph".into());
            }
        }
        let result = Self { classes: states };
        for &class in result.classes.keys() {
            result.chain(class)?;
        }
        Ok(result)
    }

    fn chain(&self, class: u64) -> Result<Vec<u64>, String> {
        let mut current = class;
        let mut visited = BTreeSet::new();
        let mut chain = Vec::new();
        while current != 0 {
            if !visited.insert(current) {
                return Err("cyclic Objective-C initialization superclass chain".into());
            }
            let state = self
                .classes
                .get(&current)
                .ok_or("unregistered Objective-C initialization class")?;
            chain.push(current);
            current = state.superclass;
        }
        chain.reverse();
        Ok(chain)
    }

    pub(super) fn is_initialized(&self, class: u64) -> bool {
        self.classes
            .get(&class)
            .is_some_and(|c| c.state == State::Initialized)
    }

    /// Reentry on this sole guest thread does not invoke +initialize twice.
    /// Queued but not started classes stay Uninitialized, permitting legitimate
    /// child initialization during a superclass's reentrant message send.
    fn start(&mut self, class: u64) -> Result<bool, String> {
        let state = self
            .classes
            .get_mut(&class)
            .ok_or("unregistered initialization start")?;
        match state.state {
            State::Uninitialized => {
                state.state = State::Initializing;
                Ok(true)
            }
            State::Initializing | State::Initialized => Ok(false),
            State::Failed => Err("guest Objective-C class initialization previously failed".into()),
        }
    }

    fn finish(&mut self, class: u64, result: Result<(), String>) -> Result<(), String> {
        let state = self
            .classes
            .get_mut(&class)
            .ok_or("unregistered initialization completion")?;
        if state.state != State::Initializing {
            return Err("invalid guest initialization completion".into());
        }
        match result {
            Ok(()) => {
                state.state = State::Initialized;
                Ok(())
            }
            Err(error) => {
                state.state = State::Failed;
                Err(format!("guest +initialize failed: {error}"))
            }
        }
    }

    /// Resolve all inherited +initialize IMPs before executing any. Apple sends
    /// inherited implementations to the particular class being initialized.
    pub(super) fn plan(
        &self,
        class: u64,
        registry: &Registry,
        initialize_selector: u64,
        mut read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
        mut executable: impl FnMut(u64, usize) -> Result<(), String>,
    ) -> Result<Vec<InitializationCall>, String> {
        let mut result = Vec::new();
        for class in self.chain(class)? {
            match self.classes[&class].state {
                State::Initialized | State::Initializing => continue,
                State::Failed => {
                    return Err("guest Objective-C class initialization previously failed".into())
                }
                State::Uninitialized => {}
            }
            if result.len() >= 8 {
                return Err(
                    "pending guest initialization exceeds bridge continuation limit".into(),
                );
            }
            let MessagePlan::Invoke(invocation) =
                registry.plan_message(class, initialize_selector, &mut read, &mut executable)?
            else {
                return Err("class +initialize resolved to nil".into());
            };
            if invocation.receiver != class
                || invocation.receiver_class != class
                || !matches!(invocation.types.as_str(), "v16@0:8" | "v@:")
            {
                return Err("unsupported guest +initialize ABI".into());
            }
            result.push(InitializationCall { class, invocation });
        }
        Ok(result)
    }
}

pub(super) fn queue_initialization(
    frame: &mut ServiceFrame<'_>,
    state: Rc<RefCell<Initialization>>,
    calls: Vec<InitializationCall>,
) -> Result<(), String> {
    for call in calls {
        let class = call.class;
        let started = Rc::new(Cell::new(false));
        let start_flag = started.clone();
        let start_state = state.clone();
        let finish_state = state.clone();
        frame.request_guest_call_with_start(
            GuestCall {
                entry: call.invocation.implementation,
                integers: vec![class, call.invocation.selector],
                ..GuestCall::default()
            },
            move || {
                let execute = start_state
                    .try_borrow_mut()
                    .map_err(|_| "reentrant initialization policy access")?
                    .start(class)?;
                start_flag.set(execute);
                Ok(execute)
            },
            move |result| {
                if started.get() {
                    finish_state
                        .try_borrow_mut()
                        .map_err(|_| "reentrant initialization completion access")?
                        .finish(class, result.map(|_| ()))
                } else {
                    result.map(|_| ())
                }
            },
        )?;
    }
    Ok(())
}

/// Dispatch a nonnil already validated IMP after genuine initialization calls.
/// Tail dispatch preserves x8, every unknown argument and the original stack.
/// Nil return handling belongs to the existing exact nil-ABI bridge service.
pub(super) fn dispatch(
    frame: &mut ServiceFrame<'_>,
    state: Rc<RefCell<Initialization>>,
    registry: &Registry,
    invocation: &Invocation,
    initialize_selector: u64,
    read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
    executable: impl FnMut(u64, usize) -> Result<(), String>,
) -> Result<(), String> {
    let calls = state
        .try_borrow()
        .map_err(|_| "reentrant initialization planning")?
        .plan(
            invocation.receiver_class,
            registry,
            initialize_selector,
            read,
            executable,
        )?;
    queue_initialization(frame, state, calls)?;
    frame.request_tail_dispatch(invocation.implementation, invocation.selector)
}

/// Frame-backed readers must plan before borrowing the frame for enqueueing.
pub(super) fn dispatch_prepared(
    frame: &mut ServiceFrame<'_>,
    state: Rc<RefCell<Initialization>>,
    invocation: &Invocation,
    calls: Vec<InitializationCall>,
) -> Result<(), String> {
    queue_initialization(frame, state, calls)?;
    frame.request_tail_dispatch(invocation.implementation, invocation.selector)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn class(address: u64, superclass: u64) -> Class {
        Class {
            address,
            isa: address + 0x100,
            superclass,
            name: format!("Class{address:x}"),
            flags: if superclass == 0 { 2 } else { 0 },
            instance_start: 8,
            instance_size: 8,
            methods: vec![],
        }
    }
    #[test]
    fn publication_preserves_completed_states_and_rejects_executing_initialization() {
        let mut state = Initialization::new([class(0x1000, 0)]).unwrap();
        state.start(0x1000).unwrap();
        assert!(state.extended_with([class(0x2000, 0x1000)]).is_err());
        state.finish(0x1000, Ok(())).unwrap();
        let extended = state.extended_with([class(0x2000, 0x1000)]).unwrap();
        assert!(extended.is_initialized(0x1000));
        assert!(!extended.is_initialized(0x2000));
        assert_eq!(extended.chain(0x2000).unwrap(), vec![0x1000, 0x2000]);
        assert!(!state.classes.contains_key(&0x2000));
        assert!(state.extended_with([class(0x2000, 0x3000)]).is_err());
    }
    #[test]
    fn initialization_reentry_and_child_early_initialization_do_not_double_execute() {
        let mut state = Initialization::new([class(0x1000, 0), class(0x2000, 0x1000)]).unwrap();
        assert_eq!(state.chain(0x2000).unwrap(), vec![0x1000, 0x2000]);
        assert!(!state.is_initialized(0x2000));
        assert!(state.start(0x1000).unwrap());
        assert!(!state.start(0x1000).unwrap());
        // Superclass +initialize sends child a message on this same thread.
        assert!(state.start(0x2000).unwrap());
        state.finish(0x2000, Ok(())).unwrap();
        state.finish(0x1000, Ok(())).unwrap();
        assert!(!state.start(0x2000).unwrap());
        assert!(state.is_initialized(0x2000));
    }
    #[test]
    fn failed_initialization_never_becomes_initialized_and_unknown_graph_rejects() {
        let mut state = Initialization::new([class(0x1000, 0)]).unwrap();
        state.start(0x1000).unwrap();
        assert!(state.finish(0x1000, Err("guest trap".into())).is_err());
        assert!(!state.is_initialized(0x1000));
        assert!(state.start(0x1000).is_err());
        assert!(Initialization::new([class(0x2000, 0x1000)]).is_err());
        assert!(Initialization::new([class(0x1000, 0x2000), class(0x2000, 0x1000)]).is_err());
    }
}
