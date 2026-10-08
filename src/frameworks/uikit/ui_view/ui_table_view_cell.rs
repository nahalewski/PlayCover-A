/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Basic table cells with real content, labels, and selection state.
use crate::frameworks::core_graphics::{CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::NSInteger;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};

#[derive(Default)]
struct CellHostObject {
    superclass: super::UIViewHostObject,
    identifier: id,
    content: id,
    label: id,
    detail: id,
    image: id,
    selected: bool,
    accessory: NSInteger,
}
impl_HostObject_with_superclass!(CellHostObject);

pub const CLASSES: ClassExports = objc_classes! {
(env, this, _cmd);
@implementation UITableViewCell: UIView
+ (id)allocWithZone:(NSZonePtr)_zone {
    env.objc.alloc_object(this, Box::<CellHostObject>::default(), &mut env.mem)
}
- (id)initWithStyle:(NSInteger)_style reuseIdentifier:(id)identifier {
    let frame = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 320.0, height: 44.0 } };
    let this: id = msg_super![env; this initWithFrame:frame];
    retain(env, identifier); env.objc.borrow_mut::<CellHostObject>(this).identifier = identifier;
    this
}
- (id)initWithFrame:(CGRect)frame reuseIdentifier:(id)identifier {
    let this: id = msg_super![env; this initWithFrame:frame];
    retain(env, identifier); env.objc.borrow_mut::<CellHostObject>(this).identifier = identifier;
    this
}
- (())dealloc {
    let identifier = env.objc.borrow::<CellHostObject>(this).identifier;
    release(env, identifier);
    msg_super![env; this dealloc]
}
- (id)reuseIdentifier { env.objc.borrow::<CellHostObject>(this).identifier }
- (id)contentView {
    let existing = env.objc.borrow::<CellHostObject>(this).content;
    if existing != nil { return existing; }
    let bounds: CGRect = msg![env; this bounds];
    let content: id = msg_class![env; UIView alloc];
    let content: id = msg![env; content initWithFrame:bounds];
    () = msg![env; this addSubview:content]; release(env, content);
    env.objc.borrow_mut::<CellHostObject>(this).content = content;
    content
}
- (id)textLabel {
    let existing = env.objc.borrow::<CellHostObject>(this).label;
    if existing != nil { return existing; }
    let content: id = msg![env; this contentView];
    let mut bounds: CGRect = msg![env; this bounds]; bounds.origin.x = 10.0; bounds.size.width -= 20.0;
    let label: id = msg_class![env; UILabel alloc]; let label: id = msg![env; label initWithFrame:bounds];
    () = msg![env; content addSubview:label]; release(env, label);
    env.objc.borrow_mut::<CellHostObject>(this).label = label;
    label
}
- (id)detailTextLabel {
    let existing = env.objc.borrow::<CellHostObject>(this).detail;
    if existing != nil { return existing; }
    let content: id = msg![env; this contentView];
    let mut bounds: CGRect = msg![env; this bounds]; bounds.origin.x = 10.0; bounds.origin.y = bounds.size.height / 2.0; bounds.size.height /= 2.0; bounds.size.width -= 20.0;
    let label: id = msg_class![env; UILabel alloc]; let label: id = msg![env; label initWithFrame:bounds];
    () = msg![env; content addSubview:label]; release(env, label);
    env.objc.borrow_mut::<CellHostObject>(this).detail = label;
    label
}
- (id)imageView {
    let existing = env.objc.borrow::<CellHostObject>(this).image;
    if existing != nil { return existing; }
    let content: id = msg![env; this contentView];
    let frame = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 40.0, height: 40.0 } };
    let image: id = msg_class![env; UIImageView alloc]; let image: id = msg![env; image initWithFrame:frame];
    () = msg![env; content addSubview:image]; release(env, image);
    env.objc.borrow_mut::<CellHostObject>(this).image = image;
    image
}
- (id)text { let label: id = msg![env; this textLabel]; msg![env; label text] }
- (())setText:(id)text { let label: id = msg![env; this textLabel]; () = msg![env; label setText:text]; }
- (bool)isSelected { env.objc.borrow::<CellHostObject>(this).selected }
- (())setSelected:(bool)selected animated:(bool)_animated {
    env.objc.borrow_mut::<CellHostObject>(this).selected = selected;
    let color: id = if selected { msg_class![env; UIColor blueColor] } else { msg_class![env; UIColor whiteColor] };
    () = msg![env; this setBackgroundColor:color];
}
- (())setSelected:(bool)selected { () = msg![env; this setSelected:selected animated:false]; }
- (NSInteger)accessoryType { env.objc.borrow::<CellHostObject>(this).accessory }
- (())setAccessoryType:(NSInteger)value { env.objc.borrow_mut::<CellHostObject>(this).accessory = value; }
@end
};
