/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Small, bounded, non-virtualized table implementation. Grouped styling,
//! editing, headers, and UIKit's full reuse/animation behavior are not provided.
use crate::frameworks::core_graphics::{CGFloat, CGPoint, CGRect, CGSize};
use crate::frameworks::foundation::{ns_string::get_static_str, NSInteger};
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, msg_class, msg_super, nil, objc_classes, release,
    retain, ClassExports, NSZonePtr,
};
use crate::Environment;

struct Row {
    cell: id,
    path: id,
    y: CGFloat,
    height: CGFloat,
}
struct TableHostObject {
    superclass: super::ui_scroll_view::UIScrollViewHostObject,
    data_source: id,
    rows: Vec<Row>,
    row_height: CGFloat,
    separator_style: NSInteger,
    style: NSInteger,
    selected: id,
    dirty: bool,
    reloading: bool,
}
impl_HostObject_with_superclass!(TableHostObject);
impl Default for TableHostObject {
    fn default() -> Self {
        Self {
            superclass: Default::default(),
            data_source: nil,
            rows: Vec::new(),
            row_height: 44.0,
            separator_style: 1,
            style: 0,
            selected: nil,
            dirty: true,
            reloading: false,
        }
    }
}
fn responds(env: &mut Environment, target: id, selector: &str) -> bool {
    let sel = env
        .objc
        .register_host_selector(selector.into(), &mut env.mem);
    msg![env; target respondsToSelector:sel]
}
fn valid_height(height: CGFloat) -> CGFloat {
    if height.is_finite() && height > 0.0 {
        height
    } else {
        44.0
    }
}
fn reload(env: &mut Environment, this: id) {
    if env.objc.borrow::<TableHostObject>(this).reloading {
        return;
    }
    let source = env.objc.borrow::<TableHostObject>(this).data_source;
    if source == nil {
        return;
    }
    env.objc.borrow_mut::<TableHostObject>(this).reloading = true;
    let rows = std::mem::take(&mut env.objc.borrow_mut::<TableHostObject>(this).rows);
    for row in rows {
        () = msg![env; (row.cell) removeFromSuperview];
        release(env, row.path);
    }
    env.objc.borrow_mut::<TableHostObject>(this).selected = nil;
    let sections: NSInteger = if responds(env, source, "numberOfSectionsInTableView:") {
        msg![env; source numberOfSectionsInTableView:this]
    } else {
        1
    };
    assert!(
        (0..=256).contains(&sections),
        "UITableView section limit exceeded"
    );
    let bounds: CGRect = msg![env; this bounds];
    let delegate: id = msg![env; this delegate];
    let default_height = env.objc.borrow::<TableHostObject>(this).row_height;
    let mut y = 0.0;
    let mut rows = Vec::new();
    for section in 0..sections {
        let count: NSInteger = msg![env; source tableView:this numberOfRowsInSection:section];
        assert!(
            count >= 0 && rows.len() + count as usize <= 4096,
            "UITableView row limit exceeded"
        );
        for row in 0..count {
            let path: id = msg_class![env; NSIndexPath indexPathForRow:row inSection:section];
            let height = valid_height(
                if responds(env, delegate, "tableView:heightForRowAtIndexPath:") {
                    msg![env; delegate tableView:this heightForRowAtIndexPath:path]
                } else {
                    default_height
                },
            );
            let cell: id = msg![env; source tableView:this cellForRowAtIndexPath:path];
            assert!(cell != nil, "UITableView data source returned nil cell");
            let frame = CGRect {
                origin: CGPoint { x: 0.0, y },
                size: CGSize {
                    width: bounds.size.width,
                    height,
                },
            };
            () = msg![env; cell setFrame:frame];
            () = msg![env; this addSubview:cell];
            retain(env, path);
            rows.push(Row {
                cell,
                path,
                y,
                height,
            });
            y += height;
        }
    }
    env.objc.borrow_mut::<TableHostObject>(this).rows = rows;
    let state = env.objc.borrow_mut::<TableHostObject>(this);
    state.dirty = false;
    state.reloading = false;
    () = msg![env; this setContentSize:(CGSize { width: bounds.size.width, height: y })];
}
pub const CLASSES: ClassExports = objc_classes! {
(env, this, _cmd);
@implementation UITableView: UIScrollView
+ (id)allocWithZone:(NSZonePtr)_zone { env.objc.alloc_object(this, Box::<TableHostObject>::default(), &mut env.mem) }
- (id)initWithFrame:(CGRect)frame style:(NSInteger)style {
    let this: id = msg_super![env; this initWithFrame:frame]; env.objc.borrow_mut::<TableHostObject>(this).style = style; this
}
- (id)initWithCoder:(id)coder {
    let this: id = msg_super![env; this initWithCoder:coder];
    let key = get_static_str(env, "UISeparatorStyle"); let style: NSInteger = msg![env; coder decodeIntForKey:key];
    env.objc.borrow_mut::<TableHostObject>(this).separator_style = style;
    // Delegates/data sources are restored via outlet connections, avoiding NIB cycles.
    this
}
- (())dealloc {
    let rows = std::mem::take(&mut env.objc.borrow_mut::<TableHostObject>(this).rows);
    for row in rows { release(env, row.path); }
    msg_super![env; this dealloc]
}
- (id)dataSource { env.objc.borrow::<TableHostObject>(this).data_source }
- (())setDataSource:(id)source { let state = env.objc.borrow_mut::<TableHostObject>(this); state.data_source = source; state.dirty = true; }
- (CGFloat)rowHeight { env.objc.borrow::<TableHostObject>(this).row_height }
- (())setRowHeight:(CGFloat)height { let state = env.objc.borrow_mut::<TableHostObject>(this); state.row_height = valid_height(height); state.dirty = true; }
- (NSInteger)style { env.objc.borrow::<TableHostObject>(this).style }
- (NSInteger)separatorStyle { env.objc.borrow::<TableHostObject>(this).separator_style }
- (())setSeparatorStyle:(NSInteger)style { env.objc.borrow_mut::<TableHostObject>(this).separator_style = style; }
- (())reloadData { reload(env, this); }
- (())layoutSubviews { if env.objc.borrow::<TableHostObject>(this).dirty { reload(env, this); } }
- (id)dequeueReusableCellWithIdentifier:(id)_identifier { nil /* No unused recycled cells in this bounded implementation. */ }
- (id)cellForRowAtIndexPath:(id)path {
    let section: NSInteger = msg![env; path section]; let row: NSInteger = msg![env; path row];
    let paths: Vec<(id,id)> = env.objc.borrow::<TableHostObject>(this).rows.iter().map(|r|(r.path,r.cell)).collect();
    for (p,c) in paths { let s: NSInteger = msg![env; p section]; let r: NSInteger = msg![env; p row]; if s == section && r == row { return c; } } nil
}
- (id)indexPathForSelectedRow { env.objc.borrow::<TableHostObject>(this).selected }
- (())deselectRowAtIndexPath:(id)path animated:(bool)animated {
    let cell: id = msg![env; this cellForRowAtIndexPath:path];
    () = msg![env; cell setSelected:false animated:animated]; env.objc.borrow_mut::<TableHostObject>(this).selected = nil;
}
- (())touchesEnded:(id)touches withEvent:(id)_event {
    let touch: id = msg![env; touches anyObject]; let location: CGPoint = msg![env; touch locationInView:this];
    let selected = env.objc.borrow::<TableHostObject>(this).rows.iter().find(|r| location.y >= r.y && location.y < r.y+r.height).map(|r|(r.cell,r.path));
    if let Some((cell,path)) = selected {
        let previous = env.objc.borrow::<TableHostObject>(this).selected;
        if previous != nil { () = msg![env; this deselectRowAtIndexPath:previous animated:false]; }
        env.objc.borrow_mut::<TableHostObject>(this).selected = path;
        () = msg![env; cell setSelected:true animated:false];
        let delegate: id = msg![env; this delegate];
        if responds(env, delegate, "tableView:didSelectRowAtIndexPath:") { () = msg![env; delegate tableView:this didSelectRowAtIndexPath:path]; }
    }
}
@end
};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_heights_are_positive_and_finite() {
        assert_eq!(valid_height(32.0), 32.0);
        for h in [0.0, -1.0, CGFloat::NAN, CGFloat::INFINITY] {
            assert_eq!(valid_height(h), 44.0);
        }
    }
}
