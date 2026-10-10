/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Host-side UIKit model for the ARM64 runtime. Pure Rust, no guest access:
//! guest objects are identified only by their 64-bit addresses. The guest
//! instances themselves carry no UIKit ivars (see a64_uikit_image.rs).
//!
//! Behaviour follows the documented UIKit semantics that the 32-bit
//! implementation in src/frameworks/uikit/ also models (frame/bounds/center
//! relationship, superview/subview ordering, key window), re-expressed for
//! 64-bit doubles (CGFloat is double on arm64).
use std::collections::BTreeMap;

pub(in crate::a64) const MAX_VIEWS: usize = 65536;
pub(in crate::a64) const MAX_CONTROLLERS: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::a64) struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::a64) struct Size {
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::a64) struct Rect {
    pub origin: Point,
    pub size: Size,
}
impl Rect {
    pub(in crate::a64) fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }
    pub(in crate::a64) fn center(&self) -> Point {
        Point {
            x: self.origin.x + self.size.width / 2.0,
            y: self.origin.y + self.size.height / 2.0,
        }
    }
    pub(in crate::a64) fn is_finite(&self) -> bool {
        [self.origin.x, self.origin.y, self.size.width, self.size.height]
            .iter()
            .all(|v| v.is_finite())
    }
}

/// UIInterfaceOrientation / UIDeviceOrientation raw values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::a64) enum Orientation {
    Portrait = 1,
    PortraitUpsideDown = 2,
    LandscapeRight = 3,
    LandscapeLeft = 4,
}
impl Orientation {
    pub(in crate::a64) fn is_landscape(self) -> bool {
        matches!(self, Self::LandscapeLeft | Self::LandscapeRight)
    }
}

/// UIApplicationState raw values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::a64) enum ApplicationState {
    Active = 0,
    Inactive = 1,
    Background = 2,
}

#[derive(Clone, Debug)]
pub(in crate::a64) struct Screen {
    /// Portrait-native bounds in points (UIScreen.nativeBounds / scale).
    pub portrait_points: Size,
    pub scale: f64,
    pub maximum_frames_per_second: i64,
}
impl Screen {
    /// iOS 8+: UIScreen.bounds follows the interface orientation.
    pub(in crate::a64) fn bounds(&self, orientation: Orientation) -> Rect {
        let Size { width, height } = self.portrait_points;
        if orientation.is_landscape() {
            Rect::new(0.0, 0.0, height, width)
        } else {
            Rect::new(0.0, 0.0, width, height)
        }
    }
    /// UIScreen.nativeBounds is always portrait, in pixels.
    pub(in crate::a64) fn native_bounds(&self) -> Rect {
        Rect::new(
            0.0,
            0.0,
            self.portrait_points.width * self.scale,
            self.portrait_points.height * self.scale,
        )
    }
}

#[derive(Clone, Debug)]
pub(in crate::a64) struct Device {
    pub model: String,
    pub system_name: String,
    pub system_version: String,
    /// UIUserInterfaceIdiom: 0 phone, 1 pad.
    pub idiom: i64,
    pub orientation: Orientation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::a64) enum ViewKind {
    View,
    Window,
}

#[derive(Clone, Debug)]
pub(in crate::a64) struct View {
    pub kind: ViewKind,
    pub frame: Rect,
    pub bounds: Rect,
    pub superview: u64,
    pub subviews: Vec<u64>,
    pub hidden: bool,
    pub opaque: bool,
    pub alpha: f64,
    pub tag: i64,
    pub user_interaction_enabled: bool,
    pub multiple_touch_enabled: bool,
    pub content_scale_factor: f64,
    /// Backing CALayer object, created on demand by QuartzCore (agent B).
    pub layer: u64,
    pub background_color: u64,
    /// Window only.
    pub root_view_controller: u64,
}
impl View {
    fn new(kind: ViewKind, frame: Rect, scale: f64) -> Self {
        Self {
            kind,
            frame,
            bounds: Rect::new(0.0, 0.0, frame.size.width, frame.size.height),
            superview: 0,
            subviews: Vec::new(),
            hidden: false,
            opaque: true,
            alpha: 1.0,
            tag: 0,
            user_interaction_enabled: true,
            multiple_touch_enabled: false,
            content_scale_factor: scale,
            layer: 0,
            background_color: 0,
            root_view_controller: 0,
        }
    }
}

/// CALayer state. Geometry follows the owning view (UIKit keeps
/// layer.bounds == view.bounds and layer.position == view.center).
#[derive(Clone, Debug)]
pub(in crate::a64) struct Layer {
    pub bounds: Rect,
    pub position: Point,
    pub contents_scale: f64,
    pub hidden: bool,
    pub opaque: bool,
    pub opacity: f32,
    pub superlayer: u64,
    pub sublayers: Vec<u64>,
    /// The view whose backing layer this is (UIView is the layer delegate).
    pub view: u64,
    pub delegate: u64,
}
impl Default for Layer {
    fn default() -> Self {
        Self {
            bounds: Rect::default(),
            position: Point::default(),
            contents_scale: 1.0,
            hidden: false,
            opaque: false,
            opacity: 1.0,
            superlayer: 0,
            sublayers: Vec::new(),
            view: 0,
            delegate: 0,
        }
    }
}
impl Layer {
    pub(in crate::a64) fn frame(&self) -> Rect {
        Rect::new(
            self.position.x - self.bounds.size.width / 2.0,
            self.position.y - self.bounds.size.height / 2.0,
            self.bounds.size.width,
            self.bounds.size.height,
        )
    }
}

#[derive(Clone, Debug, Default)]
pub(in crate::a64) struct Controller {
    pub view: u64,
    pub view_loaded: bool,
    pub parent: u64,
    pub children: Vec<u64>,
}

#[derive(Clone, Debug)]
pub(in crate::a64) struct Application {
    pub object: u64,
    pub delegate: u64,
    pub key_window: u64,
    pub windows: Vec<u64>,
    pub state: ApplicationState,
    pub idle_timer_disabled: bool,
    pub status_bar_hidden: bool,
    pub status_bar_orientation: Orientation,
    pub launched: bool,
}

#[derive(Clone, Debug)]
pub(in crate::a64) struct Model {
    pub screen: Screen,
    pub device: Device,
    pub application: Application,
    views: BTreeMap<u64, View>,
    controllers: BTreeMap<u64, Controller>,
    layers: BTreeMap<u64, Layer>,
}

impl Model {
    /// Coromon's Info.plist targets iPhone; the default is a 2x 667x375pt
    /// landscape-capable phone. Real values come from the host display later.
    pub(in crate::a64) fn new(screen: Screen, device: Device) -> Result<Self, String> {
        if !(screen.scale.is_finite() && screen.scale >= 1.0 && screen.scale <= 4.0)
            || !(screen.portrait_points.width > 0.0 && screen.portrait_points.height > 0.0)
        {
            return Err("UIKit screen metrics invalid".into());
        }
        let orientation = device.orientation;
        Ok(Self {
            screen,
            device,
            application: Application {
                object: 0,
                delegate: 0,
                key_window: 0,
                windows: Vec::new(),
                state: ApplicationState::Inactive,
                idle_timer_disabled: false,
                status_bar_hidden: false,
                status_bar_orientation: orientation,
                launched: false,
            },
            views: BTreeMap::new(),
            controllers: BTreeMap::new(),
            layers: BTreeMap::new(),
        })
    }
    pub(in crate::a64) fn screen_bounds(&self) -> Rect {
        self.screen.bounds(self.application.status_bar_orientation)
    }

    // ---- views ----
    pub(in crate::a64) fn has_view(&self, object: u64) -> bool {
        self.views.contains_key(&object)
    }
    pub(in crate::a64) fn view(&self, object: u64) -> Result<&View, String> {
        self.views
            .get(&object)
            .ok_or_else(|| format!("UIKit view {object:#x} has no host state"))
    }
    fn view_mut(&mut self, object: u64) -> Result<&mut View, String> {
        self.views
            .get_mut(&object)
            .ok_or_else(|| format!("UIKit view {object:#x} has no host state"))
    }
    /// -initWithFrame: (also used for lazily created state of views
    /// initialised through paths we do not intercept, e.g. -initWithCoder:).
    pub(in crate::a64) fn init_view(&mut self, object: u64, kind: ViewKind, frame: Rect) -> Result<(), String> {
        if object == 0 || object & 7 != 0 {
            return Err("UIKit view identity invalid".into());
        }
        if !frame.is_finite() {
            return Err("UIKit view frame is not finite".into());
        }
        if !self.views.contains_key(&object) && self.views.len() >= MAX_VIEWS {
            return Err("UIKit view limit exceeded".into());
        }
        // Re-initialisation keeps hierarchy links (UIKit tolerates this).
        let scale = self.screen.scale;
        match self.views.get_mut(&object) {
            Some(view) => {
                view.kind = kind;
                view.frame = frame;
                view.bounds = Rect::new(view.bounds.origin.x, view.bounds.origin.y, frame.size.width, frame.size.height);
            }
            None => {
                self.views.insert(object, View::new(kind, frame, scale));
            }
        }
        self.sync_layer(object);
        Ok(())
    }
    pub(in crate::a64) fn ensure_view(&mut self, object: u64, kind: ViewKind) -> Result<(), String> {
        if !self.views.contains_key(&object) {
            self.init_view(object, kind, Rect::default())?;
        }
        Ok(())
    }
    pub(in crate::a64) fn set_frame(&mut self, object: u64, frame: Rect) -> Result<(), String> {
        if !frame.is_finite() {
            return Err("UIKit view frame is not finite".into());
        }
        let view = self.view_mut(object)?;
        view.frame = frame;
        view.bounds.size = frame.size;
        self.sync_layer(object);
        Ok(())
    }
    /// Keep a view's backing layer geometry and scale in step with the view.
    pub(in crate::a64) fn sync_layer(&mut self, view: u64) {
        let Some(v) = self.views.get(&view) else { return };
        let (bounds, center, scale, hidden, opaque, layer) =
            (v.bounds, v.frame.center(), v.content_scale_factor, v.hidden, v.opaque, v.layer);
        if let Some(l) = self.layers.get_mut(&layer) {
            l.bounds = bounds;
            l.position = center;
            l.contents_scale = scale;
            l.hidden = hidden;
            l.opaque = opaque;
        }
    }

    // ---- layers ----
    pub(in crate::a64) fn init_layer(&mut self, object: u64) -> Result<(), String> {
        if object == 0 || object & 7 != 0 {
            return Err("CALayer identity invalid".into());
        }
        if !self.layers.contains_key(&object) {
            if self.layers.len() >= MAX_VIEWS {
                return Err("CALayer limit exceeded".into());
            }
            self.layers.insert(object, Layer::default());
        }
        Ok(())
    }
    pub(in crate::a64) fn has_layer(&self, object: u64) -> bool {
        self.layers.contains_key(&object)
    }
    pub(in crate::a64) fn layer(&self, object: u64) -> Result<&Layer, String> {
        self.layers
            .get(&object)
            .ok_or_else(|| format!("CALayer {object:#x} has no host state"))
    }
    pub(in crate::a64) fn layer_mut(&mut self, object: u64) -> Result<&mut Layer, String> {
        self.layers
            .get_mut(&object)
            .ok_or_else(|| format!("CALayer {object:#x} has no host state"))
    }
    /// Make `layer` the backing layer of `view` (UIView -layer).
    pub(in crate::a64) fn attach_layer(&mut self, view: u64, layer: u64) -> Result<(), String> {
        self.init_layer(layer)?;
        let current = self.view(view)?.layer;
        if current != 0 && current != layer {
            return Err("UIKit view already has a different backing layer".into());
        }
        self.view_mut(view)?.layer = layer;
        let l = self.layer_mut(layer)?;
        l.view = view;
        l.delegate = view;
        self.sync_layer(view);
        Ok(())
    }
    pub(in crate::a64) fn add_sublayer(&mut self, parent: u64, child: u64) -> Result<bool, String> {
        if parent == child {
            return Err("CALayer cannot be its own sublayer".into());
        }
        self.layer(parent)?;
        let old = self.layer(child)?.superlayer;
        let mut cursor = parent;
        let mut steps = 0;
        while cursor != 0 {
            if cursor == child || steps > MAX_VIEWS {
                return Err("CALayer sublayer would create a cycle".into());
            }
            steps += 1;
            cursor = self.layer(cursor)?.superlayer;
        }
        if let Some(p) = self.layers.get_mut(&old) {
            p.sublayers.retain(|&l| l != child);
        }
        self.layer_mut(parent)?.sublayers.push(child);
        self.layer_mut(child)?.superlayer = parent;
        Ok(old == 0)
    }
    pub(in crate::a64) fn remove_from_superlayer(&mut self, child: u64) -> Result<u64, String> {
        let old = std::mem::replace(&mut self.layer_mut(child)?.superlayer, 0);
        if let Some(p) = self.layers.get_mut(&old) {
            p.sublayers.retain(|&l| l != child);
        }
        Ok(old)
    }
    /// Disposal: returns sublayers (released by the -dealloc cleanup).
    pub(in crate::a64) fn forget_layer(&mut self, object: u64) -> Vec<u64> {
        let Some(layer) = self.layers.remove(&object) else {
            return Vec::new();
        };
        if let Some(p) = self.layers.get_mut(&layer.superlayer) {
            p.sublayers.retain(|&l| l != object);
        }
        for child in &layer.sublayers {
            if let Some(c) = self.layers.get_mut(child) {
                c.superlayer = 0;
            }
        }
        if let Some(v) = self.views.get_mut(&layer.view) {
            if v.layer == object {
                v.layer = 0;
            }
        }
        layer.sublayers
    }
    pub(in crate::a64) fn layer_count(&self) -> usize {
        self.layers.len()
    }
    pub(in crate::a64) fn set_bounds(&mut self, object: u64, bounds: Rect) -> Result<(), String> {
        if !bounds.is_finite() {
            return Err("UIKit view bounds is not finite".into());
        }
        let view = self.view_mut(object)?;
        // Changing bounds.size keeps the center fixed.
        let center = view.frame.center();
        view.bounds = bounds;
        view.frame.size = bounds.size;
        view.frame.origin = Point {
            x: center.x - bounds.size.width / 2.0,
            y: center.y - bounds.size.height / 2.0,
        };
        self.sync_layer(object);
        Ok(())
    }
    pub(in crate::a64) fn set_center(&mut self, object: u64, center: Point) -> Result<(), String> {
        if !(center.x.is_finite() && center.y.is_finite()) {
            return Err("UIKit view center is not finite".into());
        }
        let view = self.view_mut(object)?;
        view.frame.origin = Point {
            x: center.x - view.frame.size.width / 2.0,
            y: center.y - view.frame.size.height / 2.0,
        };
        self.sync_layer(object);
        Ok(())
    }
    pub(in crate::a64) fn with_view<R>(&mut self, object: u64, f: impl FnOnce(&mut View) -> R) -> Result<R, String> {
        let result = f(self.view_mut(object)?);
        self.sync_layer(object);
        Ok(result)
    }
    fn detach(&mut self, child: u64) -> Result<(), String> {
        let parent = self.view(child)?.superview;
        if parent != 0 {
            if let Some(view) = self.views.get_mut(&parent) {
                view.subviews.retain(|&s| s != child);
            }
            self.view_mut(child)?.superview = 0;
        }
        Ok(())
    }
    /// -addSubview: ; returns true if the hierarchy changed (caller retains
    /// the child in the guest when it was newly attached).
    pub(in crate::a64) fn add_subview(&mut self, parent: u64, child: u64) -> Result<bool, String> {
        if parent == child {
            return Err("UIKit view cannot be its own subview".into());
        }
        self.view(parent)?;
        self.view(child)?;
        // Reject cycles: child must not be an ancestor of parent.
        let mut cursor = parent;
        let mut steps = 0;
        while cursor != 0 {
            if cursor == child {
                return Err("UIKit subview would create a hierarchy cycle".into());
            }
            steps += 1;
            if steps > MAX_VIEWS {
                return Err("UIKit hierarchy depth limit".into());
            }
            cursor = self.view(cursor)?.superview;
        }
        let was_attached = self.view(child)?.superview != 0;
        self.detach(child)?;
        self.view_mut(parent)?.subviews.push(child);
        self.view_mut(child)?.superview = parent;
        Ok(!was_attached)
    }
    /// -removeFromSuperview ; returns the former superview (0 if none).
    pub(in crate::a64) fn remove_from_superview(&mut self, child: u64) -> Result<u64, String> {
        let parent = self.view(child)?.superview;
        self.detach(child)?;
        Ok(parent)
    }
    /// Disposal: remove the view and every reference to it. Subviews are
    /// detached (the guest's release of them is the -dealloc thunk's job).
    pub(in crate::a64) fn forget_view(&mut self, object: u64) -> Vec<u64> {
        let Some(view) = self.views.remove(&object) else {
            return Vec::new();
        };
        if let Some(parent) = self.views.get_mut(&view.superview) {
            parent.subviews.retain(|&s| s != object);
        }
        for &child in &view.subviews {
            if let Some(c) = self.views.get_mut(&child) {
                c.superview = 0;
            }
        }
        if let Some(l) = self.layers.get_mut(&view.layer) {
            l.view = 0;
            l.delegate = 0;
        }
        self.application.windows.retain(|&w| w != object);
        if self.application.key_window == object {
            self.application.key_window = 0;
        }
        for controller in self.controllers.values_mut() {
            if controller.view == object {
                controller.view = 0;
                controller.view_loaded = false;
            }
        }
        view.subviews
    }

    // ---- windows / application ----
    /// -makeKeyAndVisible
    pub(in crate::a64) fn make_key_and_visible(&mut self, window: u64) -> Result<(), String> {
        let view = self.view_mut(window)?;
        if view.kind != ViewKind::Window {
            return Err("makeKeyAndVisible receiver is not a UIWindow".into());
        }
        view.hidden = false;
        if !self.application.windows.contains(&window) {
            self.application.windows.push(window);
        }
        self.application.key_window = window;
        Ok(())
    }
    pub(in crate::a64) fn set_root_view_controller(&mut self, window: u64, controller: u64) -> Result<u64, String> {
        let view = self.view_mut(window)?;
        if view.kind != ViewKind::Window {
            return Err("rootViewController receiver is not a UIWindow".into());
        }
        let old = view.root_view_controller;
        view.root_view_controller = controller;
        if controller != 0 {
            self.ensure_controller(controller)?;
        }
        Ok(old)
    }

    // ---- view controllers ----
    pub(in crate::a64) fn ensure_controller(&mut self, object: u64) -> Result<(), String> {
        if object == 0 || object & 7 != 0 {
            return Err("UIKit view controller identity invalid".into());
        }
        if !self.controllers.contains_key(&object) {
            if self.controllers.len() >= MAX_CONTROLLERS {
                return Err("UIKit view controller limit exceeded".into());
            }
            self.controllers.insert(object, Controller::default());
        }
        Ok(())
    }
    pub(in crate::a64) fn controller(&self, object: u64) -> Option<&Controller> {
        self.controllers.get(&object)
    }
    pub(in crate::a64) fn set_controller_view(&mut self, object: u64, view: u64) -> Result<u64, String> {
        self.ensure_controller(object)?;
        let controller = self.controllers.get_mut(&object).unwrap();
        let old = controller.view;
        controller.view = view;
        controller.view_loaded = view != 0;
        Ok(old)
    }
    pub(in crate::a64) fn forget_controller(&mut self, object: u64) -> Option<Controller> {
        let removed = self.controllers.remove(&object);
        for view in self.views.values_mut() {
            if view.root_view_controller == object {
                view.root_view_controller = 0;
            }
        }
        removed
    }
    pub(in crate::a64) fn view_count(&self) -> usize {
        self.views.len()
    }
    pub(in crate::a64) fn controller_count(&self) -> usize {
        self.controllers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Model {
        Model::new(
            Screen {
                portrait_points: Size { width: 375.0, height: 667.0 },
                scale: 2.0,
                maximum_frames_per_second: 60,
            },
            Device {
                model: "iPhone".into(),
                system_name: "iOS".into(),
                system_version: "16.7.16".into(),
                idiom: 0,
                orientation: Orientation::LandscapeRight,
            },
        )
        .unwrap()
    }
    #[test]
    fn screen_bounds_follow_interface_orientation_but_native_bounds_do_not() {
        let mut m = model();
        assert_eq!(m.screen_bounds(), Rect::new(0.0, 0.0, 667.0, 375.0));
        m.application.status_bar_orientation = Orientation::Portrait;
        assert_eq!(m.screen_bounds(), Rect::new(0.0, 0.0, 375.0, 667.0));
        assert_eq!(m.screen.native_bounds(), Rect::new(0.0, 0.0, 750.0, 1334.0));
    }
    #[test]
    fn frame_bounds_center_relationships_match_uikit() {
        let mut m = model();
        m.init_view(0x1000, ViewKind::View, Rect::new(10.0, 20.0, 100.0, 50.0)).unwrap();
        assert_eq!(m.view(0x1000).unwrap().bounds, Rect::new(0.0, 0.0, 100.0, 50.0));
        m.set_center(0x1000, Point { x: 0.0, y: 0.0 }).unwrap();
        assert_eq!(m.view(0x1000).unwrap().frame, Rect::new(-50.0, -25.0, 100.0, 50.0));
        m.set_bounds(0x1000, Rect::new(0.0, 0.0, 20.0, 10.0)).unwrap();
        assert_eq!(m.view(0x1000).unwrap().frame, Rect::new(-10.0, -5.0, 20.0, 10.0));
        assert!(m.set_frame(0x1000, Rect::new(f64::NAN, 0.0, 1.0, 1.0)).is_err());
        assert!(m.set_frame(0x2000, Rect::default()).is_err());
    }
    #[test]
    fn backing_layer_follows_view_geometry_and_scale() {
        let mut m = model();
        m.init_view(0x1000, ViewKind::View, Rect::new(10.0, 20.0, 100.0, 50.0)).unwrap();
        m.attach_layer(0x1000, 0x2000).unwrap();
        assert_eq!(m.layer(0x2000).unwrap().frame(), Rect::new(10.0, 20.0, 100.0, 50.0));
        assert_eq!(m.layer(0x2000).unwrap().contents_scale, 2.0);
        m.with_view(0x1000, |v| v.content_scale_factor = 3.0).unwrap();
        m.set_center(0x1000, Point { x: 0.0, y: 0.0 }).unwrap();
        let l = m.layer(0x2000).unwrap();
        assert_eq!((l.contents_scale, l.position, l.delegate), (3.0, Point { x: 0.0, y: 0.0 }, 0x1000));
        assert!(m.attach_layer(0x1000, 0x3000).is_err());
        m.init_layer(0x4000).unwrap();
        assert!(m.add_sublayer(0x2000, 0x4000).unwrap());
        assert!(m.add_sublayer(0x4000, 0x2000).is_err());
        assert_eq!(m.forget_layer(0x2000), vec![0x4000]);
        assert_eq!(m.view(0x1000).unwrap().layer, 0);
        assert_eq!(m.layer(0x4000).unwrap().superlayer, 0);
    }
    #[test]
    fn hierarchy_rejects_cycles_and_disposal_clears_every_reference() {
        let mut m = model();
        for (object, kind) in [(0x1000, ViewKind::Window), (0x2000, ViewKind::View), (0x3000, ViewKind::View)] {
            m.init_view(object, kind, Rect::default()).unwrap();
        }
        assert!(m.add_subview(0x1000, 0x2000).unwrap());
        assert!(m.add_subview(0x2000, 0x3000).unwrap());
        assert!(m.add_subview(0x3000, 0x1000).is_err());
        assert!(m.add_subview(0x2000, 0x2000).is_err());
        // Moving an attached view is not a new attachment.
        assert!(!m.add_subview(0x1000, 0x3000).unwrap());
        assert_eq!(m.view(0x1000).unwrap().subviews, vec![0x2000, 0x3000]);
        m.make_key_and_visible(0x1000).unwrap();
        assert!(m.make_key_and_visible(0x2000).is_err());
        m.set_root_view_controller(0x1000, 0x8000).unwrap();
        m.set_controller_view(0x8000, 0x2000).unwrap();
        assert_eq!(m.forget_view(0x1000), vec![0x2000, 0x3000]);
        assert_eq!(m.application.key_window, 0);
        assert!(m.application.windows.is_empty());
        assert_eq!(m.view(0x2000).unwrap().superview, 0);
        m.forget_view(0x2000);
        assert_eq!(m.controller(0x8000).unwrap().view, 0);
        assert_eq!(m.remove_from_superview(0x3000).unwrap(), 0);
    }
}
