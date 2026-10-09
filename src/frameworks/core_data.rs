/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! The Core Data framework: a small in-memory implementation.
//!
//! Supported: models built in code (`NSEntityDescription`,
//! `NSAttributeDescription`), a coordinator with in-memory stores (every store
//! type is kept in memory, so nothing is saved across launches), contexts that
//! insert/delete/fetch managed objects, key-value access to attributes, fetch
//! requests with simple predicates (`key OP value` joined by `AND`) and sort
//! descriptors. Not supported: models loaded from `.momd`, relationships,
//! faulting, `NSFetchedResultsController`, undo.
//!
//! Apps use this mostly for small caches (e.g. the analytics SDKs' event
//! queues); some abort when the persistent store cannot be created.

use crate::abi::{CallFromHost, GuestFunction};
use crate::dyld::HostDylib;
use crate::frameworks::foundation::{ns_array, ns_dictionary, ns_string, NSUInteger};
use crate::log;
use crate::mem::{MutPtr, Ptr};
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, retain, ClassExports, HostObject,
    NSZonePtr,
};
use crate::Environment;
use std::cmp::Ordering;

pub const DYLIB: HostDylib = HostDylib {
    path: "/System/Library/Frameworks/CoreData.framework/CoreData",
    aliases: &[],
    class_exports: &[CLASSES],
    constant_exports: &[],
    function_exports: &[],
};

#[derive(Default)]
struct DescriptionHostObject {
    /// `NSString*`
    name: id,
    /// `NSString*`, entities only
    class_name: id,
    /// `NSArray*` of descriptions, entities only
    properties: id,
    attribute_type: NSUInteger,
    optional: bool,
}
impl HostObject for DescriptionHostObject {}

#[derive(Default)]
struct ModelHostObject {
    /// `NSArray*` of `NSEntityDescription*`
    entities: id,
}
impl HostObject for ModelHostObject {}

#[derive(Default)]
struct StoreHostObject {
    store_type: id,
    url: id,
}
impl HostObject for StoreHostObject {}

#[derive(Default)]
struct CoordinatorHostObject {
    model: id,
    stores: Vec<id>,
}
impl HostObject for CoordinatorHostObject {}

#[derive(Default)]
struct ContextHostObject {
    coordinator: id,
    /// Registered objects (retained), including inserted ones.
    objects: Vec<id>,
    has_changes: bool,
}
impl HostObject for ContextHostObject {}

#[derive(Default)]
struct ManagedObjectHostObject {
    entity: id,
    context: id,
    values: Vec<(String, id)>,
    deleted: bool,
}
impl HostObject for ManagedObjectHostObject {}

#[derive(Default)]
struct FetchRequestHostObject {
    entity: id,
    predicate: id,
    sort_descriptors: id,
    limit: NSUInteger,
}
impl HostObject for FetchRequestHostObject {}

#[derive(Default)]
struct FetchedResultsControllerHostObject {
    request: id,
    context: id,
    results: id,
    delegate: id,
}
impl HostObject for FetchedResultsControllerHostObject {}

#[derive(Default)]
struct FetchedSectionHostObject {
    objects: id,
}
impl HostObject for FetchedSectionHostObject {}

#[derive(Default)]
struct PredicateHostObject {
    format: String,
}
impl HostObject for PredicateHostObject {}

#[derive(Default)]
struct SortDescriptorHostObject {
    key: String,
    ascending: bool,
}
impl HostObject for SortDescriptorHostObject {}

fn set_id(slot: &mut id, new: id) -> id {
    std::mem::replace(slot, new)
}

fn array_ids(env: &mut Environment, array: id) -> Vec<id> {
    if array == nil {
        return Vec::new();
    }
    let count: NSUInteger = msg![env; array count];
    (0..count)
        .map(|index| msg![env; array objectAtIndex:index])
        .collect()
}

fn set_out_error(env: &mut Environment, error: MutPtr<id>) {
    if !error.is_null() {
        env.mem.write(error, nil);
    }
}

fn string_of(env: &mut Environment, object: id) -> String {
    if object == nil {
        return String::new();
    }
    let description: id = msg![env; object description];
    ns_string::to_rust_string(env, description).to_string()
}

fn compare_values(a: &str, b: &str) -> Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        _ => a.cmp(b),
    }
}

fn find_value(env: &Environment, object: id, key: &str) -> id {
    env.objc
        .borrow::<ManagedObjectHostObject>(object)
        .values
        .iter()
        .find(|(name, _)| name == key)
        .map_or(nil, |&(_, value)| value)
}

fn set_value(env: &mut Environment, object: id, key: &str, value: id) {
    retain(env, value);
    let host_object = env.objc.borrow_mut::<ManagedObjectHostObject>(object);
    let old = match host_object.values.iter_mut().find(|(name, _)| name == key) {
        Some((_, slot)) => std::mem::replace(slot, value),
        None => {
            host_object.values.push((key.to_string(), value));
            nil
        }
    };
    release(env, old);
    let context = env.objc.borrow::<ManagedObjectHostObject>(object).context;
    if context != nil {
        env.objc.borrow_mut::<ContextHostObject>(context).has_changes = true;
    }
}

/// Evaluates a (simple) predicate format: comparisons joined by `AND`.
fn predicate_matches(env: &mut Environment, format: &str, object: id) -> bool {
    if format.trim().is_empty() || format.trim().eq_ignore_ascii_case("TRUEPREDICATE") {
        return true;
    }
    let normalized = format.replace(" and ", " AND ").replace("&&", " AND ");
    for clause in normalized.split(" AND ") {
        let clause = clause.trim().trim_start_matches('(').trim_end_matches(')');
        let mut matched = false;
        for op in ["==", "!=", ">=", "<=", "=", ">", "<"] {
            let Some(index) = clause.find(op) else {
                continue;
            };
            let key = clause[..index].trim();
            let rhs = clause[index + op.len()..]
                .trim()
                .trim_matches(|c| c == '"' || c == '\'');
            let value = find_value(env, object, key);
            let lhs = string_of(env, value);
            let ordering = compare_values(&lhs, rhs);
            matched = match op {
                "==" | "=" => ordering == Ordering::Equal,
                "!=" => ordering != Ordering::Equal,
                ">" => ordering == Ordering::Greater,
                "<" => ordering == Ordering::Less,
                ">=" => ordering != Ordering::Less,
                _ => ordering != Ordering::Greater,
            };
            break;
        }
        if !matched {
            return false;
        }
    }
    true
}

/// The entity's name, for matching a fetch request's entity to an object's.
fn entity_name(env: &mut Environment, entity: id) -> String {
    if entity == nil {
        return String::new();
    }
    let name = env.objc.borrow::<DescriptionHostObject>(entity).name;
    if name == nil {
        String::new()
    } else {
        ns_string::to_rust_string(env, name).to_string()
    }
}

fn model_entity_named(env: &mut Environment, model: id, name: &str) -> id {
    let entities = env.objc.borrow::<ModelHostObject>(model).entities;
    for entity in array_ids(env, entities) {
        if entity_name(env, entity) == name {
            return entity;
        }
    }
    nil
}

fn context_model(env: &Environment, context: id) -> id {
    let coordinator = env.objc.borrow::<ContextHostObject>(context).coordinator;
    if coordinator == nil {
        nil
    } else {
        env.objc.borrow::<CoordinatorHostObject>(coordinator).model
    }
}

fn insert_object(env: &mut Environment, entity: id, context: id) -> id {
    let class_name = env.objc.borrow::<DescriptionHostObject>(entity).class_name;
    let mut class = nil;
    if class_name != nil {
        let class_name = ns_string::to_rust_string(env, class_name);
        class = env
            .objc
            .try_get_known_class(&class_name, &mut env.mem)
            .unwrap_or(nil);
    }
    if class == nil {
        class = env.objc.get_known_class("NSManagedObject", &mut env.mem);
    }
    let object: id = msg![env; class alloc];
    retain(env, entity);
    {
        let host_object = env.objc.borrow_mut::<ManagedObjectHostObject>(object);
        host_object.entity = entity;
        // The context owns its objects, not the other way round.
        host_object.context = context;
    }
    let context_host = env.objc.borrow_mut::<ContextHostObject>(context);
    context_host.objects.push(object);
    context_host.has_changes = true;
    object
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSPropertyDescription: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<DescriptionHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)name {
    env.objc.borrow::<DescriptionHostObject>(this).name
}
- (())setName:(id)name { // NSString*
    retain(env, name);
    let old = set_id(&mut env.objc.borrow_mut::<DescriptionHostObject>(this).name, name);
    release(env, old);
}
- (bool)isOptional {
    env.objc.borrow::<DescriptionHostObject>(this).optional
}
- (())setOptional:(bool)optional {
    env.objc.borrow_mut::<DescriptionHostObject>(this).optional = optional;
}
- (())setTransient:(bool)_transient {}
- (())setIndexed:(bool)_indexed {}

- (())dealloc {
    let &DescriptionHostObject { name, class_name, properties, .. } = env.objc.borrow(this);
    release(env, name);
    release(env, class_name);
    release(env, properties);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSAttributeDescription: NSPropertyDescription

- (NSUInteger)attributeType {
    env.objc.borrow::<DescriptionHostObject>(this).attribute_type
}
- (())setAttributeType:(NSUInteger)attribute_type {
    env.objc.borrow_mut::<DescriptionHostObject>(this).attribute_type = attribute_type;
}
- (())setDefaultValue:(id)_value {}

@end

@implementation NSEntityDescription: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<DescriptionHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)entityForName:(id)name // NSString*
        inManagedObjectContext:(id)context { // NSManagedObjectContext*
    if name == nil || context == nil {
        return nil;
    }
    let model = context_model(env, context);
    if model == nil {
        return nil;
    }
    let name = ns_string::to_rust_string(env, name).to_string();
    model_entity_named(env, model, &name)
}

+ (id)insertNewObjectForEntityForName:(id)name // NSString*
               inManagedObjectContext:(id)context { // NSManagedObjectContext*
    let entity: id = msg![env; this entityForName:name inManagedObjectContext:context];
    if entity == nil {
        log!("Warning: Core Data: no entity named {:?} in the model", name);
        return nil;
    }
    insert_object(env, entity, context)
}

- (id)name {
    env.objc.borrow::<DescriptionHostObject>(this).name
}
- (())setName:(id)name { // NSString*
    retain(env, name);
    let old = set_id(&mut env.objc.borrow_mut::<DescriptionHostObject>(this).name, name);
    release(env, old);
}
- (id)managedObjectClassName {
    env.objc.borrow::<DescriptionHostObject>(this).class_name
}
- (())setManagedObjectClassName:(id)name { // NSString*
    retain(env, name);
    let old = set_id(&mut env.objc.borrow_mut::<DescriptionHostObject>(this).class_name, name);
    release(env, old);
}
- (id)properties {
    let properties = env.objc.borrow::<DescriptionHostObject>(this).properties;
    if properties == nil {
        msg_class![env; NSArray array]
    } else {
        properties
    }
}
- (())setProperties:(id)properties { // NSArray*
    let copy: id = if properties == nil { nil } else { msg![env; properties copy] };
    let old = set_id(&mut env.objc.borrow_mut::<DescriptionHostObject>(this).properties, copy);
    release(env, old);
}
- (id)attributesByName {
    let properties = env.objc.borrow::<DescriptionHostObject>(this).properties;
    let mut pairs = Vec::new();
    for property in array_ids(env, properties) {
        let name = env.objc.borrow::<DescriptionHostObject>(property).name;
        pairs.push((name, property));
    }
    let dict = ns_dictionary::dict_from_keys_and_objects(env, &pairs);
    autorelease(env, dict)
}
- (id)propertiesByName {
    msg![env; this attributesByName]
}

- (())dealloc {
    let &DescriptionHostObject { name, class_name, properties, .. } = env.objc.borrow(this);
    release(env, name);
    release(env, class_name);
    release(env, properties);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSManagedObjectModel: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<ModelHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)mergedModelFromBundles:(id)_bundles { // NSArray*
    // Models stored in .momd bundles are not supported: an empty model.
    let model: id = msg![env; this new];
    autorelease(env, model)
}

- (id)initWithContentsOfURL:(id)_url { // NSURL*
    log!("Warning: Core Data: loading a model from a file is not supported, using an empty model");
    this
}

- (id)entities {
    let entities = env.objc.borrow::<ModelHostObject>(this).entities;
    if entities == nil {
        msg_class![env; NSArray array]
    } else {
        entities
    }
}
- (())setEntities:(id)entities { // NSArray*
    let copy: id = if entities == nil { nil } else { msg![env; entities copy] };
    let old = set_id(&mut env.objc.borrow_mut::<ModelHostObject>(this).entities, copy);
    release(env, old);
}
- (id)entitiesByName {
    let entities = env.objc.borrow::<ModelHostObject>(this).entities;
    let mut pairs = Vec::new();
    for entity in array_ids(env, entities) {
        let name = env.objc.borrow::<DescriptionHostObject>(entity).name;
        pairs.push((name, entity));
    }
    let dict = ns_dictionary::dict_from_keys_and_objects(env, &pairs);
    autorelease(env, dict)
}

- (())dealloc {
    let entities = env.objc.borrow::<ModelHostObject>(this).entities;
    release(env, entities);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSPersistentStore: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<StoreHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)type {
    env.objc.borrow::<StoreHostObject>(this).store_type
}
- (id)URL {
    env.objc.borrow::<StoreHostObject>(this).url
}

- (())dealloc {
    let &StoreHostObject { store_type, url } = env.objc.borrow(this);
    release(env, store_type);
    release(env, url);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSPersistentStoreCoordinator: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<CoordinatorHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithManagedObjectModel:(id)model { // NSManagedObjectModel*
    retain(env, model);
    env.objc.borrow_mut::<CoordinatorHostObject>(this).model = model;
    this
}

- (id)managedObjectModel {
    env.objc.borrow::<CoordinatorHostObject>(this).model
}

- (id)addPersistentStoreWithType:(id)store_type // NSString*
                   configuration:(id)_configuration // NSString*
                             URL:(id)url // NSURL*
                         options:(id)_options // NSDictionary*
                           error:(MutPtr<id>)error {
    set_out_error(env, error);
    // Every store type is kept in memory.
    let store: id = msg_class![env; NSPersistentStore alloc];
    retain(env, store_type);
    retain(env, url);
    {
        let host_object = env.objc.borrow_mut::<StoreHostObject>(store);
        host_object.store_type = store_type;
        host_object.url = url;
    }
    env.objc.borrow_mut::<CoordinatorHostObject>(this).stores.push(store);
    store
}

- (id)persistentStores {
    let stores = env.objc.borrow::<CoordinatorHostObject>(this).stores.clone();
    for &store in &stores {
        retain(env, store);
    }
    let array = ns_array::from_vec(env, stores);
    autorelease(env, array)
}

- (bool)removePersistentStore:(id)store // NSPersistentStore*
                        error:(MutPtr<id>)error {
    set_out_error(env, error);
    let host_object = env.objc.borrow_mut::<CoordinatorHostObject>(this);
    let before = host_object.stores.len();
    host_object.stores.retain(|&existing| existing != store);
    let removed = host_object.stores.len() != before;
    if removed {
        release(env, store);
    }
    removed
}

- (())dealloc {
    let (model, stores) = {
        let host_object = env.objc.borrow::<CoordinatorHostObject>(this);
        (host_object.model, host_object.stores.clone())
    };
    release(env, model);
    for store in stores {
        release(env, store);
    }
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSManagedObjectContext: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<ContextHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithConcurrencyType:(NSUInteger)_type {
    this
}

- (id)persistentStoreCoordinator {
    env.objc.borrow::<ContextHostObject>(this).coordinator
}
- (())setPersistentStoreCoordinator:(id)coordinator { // NSPersistentStoreCoordinator*
    retain(env, coordinator);
    let old = set_id(&mut env.objc.borrow_mut::<ContextHostObject>(this).coordinator, coordinator);
    release(env, old);
}
- (())setUndoManager:(id)_manager {}
- (())setMergePolicy:(id)_policy {}
- (())setStalenessInterval:(f64)_interval {}

- (bool)hasChanges {
    env.objc.borrow::<ContextHostObject>(this).has_changes
}

- (id)insertedObjects {
    msg_class![env; NSSet set]
}

- (())deleteObject:(id)object { // NSManagedObject*
    let host_object = env.objc.borrow_mut::<ContextHostObject>(this);
    let before = host_object.objects.len();
    host_object.objects.retain(|&existing| existing != object);
    if host_object.objects.len() != before {
        host_object.has_changes = true;
        env.objc.borrow_mut::<ManagedObjectHostObject>(object).deleted = true;
        release(env, object);
    }
}

- (bool)save:(MutPtr<id>)error {
    set_out_error(env, error);
    env.objc.borrow_mut::<ContextHostObject>(this).has_changes = false;
    true
}

- (())reset {
    let objects = std::mem::take(&mut env.objc.borrow_mut::<ContextHostObject>(this).objects);
    for object in objects {
        env.objc.borrow_mut::<ManagedObjectHostObject>(object).deleted = true;
        release(env, object);
    }
    env.objc.borrow_mut::<ContextHostObject>(this).has_changes = false;
}

- (())performBlock:(id)block {
    run_block(env, block);
}
- (())performBlockAndWait:(id)block {
    run_block(env, block);
}

- (id)executeFetchRequest:(id)request // NSFetchRequest*
                    error:(MutPtr<id>)error {
    set_out_error(env, error);
    let &FetchRequestHostObject { entity, predicate, sort_descriptors, limit } =
        env.objc.borrow(request);
    let wanted = entity_name(env, entity);
    let format = if predicate == nil {
        String::new()
    } else {
        env.objc.borrow::<PredicateHostObject>(predicate).format.clone()
    };

    let candidates = env.objc.borrow::<ContextHostObject>(this).objects.clone();
    let mut results: Vec<id> = Vec::new();
    for object in candidates {
        let object_entity = env.objc.borrow::<ManagedObjectHostObject>(object).entity;
        if entity != nil && entity_name(env, object_entity) != wanted {
            continue;
        }
        if predicate_matches(env, &format, object) {
            results.push(object);
        }
    }

    for descriptor in array_ids(env, sort_descriptors).into_iter().rev() {
        let (key, ascending) = {
            let host_object = env.objc.borrow::<SortDescriptorHostObject>(descriptor);
            (host_object.key.clone(), host_object.ascending)
        };
        let mut keyed: Vec<(String, id)> = results
            .iter()
            .map(|&object| {
                let value = find_value(env, object, &key);
                (string_of(env, value), object)
            })
            .collect();
        // Stable sort, applied from the last descriptor to the first.
        keyed.sort_by(|a, b| {
            let ordering = compare_values(&a.0, &b.0);
            if ascending { ordering } else { ordering.reverse() }
        });
        results = keyed.into_iter().map(|(_, object)| object).collect();
    }

    if limit != 0 {
        results.truncate(limit as usize);
    }
    for &object in &results {
        retain(env, object);
    }
    let array = ns_array::from_vec(env, results);
    autorelease(env, array)
}

- (())dealloc {
    let (coordinator, objects) = {
        let host_object = env.objc.borrow::<ContextHostObject>(this);
        (host_object.coordinator, host_object.objects.clone())
    };
    release(env, coordinator);
    for object in objects {
        release(env, object);
    }
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSManagedObject: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<ManagedObjectHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithEntity:(id)entity // NSEntityDescription*
    insertIntoManagedObjectContext:(id)context { // NSManagedObjectContext*
    retain(env, entity);
    {
        let host_object = env.objc.borrow_mut::<ManagedObjectHostObject>(this);
        host_object.entity = entity;
        host_object.context = context;
    }
    if context != nil {
        retain(env, this);
        let context_host = env.objc.borrow_mut::<ContextHostObject>(context);
        context_host.objects.push(this);
        context_host.has_changes = true;
    }
    this
}

- (id)entity {
    env.objc.borrow::<ManagedObjectHostObject>(this).entity
}
- (id)managedObjectContext {
    env.objc.borrow::<ManagedObjectHostObject>(this).context
}
- (bool)isDeleted {
    env.objc.borrow::<ManagedObjectHostObject>(this).deleted
}
- (bool)isInserted {
    !env.objc.borrow::<ManagedObjectHostObject>(this).deleted
}

- (id)valueForKey:(id)key { // NSString*
    let key = ns_string::to_rust_string(env, key).to_string();
    find_value(env, this, &key)
}
- (id)primitiveValueForKey:(id)key { // NSString*
    let key = ns_string::to_rust_string(env, key).to_string();
    find_value(env, this, &key)
}
- (())setValue:(id)value forKey:(id)key { // NSString*
    let key = ns_string::to_rust_string(env, key).to_string();
    set_value(env, this, &key, value);
}
- (())setPrimitiveValue:(id)value forKey:(id)key { // NSString*
    let key = ns_string::to_rust_string(env, key).to_string();
    set_value(env, this, &key, value);
}

- (())dealloc {
    let (entity, values) = {
        let host_object = env.objc.borrow::<ManagedObjectHostObject>(this);
        (host_object.entity, host_object.values.clone())
    };
    release(env, entity);
    for (_, value) in values {
        release(env, value);
    }
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSFetchRequest: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<FetchRequestHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)fetchRequestWithEntityName:(id)_name { // NSString*
    // Entities are looked up by the context when fetching, by name.
    let request: id = msg![env; this new];
    autorelease(env, request)
}

- (id)entity {
    env.objc.borrow::<FetchRequestHostObject>(this).entity
}
- (())setEntity:(id)entity { // NSEntityDescription*
    retain(env, entity);
    let old = set_id(&mut env.objc.borrow_mut::<FetchRequestHostObject>(this).entity, entity);
    release(env, old);
}
- (id)predicate {
    env.objc.borrow::<FetchRequestHostObject>(this).predicate
}
- (())setPredicate:(id)predicate { // NSPredicate*
    retain(env, predicate);
    let old = set_id(&mut env.objc.borrow_mut::<FetchRequestHostObject>(this).predicate, predicate);
    release(env, old);
}
- (id)sortDescriptors {
    env.objc.borrow::<FetchRequestHostObject>(this).sort_descriptors
}
- (())setSortDescriptors:(id)descriptors { // NSArray*
    retain(env, descriptors);
    let old = set_id(&mut env.objc.borrow_mut::<FetchRequestHostObject>(this).sort_descriptors, descriptors);
    release(env, old);
}
- (NSUInteger)fetchLimit {
    env.objc.borrow::<FetchRequestHostObject>(this).limit
}
- (())setFetchLimit:(NSUInteger)limit {
    env.objc.borrow_mut::<FetchRequestHostObject>(this).limit = limit;
}
- (())setFetchBatchSize:(NSUInteger)_size {}
- (())setFetchOffset:(NSUInteger)_offset {}
- (())setResultType:(NSUInteger)_type {}
- (())setIncludesPropertyValues:(bool)_include {}
- (())setIncludesSubentities:(bool)_include {}
- (())setReturnsObjectsAsFaults:(bool)_faults {}

- (())dealloc {
    let &FetchRequestHostObject { entity, predicate, sort_descriptors, .. } = env.objc.borrow(this);
    release(env, entity);
    release(env, predicate);
    release(env, sort_descriptors);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSFetchedResultsController: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<FetchedResultsControllerHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)initWithFetchRequest:(id)request // NSFetchRequest*
      managedObjectContext:(id)context // NSManagedObjectContext*
        sectionNameKeyPath:(id)_section_name_key_path // NSString*
                 cacheName:(id)_cache_name { // NSString*
    retain(env, request);
    retain(env, context);
    let host_object = env.objc.borrow_mut::<FetchedResultsControllerHostObject>(this);
    host_object.request = request;
    host_object.context = context;
    this
}

- (id)fetchRequest {
    env.objc.borrow::<FetchedResultsControllerHostObject>(this).request
}
- (id)managedObjectContext {
    env.objc.borrow::<FetchedResultsControllerHostObject>(this).context
}
- (id)delegate {
    env.objc.borrow::<FetchedResultsControllerHostObject>(this).delegate
}
- (())setDelegate:(id)delegate {
    // Changes are never made behind the controller's back, so a delegate
    // would never be called back.
    env.objc.borrow_mut::<FetchedResultsControllerHostObject>(this).delegate = delegate;
}

- (bool)performFetch:(MutPtr<id>)error {
    let &FetchedResultsControllerHostObject { request, context, .. } = env.objc.borrow(this);
    let results: id = msg![env; context executeFetchRequest:request error:error];
    retain(env, results);
    let old = set_id(&mut env.objc.borrow_mut::<FetchedResultsControllerHostObject>(this).results, results);
    release(env, old);
    true
}

- (id)sections {
    let results = env.objc.borrow::<FetchedResultsControllerHostObject>(this).results;
    // No section key paths: everything is in a single section.
    let section: id = msg_class![env; _touchHLE_FetchedResultsSection alloc];
    retain(env, results);
    env.objc.borrow_mut::<FetchedSectionHostObject>(section).objects = results;
    let array = ns_array::from_vec(env, vec![section]);
    autorelease(env, array)
}

- (id)fetchedObjects {
    env.objc.borrow::<FetchedResultsControllerHostObject>(this).results
}

- (())dealloc {
    let &FetchedResultsControllerHostObject { request, context, results, .. } = env.objc.borrow(this);
    release(env, request);
    release(env, context);
    release(env, results);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation _touchHLE_FetchedResultsSection: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<FetchedSectionHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (NSUInteger)numberOfObjects {
    let objects = env.objc.borrow::<FetchedSectionHostObject>(this).objects;
    if objects == nil { 0 } else { msg![env; objects count] }
}
- (id)objects {
    env.objc.borrow::<FetchedSectionHostObject>(this).objects
}
- (id)name {
    nil
}
- (id)indexTitle {
    nil
}

- (())dealloc {
    let objects = env.objc.borrow::<FetchedSectionHostObject>(this).objects;
    release(env, objects);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

@implementation NSPredicate: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<PredicateHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)predicateWithFormat:(id)format, // NSString*
                           ...args {
    let format = ns_string::with_format(env, format, args.start());
    let predicate: id = msg![env; this new];
    env.objc.borrow_mut::<PredicateHostObject>(predicate).format = format;
    autorelease(env, predicate)
}

- (id)predicateFormat {
    let format = env.objc.borrow::<PredicateHostObject>(this).format.clone();
    let string = ns_string::from_rust_string(env, format);
    autorelease(env, string)
}

@end

@implementation NSSortDescriptor: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<SortDescriptorHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

+ (id)sortDescriptorWithKey:(id)key // NSString*
                  ascending:(bool)ascending {
    let descriptor: id = msg![env; this alloc];
    let descriptor: id = msg![env; descriptor initWithKey:key ascending:ascending];
    autorelease(env, descriptor)
}

+ (id)sortDescriptorWithKey:(id)key // NSString*
                  ascending:(bool)ascending
                   selector:(crate::objc::SEL)_selector {
    msg![env; this sortDescriptorWithKey:key ascending:ascending]
}

- (id)initWithKey:(id)key // NSString*
        ascending:(bool)ascending {
    let key = ns_string::to_rust_string(env, key).to_string();
    let host_object = env.objc.borrow_mut::<SortDescriptorHostObject>(this);
    host_object.key = key;
    host_object.ascending = ascending;
    this
}

- (id)initWithKey:(id)key // NSString*
        ascending:(bool)ascending
         selector:(crate::objc::SEL)_selector {
    msg![env; this initWithKey:key ascending:ascending]
}

- (bool)ascending {
    env.objc.borrow::<SortDescriptorHostObject>(this).ascending
}

- (id)key {
    let key = env.objc.borrow::<SortDescriptorHostObject>(this).key.clone();
    let string = ns_string::from_rust_string(env, key);
    autorelease(env, string)
}

@end

};

fn run_block(env: &mut Environment, block: id) {
    if block == nil {
        return;
    }
    let invoke: u32 = env.mem.read(Ptr::<u32, false>::from_bits(block.to_bits() + 12));
    let invoke = GuestFunction::from_addr_with_thumb_bit(invoke);
    let () = invoke.call_from_host(env, (block,));
}

/// `@dynamic` properties of `NSManagedObject` subclasses have no methods in
/// the app: Core Data synthesizes the accessors at run time. This is the
/// equivalent, called by the message dispatcher for a selector that the
/// object's class does not implement: `foo` reads and `setFoo:` writes the
/// attribute `foo` (object values only). Returns whether it handled the
/// message (leaving the result in r0).
pub fn try_dynamic_accessor(
    env: &mut Environment,
    receiver: id,
    class: crate::objc::Class,
    selector: crate::objc::SEL,
) -> bool {
    let Some(base) = env.objc.try_get_known_class("NSManagedObject", &mut env.mem) else {
        return false;
    };
    if !env.objc.class_is_subclass_of(class, base) {
        return false;
    }
    let name = selector.as_str(&env.mem).to_string();
    // The message sends below clobber the registers that hold the arguments.
    let saved_regs = *env.cpu.regs();
    let saved_cpsr = env.cpu.cpsr();
    // The attribute's type (NSAttributeType), if the entity has the attribute.
    let attribute_type = |env: &mut Environment, key: &str| -> Option<NSUInteger> {
        let entity = env.objc.borrow::<ManagedObjectHostObject>(receiver).entity;
        if entity == nil {
            return None;
        }
        let properties = env.objc.borrow::<DescriptionHostObject>(entity).properties;
        for property in array_ids(env, properties) {
            let (property_name, attribute_type) = {
                let host_object = env.objc.borrow::<DescriptionHostObject>(property);
                (host_object.name, host_object.attribute_type)
            };
            if property_name != nil && ns_string::to_rust_string(env, property_name) == key {
                return Some(attribute_type);
            }
        }
        None
    };
    // NSAttributeType values that are accessed as C scalars.
    const INTEGER_16: NSUInteger = 100;
    const INTEGER_32: NSUInteger = 200;
    const INTEGER_64: NSUInteger = 300;
    const DOUBLE: NSUInteger = 500;
    const FLOAT: NSUInteger = 600;
    const BOOLEAN: NSUInteger = 800;

    if let Some(rest) = name.strip_prefix("set").and_then(|rest| rest.strip_suffix(':')) {
        let mut chars = rest.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        let key = format!("{}{}", first.to_lowercase(), chars.as_str());
        let Some(kind) = attribute_type(env, &key) else {
            return false;
        };
        let regs = saved_regs;
        let value: id = match kind {
            INTEGER_16 | INTEGER_32 => msg_class![env; NSNumber numberWithInt:(regs[2] as i32)],
            BOOLEAN => msg_class![env; NSNumber numberWithBool:(regs[2] & 0xff != 0)],
            INTEGER_64 => {
                let value = (regs[2] as u64 | ((regs[3] as u64) << 32)) as i64;
                msg_class![env; NSNumber numberWithLongLong:value]
            }
            DOUBLE => {
                let value = f64::from_bits(regs[2] as u64 | ((regs[3] as u64) << 32));
                msg_class![env; NSNumber numberWithDouble:value]
            }
            FLOAT => msg_class![env; NSNumber numberWithFloat:(f32::from_bits(regs[2]))],
            _ => crate::mem::Ptr::from_bits(regs[2]),
        };
        set_value(env, receiver, &key, value);
        *env.cpu.regs_mut() = saved_regs;
        env.cpu.set_cpsr(saved_cpsr);
        return true;
    }
    if !name.contains(':') {
        let Some(kind) = attribute_type(env, &name) else {
            return false;
        };
        let value = find_value(env, receiver, &name);
        let (mut r0, mut r1) = (0u32, 0u32);
        match kind {
            INTEGER_16 | INTEGER_32 | BOOLEAN => {
                let number: i32 = if value == nil { 0 } else { msg![env; value intValue] };
                r0 = number as u32;
            }
            INTEGER_64 => {
                let number: i64 = if value == nil { 0 } else { msg![env; value longLongValue] };
                r0 = number as u32;
                r1 = (number >> 32) as u32;
            }
            DOUBLE => {
                let number: f64 = if value == nil { 0.0 } else { msg![env; value doubleValue] };
                let bits = number.to_bits();
                r0 = bits as u32;
                r1 = (bits >> 32) as u32;
            }
            FLOAT => {
                let number: f32 = if value == nil { 0.0 } else { msg![env; value floatValue] };
                r0 = number.to_bits();
            }
            _ => r0 = value.to_bits(),
        }
        *env.cpu.regs_mut() = saved_regs;
        env.cpu.set_cpsr(saved_cpsr);
        env.cpu.regs_mut()[0] = r0;
        env.cpu.regs_mut()[1] = r1;
        return true;
    }
    false
}
