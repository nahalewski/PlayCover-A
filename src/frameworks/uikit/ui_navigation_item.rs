/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Retained navigation-item and bar-button state.
use crate::frameworks::foundation::{ns_array, ns_string::get_static_str, NSInteger};
use crate::objc::{
    autorelease, id, msg, msg_class, msg_send, nil, objc_classes, release, retain, ClassExports,
    HostObject, NSZonePtr, SEL,
};
use crate::{abi::GuestRet, Environment};

#[derive(Default)]
struct NavigationItem {
    title: id,
    prompt: id,
    title_view: id,
    left: id,
    right: id,
    back: id,
    hides_back: bool,
    /// Weak reference; UINavigationBar clears this before releasing items.
    navigation_bar: id,
}
impl HostObject for NavigationItem {}
pub fn set_navigation_bar(env: &mut Environment, item: id, bar: id) {
    if item != nil {
        env.objc.borrow_mut::<NavigationItem>(item).navigation_bar = bar;
    }
}
fn refresh_bar(env: &mut Environment, item: id) {
    let bar = env.objc.borrow::<NavigationItem>(item).navigation_bar;
    if bar != nil {
        () = msg![env; bar layoutSubviews];
    }
}
struct BarItem {
    title: id,
    image: id,
    custom: id,
    target: id,
    action: Option<SEL>,
    style: NSInteger,
    system: NSInteger,
    enabled: bool,
    width: f32,
}
impl Default for BarItem {
    fn default() -> Self {
        Self {
            title: nil,
            image: nil,
            custom: nil,
            target: nil,
            action: None,
            style: 0,
            system: -1,
            enabled: true,
            width: 0.0,
        }
    }
}
impl HostObject for BarItem {}
fn replace(env: &mut Environment, old: id, new: id, copy: bool) -> id {
    let new = if copy {
        msg![env;new copy]
    } else {
        retain(env, new)
    };
    release(env, old);
    new
}
fn decode(env: &mut Environment, coder: id, key: &'static str) -> id {
    let key = get_static_str(env, key);
    msg![env;coder decodeObjectForKey:key]
}
fn contains(env: &mut Environment, coder: id, key: &'static str) -> bool {
    let key = get_static_str(env, key);
    msg![env;coder containsValueForKey:key]
}
fn single(env: &mut Environment, item: id) -> id {
    if item == nil {
        nil
    } else {
        let item = retain(env, item);
        let a = ns_array::from_vec(env, vec![item]);
        autorelease(env, a)
    }
}
/// An autoreleased display/control view. The caller adds/retains it as a subview.
pub fn button_view(env: &mut Environment, item: id) -> id {
    if item == nil {
        return nil;
    }
    let h = env.objc.borrow::<BarItem>(item);
    let (custom, title, image, _target, action, enabled, system) = (
        h.custom, h.title, h.image, h.target, h.action, h.enabled, h.system,
    );
    if custom != nil {
        return custom;
    }
    let button: id = msg_class![env;UIButton buttonWithType:0i32];
    let title = if title != nil {
        title
    } else {
        let label = match system {
            0 => "Done",
            1 => "Cancel",
            2 => "Edit",
            3 => "Save",
            4 => "+",
            5 => " ",
            6 => "",
            7 => "Compose",
            8 => "Reply",
            9 => "Action",
            10 => "Organize",
            11 => "Bookmarks",
            12 => "Search",
            13 => "Refresh",
            14 => "Stop",
            15 => "Camera",
            16 => "Trash",
            17 => "Play",
            18 => "Pause",
            19 => "Rewind",
            20 => "Fast Forward",
            _ => "",
        };
        get_static_str(env, label)
    };
    () = msg![env;button setTitle:title forState:0u32];
    if image != nil {
        () = msg![env;button setImage:image forState:0u32];
    }
    () = msg![env;button setEnabled:enabled];
    if action.is_some_and(|s| !s.is_null()) {
        let dispatch = env
            .objc
            .register_host_selector("_touchHLEBarButtonTapped:".into(), &mut env.mem);
        () = msg![env;button addTarget:item action:dispatch forControlEvents:64u32];
    }
    button
}
pub const CLASSES: ClassExports = objc_classes! {
(env,this,_cmd);
@implementation UINavigationItem:NSObject
+ (id)allocWithZone:(NSZonePtr)_zone {env.objc.alloc_object(this,Box::<NavigationItem>::default(),&mut env.mem)}
- (id)initWithTitle:(id)title {()=msg![env;this setTitle:title];this}
- (id)initWithCoder:(id)coder {
if contains(env,coder,"UITitle") {let value=decode(env,coder,"UITitle");()=msg![env;this setTitle:value];}
if contains(env,coder,"UIPrompt") {let value=decode(env,coder,"UIPrompt");()=msg![env;this setPrompt:value];}
if contains(env,coder,"UITitleView") {let value=decode(env,coder,"UITitleView");()=msg![env;this setTitleView:value];}
if contains(env,coder,"UILeftBarButtonItems") {let value=decode(env,coder,"UILeftBarButtonItems");()=msg![env;this setLeftBarButtonItems:value];}
if contains(env,coder,"UIRightBarButtonItems") {let value=decode(env,coder,"UIRightBarButtonItems");()=msg![env;this setRightBarButtonItems:value];}
if contains(env,coder,"UILeftBarButtonItem") {let value=decode(env,coder,"UILeftBarButtonItem");()=msg![env;this setLeftBarButtonItem:value];}
if contains(env,coder,"UIRightBarButtonItem") {let value=decode(env,coder,"UIRightBarButtonItem");()=msg![env;this setRightBarButtonItem:value];}
if contains(env,coder,"UIBackBarButtonItem") {let value=decode(env,coder,"UIBackBarButtonItem");()=msg![env;this setBackBarButtonItem:value];}
if contains(env,coder,"UIHidesBackButton") {let key=get_static_str(env,"UIHidesBackButton");let v:bool=msg![env;coder decodeBoolForKey:key];()=msg![env;this setHidesBackButton:v];}this
}
- (id)title {env.objc.borrow::<NavigationItem>(this).title}
- (())setTitle:(id)value {let old=env.objc.borrow::<NavigationItem>(this).title;let value=replace(env,old,value,true);env.objc.borrow_mut::<NavigationItem>(this).title=value;refresh_bar(env,this);}
- (id)prompt {env.objc.borrow::<NavigationItem>(this).prompt}
- (())setPrompt:(id)value {let old=env.objc.borrow::<NavigationItem>(this).prompt;let value=replace(env,old,value,true);env.objc.borrow_mut::<NavigationItem>(this).prompt=value;refresh_bar(env,this);}
- (id)titleView {env.objc.borrow::<NavigationItem>(this).title_view}
- (())setTitleView:(id)value {let old=env.objc.borrow::<NavigationItem>(this).title_view;let value=replace(env,old,value,false);env.objc.borrow_mut::<NavigationItem>(this).title_view=value;refresh_bar(env,this);}
- (id)leftBarButtonItems {env.objc.borrow::<NavigationItem>(this).left}
- (())setLeftBarButtonItems:(id)value {let old=env.objc.borrow::<NavigationItem>(this).left;let value=replace(env,old,value,true);env.objc.borrow_mut::<NavigationItem>(this).left=value;refresh_bar(env,this);}
- (id)rightBarButtonItems {env.objc.borrow::<NavigationItem>(this).right}
- (())setRightBarButtonItems:(id)value {let old=env.objc.borrow::<NavigationItem>(this).right;let value=replace(env,old,value,true);env.objc.borrow_mut::<NavigationItem>(this).right=value;refresh_bar(env,this);}
- (id)backBarButtonItem {env.objc.borrow::<NavigationItem>(this).back}
- (())setBackBarButtonItem:(id)value {let old=env.objc.borrow::<NavigationItem>(this).back;let value=replace(env,old,value,false);env.objc.borrow_mut::<NavigationItem>(this).back=value;refresh_bar(env,this);}
- (id)leftBarButtonItem {let array=env.objc.borrow::<NavigationItem>(this).left;let n:u32=msg![env;array count];if n==0 {nil}else{msg![env;array objectAtIndex:0u32]}}
- (())setLeftBarButtonItem:(id)value {let array=single(env,value);()=msg![env;this setLeftBarButtonItems:array];}
- (())setLeftBarButtonItem:(id)value animated:(bool)_animated {()=msg![env;this setLeftBarButtonItem:value];}
- (())setLeftBarButtonItems:(id)value animated:(bool)_animated {()=msg![env;this setLeftBarButtonItems:value];}
- (id)rightBarButtonItem {let array=env.objc.borrow::<NavigationItem>(this).right;let n:u32=msg![env;array count];if n==0 {nil}else{msg![env;array objectAtIndex:0u32]}}
- (())setRightBarButtonItem:(id)value {let array=single(env,value);()=msg![env;this setRightBarButtonItems:array];}
- (())setRightBarButtonItem:(id)value animated:(bool)_animated {()=msg![env;this setRightBarButtonItem:value];}
- (())setRightBarButtonItems:(id)value animated:(bool)_animated {()=msg![env;this setRightBarButtonItems:value];}
- (bool)hidesBackButton {env.objc.borrow::<NavigationItem>(this).hides_back}
- (())setHidesBackButton:(bool)value {env.objc.borrow_mut::<NavigationItem>(this).hides_back=value;refresh_bar(env,this);}
- (())setHidesBackButton:(bool)value animated:(bool)_animated {()=msg![env;this setHidesBackButton:value];}
- (())dealloc {let h=env.objc.borrow::<NavigationItem>(this);let values=[h.title,h.prompt,h.title_view,h.left,h.right,h.back];for value in values {release(env,value);}env.objc.dealloc_object(this,&mut env.mem);}
@end
@implementation UIBarButtonItem:NSObject
+ (id)allocWithZone:(NSZonePtr)_zone {env.objc.alloc_object(this,Box::<BarItem>::default(),&mut env.mem)}
- (id)initWithTitle:(id)title style:(NSInteger)style target:(id)target action:(SEL)action {
 ()=msg![env;this setTitle:title];()=msg![env;this setStyle:style];()=msg![env;this setTarget:target];()=msg![env;this setAction:action];this
}
- (id)initWithImage:(id)image style:(NSInteger)style target:(id)target action:(SEL)action {
 ()=msg![env;this setImage:image];()=msg![env;this setStyle:style];()=msg![env;this setTarget:target];()=msg![env;this setAction:action];this
}
- (id)initWithBarButtonSystemItem:(NSInteger)system target:(id)target action:(SEL)action {
 env.objc.borrow_mut::<BarItem>(this).system=system;()=msg![env;this setTarget:target];()=msg![env;this setAction:action];this
}
- (id)initWithCustomView:(id)view {()=msg![env;this setCustomView:view];this}
- (id)initWithCoder:(id)coder {
if contains(env,coder,"UITitle") {let value=decode(env,coder,"UITitle");()=msg![env;this setTitle:value];}
if contains(env,coder,"UIImage") {let value=decode(env,coder,"UIImage");()=msg![env;this setImage:value];}
if contains(env,coder,"UICustomView") {let value=decode(env,coder,"UICustomView");()=msg![env;this setCustomView:value];}
for (key,field) in [("UIStyle",0),("UISystemItem",1)] {if contains(env,coder,key) {let key=get_static_str(env,key);let v:NSInteger=msg![env;coder decodeIntForKey:key];let h=env.objc.borrow_mut::<BarItem>(this);if field==0{h.style=v;}else{h.system=v;}}}
if contains(env,coder,"UIEnabled") {let key=get_static_str(env,"UIEnabled");let v:bool=msg![env;coder decodeBoolForKey:key];()=msg![env;this setEnabled:v];}this
}
- (id)title {env.objc.borrow::<BarItem>(this).title}
- (())setTitle:(id)value {let old=env.objc.borrow::<BarItem>(this).title;let value=replace(env,old,value,true);env.objc.borrow_mut::<BarItem>(this).title=value;}
- (id)image {env.objc.borrow::<BarItem>(this).image}
- (())setImage:(id)value {let old=env.objc.borrow::<BarItem>(this).image;let value=replace(env,old,value,false);env.objc.borrow_mut::<BarItem>(this).image=value;}
- (id)customView {env.objc.borrow::<BarItem>(this).custom}
- (())setCustomView:(id)value {let old=env.objc.borrow::<BarItem>(this).custom;let value=replace(env,old,value,false);env.objc.borrow_mut::<BarItem>(this).custom=value;}
- (NSInteger)style {env.objc.borrow::<BarItem>(this).style}
- (())setStyle:(NSInteger)value {env.objc.borrow_mut::<BarItem>(this).style=value;}
- (bool)isEnabled {env.objc.borrow::<BarItem>(this).enabled}
- (())setEnabled:(bool)value {env.objc.borrow_mut::<BarItem>(this).enabled=value;}
- (f32)width {env.objc.borrow::<BarItem>(this).width}
- (())setWidth:(f32)value {env.objc.borrow_mut::<BarItem>(this).width=value;}
- (id)target {env.objc.borrow::<BarItem>(this).target}
- (())setTarget:(id)value {env.objc.borrow_mut::<BarItem>(this).target=value;}
- (SEL)action {env.objc.borrow::<BarItem>(this).action.unwrap_or_else(||<SEL as GuestRet>::from_regs(&[0]))}
- (())setAction:(SEL)value {env.objc.borrow_mut::<BarItem>(this).action=Some(value);}
- (())_touchHLEBarButtonTapped:(id)_sender {
 let h=env.objc.borrow::<BarItem>(this);let (target,action,enabled)=(h.target,h.action,h.enabled);
 if enabled && target!=nil {if let Some(action)=action.filter(|s|!s.is_null()) {
 let count=action.as_str(&env.mem).bytes().filter(|b|*b==b':').count();
 match count {0=>{()=msg_send(env,(target,action));},1=>{()=msg_send(env,(target,action,this));},_=>panic!("unsupported UIBarButtonItem action arity")}
 }}
}
- (())dealloc {let h=env.objc.borrow::<BarItem>(this);let values=[h.title,h.image,h.custom];for value in values{release(env,value);}env.objc.dealloc_object(this,&mut env.mem);}
@end
};
