/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! CommonCrypto and friends

use crate::dyld::FunctionExports;
use crate::mem::{ConstVoidPtr, GuestUSize, MutPtr, MutVoidPtr};
use crate::{export_c_func, Environment};
use aes::{Aes128, Aes192, Aes256};
use cbc::cipher::block_padding::{NoPadding, Pkcs7};
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use digest::Digest;
use md5::Md5;
use sha1::Sha1;
use sha2::Sha256;
use std::collections::HashMap;

// TODO: struct definition
#[allow(non_camel_case_types)]
struct CC_MD5_CTX {}

#[derive(Default)]
pub struct State {
    md5_contexts: HashMap<MutPtr<CC_MD5_CTX>, Md5>,
}
impl State {
    fn get_mut(env: &mut Environment) -> &mut Self {
        &mut env.libc_state.crypto
    }
}

fn CC_MD5_Init(env: &mut Environment, ctx: MutPtr<CC_MD5_CTX>) -> i32 {
    log_once!("Warning: CC_MD5_Init doesn't update side effects! (internal changes to CC_MD5_CTX are not done)");
    assert!(!State::get_mut(env).md5_contexts.contains_key(&ctx));
    State::get_mut(env).md5_contexts.insert(ctx, Md5::new());
    1 // success
}

fn CC_MD5_Update(
    env: &mut Environment,
    ctx: MutPtr<CC_MD5_CTX>,
    data: ConstVoidPtr,
    len: u32,
) -> i32 {
    log_once!("Warning: CC_MD5_Update doesn't update side effects! (internal changes to CC_MD5_CTX are not done)");
    let hasher = env.libc_state.crypto.md5_contexts.get_mut(&ctx).unwrap();
    hasher.update(env.mem.bytes_at(data.cast(), len));
    1 // success
}

fn CC_MD5_Final(env: &mut Environment, md: MutPtr<u8>, ctx: MutPtr<CC_MD5_CTX>) -> i32 {
    log_once!("Warning: CC_MD5_Final doesn't update side effects! (internal changes to CC_MD5_CTX are not done)");
    let hasher = State::get_mut(env).md5_contexts.remove(&ctx).unwrap();
    let digest = hasher.finalize();
    env.mem.bytes_at_mut(md, 16).copy_from_slice(&digest[..]);
    1 // success
}

fn CC_MD5(env: &mut Environment, data: ConstVoidPtr, len: u32, md: MutPtr<u8>) -> MutPtr<u8> {
    let mut hasher = Md5::new();
    hasher.update(env.mem.bytes_at(data.cast(), len));
    let digest = hasher.finalize();
    env.mem.bytes_at_mut(md, 16).copy_from_slice(&digest[..]);
    md
}

fn CC_SHA256(env: &mut Environment, data: ConstVoidPtr, len: u32, md: MutPtr<u8>) -> MutPtr<u8> {
    let mut hasher = Sha256::new();
    hasher.update(env.mem.bytes_at(data.cast(), len));
    let digest = hasher.finalize();
    env.mem.bytes_at_mut(md, 32).copy_from_slice(&digest[..]);
    md
}

fn CC_SHA1(env: &mut Environment, data: ConstVoidPtr, len: u32, md: MutPtr<u8>) -> MutPtr<u8> {
    let mut hasher = Sha1::new();
    hasher.update(env.mem.bytes_at(data.cast(), len));
    let digest = hasher.finalize();
    env.mem.bytes_at_mut(md, 20).copy_from_slice(&digest[..]);
    md
}

// CCCrypt() status codes
const kCCSuccess: i32 = 0;
const kCCParamError: i32 = -4300;
const kCCBufferTooSmall: i32 = -4301;
const kCCAlignmentError: i32 = -4303;
const kCCDecodeError: i32 = -4304;
const kCCUnimplemented: i32 = -4305;

const kCCEncrypt: u32 = 0;
const kCCDecrypt: u32 = 1;
const kCCAlgorithmAES128: u32 = 0;
const kCCOptionPKCS7Padding: u32 = 1;
const kCCOptionECBMode: u32 = 2;

/// AES in CBC or ECB mode, with PKCS7 padding or none. The key size selects
/// AES-128, -192 or -256 (as CommonCrypto does, despite the "AES128" name).
fn aes_crypt(
    encrypt: bool,
    cbc_mode: bool,
    pkcs7: bool,
    key: &[u8],
    iv: &[u8; 16],
    data: &[u8],
) -> Result<Vec<u8>, i32> {
    macro_rules! run {
        ($aes:ty) => {{
            match (encrypt, cbc_mode, pkcs7) {
                (true, true, true) => cbc::Encryptor::<$aes>::new_from_slices(key, iv)
                    .map(|c| c.encrypt_padded_vec_mut::<Pkcs7>(data))
                    .map_err(|_| kCCParamError),
                (true, true, false) => cbc::Encryptor::<$aes>::new_from_slices(key, iv)
                    .map(|c| c.encrypt_padded_vec_mut::<NoPadding>(data))
                    .map_err(|_| kCCParamError),
                (true, false, true) => ecb::Encryptor::<$aes>::new_from_slice(key)
                    .map(|c| c.encrypt_padded_vec_mut::<Pkcs7>(data))
                    .map_err(|_| kCCParamError),
                (true, false, false) => ecb::Encryptor::<$aes>::new_from_slice(key)
                    .map(|c| c.encrypt_padded_vec_mut::<NoPadding>(data))
                    .map_err(|_| kCCParamError),
                (false, true, true) => cbc::Decryptor::<$aes>::new_from_slices(key, iv)
                    .map_err(|_| kCCParamError)?
                    .decrypt_padded_vec_mut::<Pkcs7>(data)
                    .map_err(|_| kCCDecodeError),
                (false, true, false) => cbc::Decryptor::<$aes>::new_from_slices(key, iv)
                    .map_err(|_| kCCParamError)?
                    .decrypt_padded_vec_mut::<NoPadding>(data)
                    .map_err(|_| kCCDecodeError),
                (false, false, true) => ecb::Decryptor::<$aes>::new_from_slice(key)
                    .map_err(|_| kCCParamError)?
                    .decrypt_padded_vec_mut::<Pkcs7>(data)
                    .map_err(|_| kCCDecodeError),
                (false, false, false) => ecb::Decryptor::<$aes>::new_from_slice(key)
                    .map_err(|_| kCCParamError)?
                    .decrypt_padded_vec_mut::<NoPadding>(data)
                    .map_err(|_| kCCDecodeError),
            }
        }};
    }
    // Decrypting nothing yields nothing, padding or not.
    if !encrypt && data.is_empty() {
        return Ok(Vec::new());
    }
    // Without padding the data must be whole blocks.
    if !pkcs7 && data.len() % 16 != 0 {
        return Err(kCCAlignmentError);
    }
    match key.len() {
        16 => run!(Aes128),
        24 => run!(Aes192),
        32 => run!(Aes256),
        _ => Err(kCCParamError),
    }
}

/// Only AES is implemented (that is all `kCCAlgorithmAES128` covers; DES and
/// friends return `kCCUnimplemented`). A NULL IV means an all-zero IV.
fn CCCrypt(
    env: &mut Environment,
    op: u32,
    alg: u32,
    options: u32,
    key: ConstVoidPtr,
    key_length: GuestUSize,
    iv: ConstVoidPtr,
    data_in: ConstVoidPtr,
    data_in_length: GuestUSize,
    data_out: MutVoidPtr,
    data_out_available: GuestUSize,
    data_out_moved: MutPtr<GuestUSize>,
) -> i32 {
    if alg != kCCAlgorithmAES128 {
        log!("TODO: CCCrypt() algorithm {} is not implemented", alg);
        return kCCUnimplemented;
    }
    if op != kCCEncrypt && op != kCCDecrypt {
        return kCCParamError;
    }
    if key_length == 0 || key.is_null() {
        return kCCParamError;
    }
    let key_bytes = env.mem.bytes_at(key.cast(), key_length).to_vec();
    let mut iv_bytes = [0u8; 16];
    if !iv.is_null() {
        iv_bytes.copy_from_slice(env.mem.bytes_at(iv.cast(), 16));
    }
    // Empty input is legal (and then data_in may be NULL), so only read memory
    // when there is something to read.
    let input = if data_in_length == 0 {
        Vec::new()
    } else {
        env.mem.bytes_at(data_in.cast(), data_in_length).to_vec()
    };

    let result = aes_crypt(
        op == kCCEncrypt,
        options & kCCOptionECBMode == 0,
        options & kCCOptionPKCS7Padding != 0,
        &key_bytes,
        &iv_bytes,
        &input,
    );
    let output = match result {
        Ok(output) => output,
        Err(status) => return status,
    };
    if !data_out_moved.is_null() {
        env.mem.write(data_out_moved, output.len() as GuestUSize);
    }
    if (data_out_available as usize) < output.len() {
        return kCCBufferTooSmall;
    }
    if !output.is_empty() {
        env.mem
            .bytes_at_mut(data_out.cast(), output.len() as GuestUSize)
            .copy_from_slice(&output);
    }
    kCCSuccess
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(CC_MD5_Init(_)),
    export_c_func!(CC_MD5_Update(_, _, _)),
    export_c_func!(CC_MD5_Final(_, _)),
    export_c_func!(CC_MD5(_, _, _)),
    export_c_func!(CC_SHA1(_, _, _)),
    export_c_func!(CC_SHA256(_, _, _)),
    export_c_func!(CCCrypt(_, _, _, _, _, _, _, _, _, _, _)),
];
