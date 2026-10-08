/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Functions, traits, and all kinds of things to assist with bridging the gap
//! between guest and host when it comes to animations in Core Animation.
//! Based in Apple's documented behavior for Core Animation, although not an
//! exact match.
//! References:
//! - Core Animation Programming Guide
//!   <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/CoreAnimation_guide/Introduction/Introduction.html>
//! - List of Animatable properties
//!   <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/CoreAnimation_guide/AnimatableProperties/AnimatableProperties.html#//apple_ref/doc/uid/TP40004514-CH11-SW2>
//! - Animation timing behavior, layers' local time, autoreverses, etc.
//!   <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/CoreAnimation_guide/AdvancedAnimationTricks/AdvancedAnimationTricks.html>
//! - Algorithm for choosing interpolation values
//!   <https://developer.apple.com/documentation/quartzcore/cabasicanimation?language=objc>
use std::ops::Sub;

use crate::frameworks::core_animation::ca_animation::{
    get_animation_start_time, kCAFillModeBackwards, kCAFillModeBoth, kCAFillModeForwards,
    CAMediaTimingFillMode,
};
use crate::frameworks::core_animation::ca_layer::remove_anonymous_animation;
use crate::frameworks::core_animation::{ca_layer::CALayerHostObject, CACurrentMediaTime};
use crate::frameworks::core_foundation::time::CFTimeInterval;
use crate::frameworks::core_graphics::cg_affine_transform::CGAffineTransform;
use crate::frameworks::core_graphics::cg_color::CGColorHostObject;
use crate::frameworks::foundation::ns_string::{from_rust_string, to_rust_string};
use crate::objc::{id, msg, nil, release, retain};
use crate::Environment;

#[derive(Default)]
pub struct State {
    started_animations: Vec<id>,
    finished_animations: Vec<(id, id, bool, bool, Option<String>)>,
}
impl State {
    pub fn create_presentation_layer(
        &mut self,
        env: &mut Environment,
        layer: id,
    ) -> CALayerHostObject {
        // Clone given layer
        let original = env.objc.borrow::<CALayerHostObject>(layer);
        let mut presentation = original.clone();

        // Loop over all animations and set the presentation layer's values
        let named_animations: Vec<(Option<String>, id)> = presentation
            .animations
            .iter()
            .map(|(key, anim)| (Some(key.clone()), *anim))
            .collect();
        let anonymous_animations: Vec<(Option<String>, id)> = presentation
            .anonymous_animations
            .iter()
            .map(|anim| (None, *anim))
            .collect();

        for (key, animation) in
            Iterator::chain(named_animations.iter(), anonymous_animations.iter())
        {
            let animation = *animation;

            let fill_mode: CAMediaTimingFillMode = msg![env; animation fillMode];
            let fill_mode = to_rust_string(env, fill_mode);

            // TODO: Convert to local time
            let current_time = CACurrentMediaTime(env);
            let begin_time: CFTimeInterval = msg![env; animation beginTime];
            let start_time = get_animation_start_time(env, animation);

            if current_time >= begin_time {
                if start_time.is_none() {
                    // Animation started but isn't marked as such
                    start_time.replace(current_time);
                    self.started_animations.push(animation);
                }
            } else if fill_mode != kCAFillModeBackwards && fill_mode != kCAFillModeBoth {
                continue;
            }

            let effective_begin_time = start_time.unwrap_or(begin_time);

            if let Some(key) = key {
                log_dbg!(
                    "Animate CALayer {:?} animation {} {:?}",
                    layer,
                    key,
                    animation
                );
            } else {
                log_dbg!("Animate CALayer {:?} animation {:?}", layer, animation);
            }

            let repeat_count: f32 = msg![env; animation repeatCount];
            assert!(repeat_count >= 0.0);
            // Setting [repeatCount] to greatestFiniteMagnitude will cause
            // the animation to repeat forever.
            let effective_repeat_count = if repeat_count == f32::MAX {
                f32::INFINITY
            } else if repeat_count == 0.0 {
                1.0
            } else {
                repeat_count
            };

            let duration: CFTimeInterval = msg![env; animation duration];
            let current_repeat = (((current_time - effective_begin_time).max(0.0) / duration)
                as f32)
                .min(effective_repeat_count);

            let autoreverses: bool = msg![env; animation autoreverses];
            let progress = animation_progress(current_repeat, effective_repeat_count, autoreverses);

            let timing_function: id = msg![env; animation timingFunction];
            let interpolation_amount: f32 = msg![env; timing_function _solveForInput: progress];

            if current_repeat >= effective_repeat_count {
                let removed_on_completion: bool = msg![env; animation isRemovedOnCompletion];
                self.finished_animations.push((
                    layer,
                    animation,
                    true,
                    removed_on_completion,
                    key.to_owned(),
                ));
                if fill_mode != kCAFillModeForwards && fill_mode != kCAFillModeBoth {
                    continue;
                }
            }

            // Assuming all animations here are CABasicAnimation
            // TODO: Handle other types of animations

            let from_value: id = msg![env; animation fromValue];
            let to_value: id = msg![env; animation toValue];
            let by_value: id = msg![env; animation byValue];

            // Update values only in the presentation layer
            let key_path: id = msg![env; animation keyPath];
            let key_path = to_rust_string(env, key_path);
            // Only these properties are animatable
            // TODO: Implement for all properties
            match &*key_path {
                // Apple defines these keys as scalar radians about z.
                // Interpolate angles directly so full turns retain their winding.
                "transform.rotation.z" | "transform.rotation" => {
                    let from = id_as_option(from_value).map(|obj| msg![env; obj floatValue]);
                    let to = id_as_option(to_value).map(|obj| msg![env; obj floatValue]);
                    let by = id_as_option(by_value).map(|obj| msg![env; obj floatValue]);
                    presentation.affine_transform = animate_z_rotation(
                        presentation.affine_transform,
                        from,
                        to,
                        by,
                        interpolation_amount,
                    )
                    .unwrap_or_else(|reason| panic!("Unsupported z rotation animation: {reason}"));
                }
                "anchorPoint" => {
                    let from_value =
                        id_as_option(from_value).map(|obj| msg![env; obj CGPointValue]);
                    let to_value = id_as_option(to_value).map(|obj| msg![env; obj CGPointValue]);
                    let by_value = id_as_option(by_value).map(|obj| msg![env; obj CGPointValue]);
                    let (from_value, by_value) = get_from_and_by_values(
                        Some(presentation.anchor_point),
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.anchor_point = from_value + by_value * interpolation_amount;
                }
                "backgroundColor" => {
                    let from_value = id_as_option(from_value)
                        .map(|obj| *env.objc.borrow::<CGColorHostObject>(obj));
                    let to_value = id_as_option(to_value)
                        .map(|obj| *env.objc.borrow::<CGColorHostObject>(obj));
                    let by_value = id_as_option(by_value)
                        .map(|obj| *env.objc.borrow::<CGColorHostObject>(obj));
                    let (from_value, by_value) = get_from_and_by_values(
                        presentation.background_color,
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.background_color =
                        Some(from_value + by_value * interpolation_amount)
                }
                "bounds" => {
                    let from_value = id_as_option(from_value).map(|obj| msg![env; obj CGRectValue]);
                    let to_value = id_as_option(to_value).map(|obj| msg![env; obj CGRectValue]);
                    let by_value = id_as_option(by_value).map(|obj| msg![env; obj CGRectValue]);
                    let (from_value, by_value) = get_from_and_by_values(
                        Some(presentation.bounds),
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.bounds = from_value + by_value * interpolation_amount;
                }
                "cornerRadius" => {
                    let from_value = id_as_option(from_value).map(|obj| msg![env; obj floatValue]);
                    let to_value = id_as_option(to_value).map(|obj| msg![env; obj floatValue]);
                    let by_value = id_as_option(by_value).map(|obj| msg![env; obj floatValue]);
                    let (from_value, by_value) = get_from_and_by_values(
                        Some(presentation.corner_radius),
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.corner_radius = from_value + by_value * interpolation_amount;
                }
                "hidden" => {
                    let from_value = id_as_option(from_value)
                        .map(|obj| msg![env; obj boolValue])
                        .map(|val: bool| val as i32 as f32);
                    let to_value = id_as_option(to_value)
                        .map(|obj| msg![env; obj boolValue])
                        .map(|val: bool| val as i32 as f32);
                    let by_value = id_as_option(by_value)
                        .map(|obj| msg![env; obj boolValue])
                        .map(|val: bool| val as i32 as f32);
                    let (from_value, by_value) = get_from_and_by_values(
                        Some(presentation.hidden as i32 as f32),
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.hidden = (from_value + by_value * interpolation_amount) > 0.5;
                }
                "opacity" => {
                    let from_value = id_as_option(from_value).map(|obj| msg![env; obj floatValue]);
                    let to_value = id_as_option(to_value).map(|obj| msg![env; obj floatValue]);
                    let by_value = id_as_option(by_value).map(|obj| msg![env; obj floatValue]);
                    let (from_value, by_value) = get_from_and_by_values(
                        Some(presentation.opacity),
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.opacity = from_value + by_value * interpolation_amount;
                }
                "position" => {
                    let from_value =
                        id_as_option(from_value).map(|obj| msg![env; obj CGPointValue]);
                    let to_value = id_as_option(to_value).map(|obj| msg![env; obj CGPointValue]);
                    let by_value = id_as_option(by_value).map(|obj| msg![env; obj CGPointValue]);
                    let (from_value, by_value) = get_from_and_by_values(
                        Some(presentation.position),
                        from_value,
                        to_value,
                        by_value,
                    );
                    presentation.position = from_value + by_value * interpolation_amount;
                }
                _ => panic!("Attempted to animate on key {}", key_path),
            }
        }

        presentation
    }

    pub fn update_started_and_finished_animations(self, env: &mut Environment) {
        for animation in self.started_animations {
            let delegate = msg![env; animation delegate];
            if delegate != nil {
                () = msg![env; delegate animationDidStart: animation];
            }
        }

        for (layer, ..) in &self.finished_animations {
            retain(env, *layer);
        }
        for (layer, animation, finished, removed_on_completion, key) in self.finished_animations {
            let delegate = msg![env; animation delegate];
            if delegate != nil {
                () = msg![env; delegate animationDidStop: animation finished: finished];
            }

            if removed_on_completion {
                if let Some(key) = key {
                    let key = from_rust_string(env, key);
                    () = msg![env; layer removeAnimationForKey: key];
                } else {
                    remove_anonymous_animation(env, layer, animation);
                }
            }

            release(env, layer);
        }
    }
}

fn get_from_and_by_values<T>(
    current_value: Option<T>,
    from_value: Option<T>,
    to_value: Option<T>,
    by_value: Option<T>,
) -> (T, T)
where
    T: Copy + Sub<Output = T>,
{
    if from_value.is_some() && to_value.is_some() && by_value.is_some() {
        panic!("Cannot specify all three of fromValue, toValue, and byValue");
    } else if let (Some(from_value), Some(to_value)) = (from_value, to_value) {
        let by_value = to_value - from_value.to_owned();
        (from_value, by_value)
    } else if let (Some(from_value), Some(by_value)) = (from_value, by_value) {
        (from_value.to_owned(), by_value.to_owned())
    } else if let (Some(to_value), Some(by_value)) = (to_value, by_value) {
        let from_value = to_value - by_value;
        (from_value, by_value.to_owned())
    } else if let Some(from_value) = from_value {
        let by_value = current_value.unwrap() - from_value;
        (from_value.to_owned(), by_value)
    } else if let Some(to_value) = to_value {
        let from_value = current_value.unwrap();
        let by_value = to_value - from_value;
        (from_value.to_owned(), by_value)
    } else if let Some(by_value) = by_value {
        let from_value = current_value.unwrap();
        (from_value.to_owned(), by_value.to_owned())
    } else {
        // TODO: All properties are nil. Interpolates between the previous
        // value of keyPath in the target layer’s presentation layer and the
        // current value of keyPath in the target layer’s presentation layer.
        unimplemented!()
    }
}

fn id_as_option(value: id) -> Option<id> {
    if value == nil {
        None
    } else {
        Some(value)
    }
}

fn animation_progress(current_repeat: f32, repeat_count: f32, autoreverses: bool) -> f32 {
    let mut progress = current_repeat.fract();
    // Completed whole forward repeats end at 1, not fract()'s zero. An
    // autoreversed whole repeat still ends at its initial value.
    if current_repeat >= repeat_count && current_repeat > 0.0 && progress == 0.0 {
        progress = 1.0;
    }
    if autoreverses {
        ((progress * 2.0 - 1.0).abs() - 1.0).abs()
    } else {
        progress
    }
}

/// Preserve positive axis scales and translation while replacing 2D rotation.
/// Shear, reflection and degenerate transforms need fuller decomposition.
fn animate_z_rotation(
    transform: CGAffineTransform,
    from: Option<f32>,
    to: Option<f32>,
    by: Option<f32>,
    progress: f32,
) -> Result<CGAffineTransform, &'static str> {
    let CGAffineTransform { a, b, c, d, tx, ty } = transform;
    if ![a, b, c, d, tx, ty, progress]
        .into_iter()
        .all(f32::is_finite)
        || from.into_iter().chain(to).chain(by).any(|v| !v.is_finite())
    {
        return Err("nonfinite scalar or affine transform");
    }
    let sx = a.hypot(b);
    let sy = c.hypot(d);
    let product = sx * sy;
    if !product.is_finite()
        || product == 0.0
        || a * d - b * c <= 0.0
        || (a * c + b * d).abs() > product * 1e-5
    {
        return Err("requires nonsingular positive-scale affine transform without shear");
    }
    if from.is_none() && to.is_none() && by.is_none() {
        return Err("no scalar endpoints");
    }
    if from.is_some() && to.is_some() && by.is_some() {
        return Err("from, to, and by cannot all be supplied");
    }
    let (from, by) = get_from_and_by_values(Some(b.atan2(a)), from, to, by);
    let angle = from + by * progress;
    if !angle.is_finite() {
        return Err("interpolated rotation overflow");
    }
    let (sin, cos) = angle.sin_cos();
    Ok(CGAffineTransform {
        a: sx * cos,
        b: sx * sin,
        c: -sy * sin,
        d: sy * cos,
        tx,
        ty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frameworks::core_graphics::cg_affine_transform::CGAffineTransformIdentity;
    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
    #[test]
    fn rotation_retains_full_turn_winding() {
        let t = animate_z_rotation(
            CGAffineTransformIdentity,
            Some(0.0),
            Some(std::f32::consts::TAU),
            None,
            0.25,
        )
        .unwrap();
        close(t.a, 0.0);
        close(t.b, 1.0);
        close(t.c, -1.0);
        close(t.d, 0.0);
        let t = animate_z_rotation(
            CGAffineTransformIdentity,
            Some(0.0),
            Some(std::f32::consts::TAU),
            None,
            0.5,
        )
        .unwrap();
        close(t.a, -1.0);
        close(t.b, 0.0);
    }
    #[test]
    fn rotation_preserves_scales_translation_and_implicit_model_angle() {
        let model = CGAffineTransform {
            a: 0.0,
            b: 2.0,
            c: -3.0,
            d: 0.0,
            tx: 7.0,
            ty: -9.0,
        };
        let t =
            animate_z_rotation(model, None, None, Some(std::f32::consts::FRAC_PI_2), 1.0).unwrap();
        close(t.a, -2.0);
        close(t.b, 0.0);
        close(t.c, 0.0);
        close(t.d, -3.0);
        close(t.tx, 7.0);
        close(t.ty, -9.0);
        let t =
            animate_z_rotation(model, Some(0.0), Some(std::f32::consts::PI), None, 0.0).unwrap();
        close(t.a, 2.0);
        close(t.d, 3.0);
    }
    #[test]
    fn rotation_rejects_unsupported_matrices_and_nonfinite_values() {
        let mut t = CGAffineTransformIdentity;
        t.c = 0.5;
        assert!(animate_z_rotation(t, Some(0.0), Some(1.0), None, 0.5).is_err());
        t = CGAffineTransformIdentity;
        t.d = -1.0;
        assert!(animate_z_rotation(t, Some(0.0), Some(1.0), None, 0.5).is_err());
        assert!(
            animate_z_rotation(CGAffineTransformIdentity, Some(f32::NAN), None, None, 0.5).is_err()
        );
    }
    #[test]
    fn completed_forward_repeat_keeps_endpoint_and_autoreverse_returns_to_start() {
        close(animation_progress(1.0, 1.0, false), 1.0);
        close(animation_progress(2.0, 2.0, false), 1.0);
        close(animation_progress(1.0, 1.0, true), 0.0);
        close(animation_progress(1.5, 1.5, true), 1.0);
        close(animation_progress(1.0, 2.0, false), 0.0);
        close(animation_progress(0.25, f32::INFINITY, false), 0.25);
    }
}
