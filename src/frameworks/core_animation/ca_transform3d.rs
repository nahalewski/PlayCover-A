/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
//! Core Animation's row-vector, 4-by-4 transform matrix.

use crate::abi::{impl_GuestRet_for_large_struct, GuestArg};
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::frameworks::core_graphics::CGFloat;
use crate::mem::SafeRead;
use crate::Environment;

/// Entries in ABI order m11, m12, ... m44. CGFloat is 32-bit in this runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct CATransform3D {
    pub rows: [[CGFloat; 4]; 4],
}
unsafe impl SafeRead for CATransform3D {}
impl GuestArg for CATransform3D {
    const REG_COUNT: usize = 16;
    fn from_regs(regs: &[u32]) -> Self {
        let mut result = CATransform3DIdentity;
        for (i, value) in result.rows.iter_mut().flatten().enumerate() {
            *value = CGFloat::from_regs(&regs[i..i + 1]);
        }
        result
    }
    fn to_regs(self, regs: &mut [u32]) {
        for (i, value) in self.rows.into_iter().flatten().enumerate() {
            value.to_regs(&mut regs[i..i + 1]);
        }
    }
}
impl_GuestRet_for_large_struct!(CATransform3D);

pub const CATransform3DIdentity: CATransform3D = CATransform3D {
    rows: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ],
};

impl CATransform3D {
    /// Apple's documented concatenation is a * b, in row-vector layout.
    pub fn concat(self, other: Self) -> Self {
        let mut rows = [[0.0; 4]; 4];
        for (i, row) in rows.iter_mut().enumerate() {
            for (j, value) in row.iter_mut().enumerate() {
                *value = (0..4).map(|k| self.rows[i][k] * other.rows[k][j]).sum();
            }
        }
        Self { rows }
    }

    pub fn make_rotation(angle: CGFloat, x: CGFloat, y: CGFloat, z: CGFloat) -> Self {
        let length = x.hypot(y).hypot(z);
        if length == 0.0 {
            return CATransform3DIdentity;
        }
        let (x, y, z) = (x / length, y / length, z / length);
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        Self {
            rows: [
                [t * x * x + c, t * x * y + s * z, t * x * z - s * y, 0.0],
                [t * x * y - s * z, t * y * y + c, t * y * z + s * x, 0.0],
                [t * x * z + s * y, t * y * z - s * x, t * z * z + c, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }
}

fn CATransform3DMakeRotation(
    _env: &mut Environment,
    angle: CGFloat,
    x: CGFloat,
    y: CGFloat,
    z: CGFloat,
) -> CATransform3D {
    CATransform3D::make_rotation(angle, x, y, z)
}
fn CATransform3DConcat(
    _env: &mut Environment,
    a: CATransform3D,
    b: CATransform3D,
) -> CATransform3D {
    a.concat(b)
}

pub const CONSTANTS: ConstantExports = &[(
    "_CATransform3DIdentity",
    HostConstant::Custom(|env| {
        env.mem
            .alloc_and_write(CATransform3DIdentity)
            .cast()
            .cast_const()
    }),
)];
pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CATransform3DMakeRotation(_, _, _, _)),
    export_c_func!(CATransform3DConcat(_, _)),
];

#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: CGFloat, b: CGFloat) {
        assert!((a - b).abs() < 0.00001, "{a} != {b}");
    }
    #[test]
    fn rotation_normalizes_axis_and_has_correct_orientation() {
        let r = CATransform3D::make_rotation(std::f32::consts::FRAC_PI_2, 0.0, 0.0, 5.0);
        near(r.rows[0][0], 0.0);
        near(r.rows[0][1], 1.0);
        near(r.rows[1][0], -1.0);
        assert_eq!(
            CATransform3D::make_rotation(1.0, 0.0, 0.0, 0.0),
            CATransform3DIdentity
        );
        let inverse = CATransform3D::make_rotation(-std::f32::consts::FRAC_PI_2, 0.0, 0.0, 1.0);
        for (a, b) in r
            .concat(inverse)
            .rows
            .into_iter()
            .flatten()
            .zip(CATransform3DIdentity.rows.into_iter().flatten())
        {
            near(a, b);
        }
    }
    #[test]
    fn concatenation_order_and_guest_abi_layout() {
        let mut translation = CATransform3DIdentity;
        translation.rows[3][0] = 7.0;
        let mut scale = CATransform3DIdentity;
        scale.rows[0][0] = 3.0;
        assert_eq!(translation.concat(scale).rows[3][0], 21.0);
        assert_eq!(scale.concat(translation).rows[3][0], 7.0);
        let mut regs = [0; 16];
        translation.to_regs(&mut regs);
        assert_eq!(regs[12], 7.0f32.to_bits());
        assert_eq!(CATransform3D::from_regs(&regs), translation);
        assert_eq!(std::mem::size_of::<CATransform3D>(), 64);
    }
}
