/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Minimal AArch64 assembler for the UIKit image's owned thunks. Only the
//! handful of encodings used there; each was checked against capstone
//! (see the test below for the exact words).

pub(in crate::a64) const SP: u32 = 31;
pub(in crate::a64) const FP: u32 = 29;
pub(in crate::a64) const LR: u32 = 30;

#[derive(Clone, Copy)]
pub(in crate::a64) struct Label(usize);

enum Fixup {
    Literal { at: usize, register: u32, literal: usize },
    Branch { at: usize, label: Label },
    Cbz { at: usize, register: u32, label: Label },
}

#[derive(Default)]
pub(in crate::a64) struct Asm {
    words: Vec<u32>,
    literals: Vec<u64>,
    labels: Vec<Option<usize>>,
    fixups: Vec<Fixup>,
}

impl Asm {
    pub(in crate::a64) fn raw(&mut self, word: u32) -> &mut Self {
        self.words.push(word);
        self
    }
    pub(in crate::a64) fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }
    pub(in crate::a64) fn bind(&mut self, label: Label) {
        self.labels[label.0] = Some(self.words.len());
    }
    /// `ldr xT, =value` from the literal pool appended after the code.
    pub(in crate::a64) fn ldr_literal(&mut self, register: u32, value: u64) -> &mut Self {
        self.literals.push(value);
        self.fixups.push(Fixup::Literal {
            at: self.words.len(),
            register,
            literal: self.literals.len() - 1,
        });
        self.raw(0x5800_0000)
    }
    /// `ldr xT, =slot; ldr xT, [xT]` (load through a bound data slot).
    pub(in crate::a64) fn ldr_indirect(&mut self, register: u32, slot: u64) -> &mut Self {
        self.ldr_literal(register, slot);
        self.ldr(register, register, 0)
    }
    pub(in crate::a64) fn b(&mut self, label: Label) -> &mut Self {
        self.fixups.push(Fixup::Branch { at: self.words.len(), label });
        self.raw(0x1400_0000)
    }
    pub(in crate::a64) fn cbz(&mut self, register: u32, label: Label) -> &mut Self {
        self.fixups.push(Fixup::Cbz { at: self.words.len(), register, label });
        self.raw(0xb400_0000)
    }
    pub(in crate::a64) fn blr(&mut self, n: u32) -> &mut Self {
        self.raw(0xd63f_0000 | n << 5)
    }
    pub(in crate::a64) fn br(&mut self, n: u32) -> &mut Self {
        self.raw(0xd61f_0000 | n << 5)
    }
    pub(in crate::a64) fn ret(&mut self) -> &mut Self {
        self.raw(0xd65f_03c0)
    }
    pub(in crate::a64) fn movz(&mut self, d: u32, value: u16) -> &mut Self {
        self.raw(0xd280_0000 | (value as u32) << 5 | d)
    }
    /// `mov xD, xM` (ORR with XZR; not valid for SP).
    pub(in crate::a64) fn mov(&mut self, d: u32, m: u32) -> &mut Self {
        self.raw(0xaa00_03e0 | m << 16 | d)
    }
    /// `mov xD, sp` / `add xD, xN, #imm`.
    pub(in crate::a64) fn add_imm(&mut self, d: u32, n: u32, imm: u32) -> &mut Self {
        assert!(imm < 4096);
        self.raw(0x9100_0000 | imm << 10 | n << 5 | d)
    }
    pub(in crate::a64) fn ldr(&mut self, t: u32, n: u32, offset: u32) -> &mut Self {
        assert!(offset % 8 == 0 && offset / 8 < 4096);
        self.raw(0xf940_0000 | (offset / 8) << 10 | n << 5 | t)
    }
    pub(in crate::a64) fn str(&mut self, t: u32, n: u32, offset: u32) -> &mut Self {
        assert!(offset % 8 == 0 && offset / 8 < 4096);
        self.raw(0xf900_0000 | (offset / 8) << 10 | n << 5 | t)
    }
    pub(in crate::a64) fn str_d(&mut self, t: u32, n: u32, offset: u32) -> &mut Self {
        assert!(offset % 8 == 0 && offset / 8 < 4096);
        self.raw(0xfd00_0000 | (offset / 8) << 10 | n << 5 | t)
    }
    fn pair(base: u32, t: u32, t2: u32, n: u32, offset: i32) -> u32 {
        assert!(offset % 8 == 0 && (-512..512).contains(&offset));
        base | ((offset / 8) as u32 & 0x7f) << 15 | t2 << 10 | n << 5 | t
    }
    pub(in crate::a64) fn stp(&mut self, t: u32, t2: u32, n: u32, offset: i32) -> &mut Self {
        self.raw(Self::pair(0xa900_0000, t, t2, n, offset))
    }
    pub(in crate::a64) fn ldp(&mut self, t: u32, t2: u32, n: u32, offset: i32) -> &mut Self {
        self.raw(Self::pair(0xa940_0000, t, t2, n, offset))
    }
    pub(in crate::a64) fn ldp_d(&mut self, t: u32, t2: u32, n: u32, offset: i32) -> &mut Self {
        self.raw(Self::pair(0x6d40_0000, t, t2, n, offset))
    }
    /// `stp x29, x30, [sp, #-frame]!` ; `mov x29, sp`
    pub(in crate::a64) fn prologue(&mut self, frame: i32) -> &mut Self {
        self.raw(Self::pair(0xa980_0000, FP, LR, SP, -frame));
        self.add_imm(FP, SP, 0)
    }
    /// `ldp x29, x30, [sp], #frame` ; `ret`
    pub(in crate::a64) fn epilogue(&mut self, frame: i32) -> &mut Self {
        self.raw(Self::pair(0xa8c0_0000, FP, LR, SP, frame));
        self.ret()
    }
    /// `and xD, xN, #0xffffffff8` (objc4 arm64 non-ptrauth ISA_MASK).
    pub(in crate::a64) fn and_isa_mask(&mut self, d: u32, n: u32) -> &mut Self {
        self.raw(0x927d_8000 | n << 5 | d)
    }

    /// Resolve labels and literals; the literal pool is 8-byte aligned when
    /// the code is placed at an 8-byte aligned address.
    pub(in crate::a64) fn finish(&self) -> Vec<u8> {
        let mut words = self.words.clone();
        if words.len() % 2 != 0 {
            words.push(0xd503_201f); // nop
        }
        let pool = words.len();
        for fixup in &self.fixups {
            match *fixup {
                Fixup::Literal { at, register, literal } => {
                    let delta = (pool + literal * 2 - at) as u32;
                    words[at] |= (delta & 0x7ffff) << 5 | register;
                }
                Fixup::Branch { at, label } => {
                    let target = self.labels[label.0].expect("unbound label") as i64;
                    words[at] |= ((target - at as i64) as u32) & 0x3ff_ffff;
                }
                Fixup::Cbz { at, register, label } => {
                    let target = self.labels[label.0].expect("unbound label") as i64;
                    words[at] |= (((target - at as i64) as u32) & 0x7ffff) << 5 | register;
                }
            }
        }
        let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        for value in &self.literals {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(asm: &Asm) -> Vec<u32> {
        asm.finish()
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect()
    }
    #[test]
    fn encodings_match_capstone_verified_words() {
        let mut a = Asm::default();
        a.and_isa_mask(9, 9)
            .movz(17, 5)
            .mov(19, 0)
            .mov(0, 19)
            .ldp(0, 1, 19, 0)
            .ldp(6, 7, 19, 48)
            .ldp_d(0, 1, 19, 64)
            .ldp_d(2, 3, 19, 80)
            .stp(0, 1, 19, 104)
            .str_d(0, 19, 120)
            .ldr(16, 19, 96)
            .stp(19, 20, SP, 16)
            .ldp(19, 20, SP, 16)
            .br(16)
            .ldr(9, 0, 0)
            .mov(1, 0)
            .ldr(0, SP, 16)
            .prologue(32)
            .epilogue(32);
        assert_eq!(
            words(&a),
            vec![
                0x927d8129, 0xd28000b1, 0xaa0003f3, 0xaa1303e0, 0xa9400660, 0xa9431e66, 0x6d440660,
                0x6d450e62, 0xa9068660, 0xfd003e60, 0xf9403270, 0xa90153f3, 0xa94153f3, 0xd61f0200,
                0xf9400009, 0xaa0003e1, 0xf9400be0, 0xa9be7bfd, 0x910003fd, 0xa8c27bfd, 0xd65f03c0,
                0xd503201f,
            ]
        );
    }
    #[test]
    fn labels_and_literals_resolve() {
        let mut a = Asm::default();
        let top = a.label();
        let done = a.label();
        a.bind(top);
        a.cbz(0, done); // +2 words
        a.b(top); // -1 word
        a.bind(done);
        a.ldr_literal(16, 0x1122_3344_5566_7788);
        let w = words(&a);
        assert_eq!(w[0], 0xb4000040);
        assert_eq!(w[1], 0x17ffffff);
        // literal at word 4 (after nop pad), ldr at word 2: delta 2 words.
        assert_eq!(w[2], 0x58000050);
        assert_eq!(&a.finish()[16..24], &0x1122_3344_5566_7788u64.to_le_bytes());
    }
}
