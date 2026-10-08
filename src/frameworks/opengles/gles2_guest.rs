/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Wrapper functions exposing the parts of OpenGL ES 2.0 that are not shared
//! with OpenGL ES 1.1 to the guest. See [super::gles_guest] for the shared
//! parts and for notes on the style of these wrappers.
//!
//! ES 2.0's core framebuffer and renderbuffer functions have identical
//! semantics to ES 1.1's `OES_framebuffer_object` ones, so these wrappers
//! just call the same [GLES] methods.

use touchHLE_gl_bindings::gles11::ARRAY_BUFFER_BINDING;

use super::gles_guest::{translate_pointer_or_offset_to_host, with_ctx_and_mem};
use crate::dyld::{export_c_func, FunctionExports};
use crate::gles::gles11_raw as gles11; // constants only
use crate::gles::gles11_raw::types::{GLboolean, GLchar, GLenum, GLfloat, GLint, GLsizei, GLuint};
use crate::mem::{ConstPtr, ConstVoidPtr, GuestUSize, MutPtr, Ptr};
use crate::Environment;

// Shaders and programs

fn glCreateShader(env: &mut Environment, type_: GLenum) -> GLuint {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.CreateShader(type_) })
}
fn glDeleteShader(env: &mut Environment, shader: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.DeleteShader(shader) })
}
fn glShaderSource(
    env: &mut Environment,
    shader: GLuint,
    count: GLsizei,
    string: ConstPtr<ConstPtr<u8>>, // const GLchar *const *
    length: ConstPtr<GLint>,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        // The guest passes an array of guest pointers; the host driver needs
        // an array of host pointers. The strings themselves can stay where
        // they are, since guest memory is directly addressable by the host.
        let count_usize: usize = count.try_into().unwrap();
        let mut host_strings: Vec<*const GLchar> = Vec::with_capacity(count_usize);
        for i in 0..count_usize {
            let element: ConstPtr<ConstPtr<u8>> =
                Ptr::from_bits(string.to_bits() + (i as GuestUSize) * 4);
            let guest_string: ConstPtr<u8> = mem.read(element);
            host_strings.push(mem.ptr_at(guest_string, 1).cast());
        }
        let host_length = if length.is_null() {
            std::ptr::null()
        } else {
            mem.ptr_at(length, count_usize as GuestUSize)
        };
        gles.ShaderSource(shader, count, host_strings.as_ptr(), host_length)
    })
}
fn glCompileShader(env: &mut Environment, shader: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.CompileShader(shader) })
}
fn glGetShaderiv(env: &mut Environment, shader: GLuint, pname: GLenum, params: MutPtr<GLint>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.GetShaderiv(shader, pname, mem.ptr_at_mut(params, 1))
    })
}
fn glGetShaderInfoLog(
    env: &mut Environment,
    shader: GLuint,
    buf_size: GLsizei,
    length: MutPtr<GLsizei>,
    info_log: MutPtr<u8>,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        let host_length = if length.is_null() {
            std::ptr::null_mut()
        } else {
            mem.ptr_at_mut(length, 1)
        };
        let host_log = mem.ptr_at_mut(info_log, buf_size.try_into().unwrap());
        gles.GetShaderInfoLog(shader, buf_size, host_length, host_log.cast())
    })
}
fn glCreateProgram(env: &mut Environment) -> GLuint {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.CreateProgram() })
}
fn glDeleteProgram(env: &mut Environment, program: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.DeleteProgram(program) })
}
fn glAttachShader(env: &mut Environment, program: GLuint, shader: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.AttachShader(program, shader)
    })
}
fn glDetachShader(env: &mut Environment, program: GLuint, shader: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.DetachShader(program, shader)
    })
}
fn glLinkProgram(env: &mut Environment, program: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.LinkProgram(program) })
}
fn glValidateProgram(env: &mut Environment, program: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.ValidateProgram(program) })
}
fn glUseProgram(env: &mut Environment, program: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.UseProgram(program) })
}
fn glGetProgramiv(env: &mut Environment, program: GLuint, pname: GLenum, params: MutPtr<GLint>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.GetProgramiv(program, pname, mem.ptr_at_mut(params, 1))
    })
}
fn glGetProgramInfoLog(
    env: &mut Environment,
    program: GLuint,
    buf_size: GLsizei,
    length: MutPtr<GLsizei>,
    info_log: MutPtr<u8>,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        let host_length = if length.is_null() {
            std::ptr::null_mut()
        } else {
            mem.ptr_at_mut(length, 1)
        };
        let host_log = mem.ptr_at_mut(info_log, buf_size.try_into().unwrap());
        gles.GetProgramInfoLog(program, buf_size, host_length, host_log.cast())
    })
}
fn glBindAttribLocation(env: &mut Environment, program: GLuint, index: GLuint, name: ConstPtr<u8>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.BindAttribLocation(program, index, mem.ptr_at(name, 1).cast())
    })
}
fn glGetAttribLocation(env: &mut Environment, program: GLuint, name: ConstPtr<u8>) -> GLint {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.GetAttribLocation(program, mem.ptr_at(name, 1).cast())
    })
}
fn glGetUniformLocation(env: &mut Environment, program: GLuint, name: ConstPtr<u8>) -> GLint {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.GetUniformLocation(program, mem.ptr_at(name, 1).cast())
    })
}

// Uniforms

fn glUniform1i(env: &mut Environment, location: GLint, v0: GLint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.Uniform1i(location, v0) })
}
fn glUniform1f(env: &mut Environment, location: GLint, v0: GLfloat) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.Uniform1f(location, v0) })
}
fn glUniform2f(env: &mut Environment, location: GLint, v0: GLfloat, v1: GLfloat) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.Uniform2f(location, v0, v1)
    })
}
fn glUniform3f(env: &mut Environment, location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.Uniform3f(location, v0, v1, v2)
    })
}
fn glUniform4f(
    env: &mut Environment,
    location: GLint,
    v0: GLfloat,
    v1: GLfloat,
    v2: GLfloat,
    v3: GLfloat,
) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.Uniform4f(location, v0, v1, v2, v3)
    })
}

/// Number of `GLfloat`s that need to be readable for a uniform array of
/// `count` elements, each made of `components` floats.
fn uniform_float_count(count: GLsizei, components: GuestUSize) -> GuestUSize {
    GuestUSize::try_from(count).unwrap() * components
}

fn glUniform1fv(env: &mut Environment, location: GLint, count: GLsizei, value: ConstPtr<GLfloat>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.Uniform1fv(
            location,
            count,
            mem.ptr_at(value, uniform_float_count(count, 1)),
        )
    })
}
fn glUniform2fv(env: &mut Environment, location: GLint, count: GLsizei, value: ConstPtr<GLfloat>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.Uniform2fv(
            location,
            count,
            mem.ptr_at(value, uniform_float_count(count, 2)),
        )
    })
}
fn glUniform3fv(env: &mut Environment, location: GLint, count: GLsizei, value: ConstPtr<GLfloat>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.Uniform3fv(
            location,
            count,
            mem.ptr_at(value, uniform_float_count(count, 3)),
        )
    })
}
fn glUniform4fv(env: &mut Environment, location: GLint, count: GLsizei, value: ConstPtr<GLfloat>) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        gles.Uniform4fv(
            location,
            count,
            mem.ptr_at(value, uniform_float_count(count, 4)),
        )
    })
}
fn glUniformMatrix2fv(
    env: &mut Environment,
    location: GLint,
    count: GLsizei,
    transpose: GLboolean,
    value: ConstPtr<GLfloat>,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        let value = mem.ptr_at(value, uniform_float_count(count, 4));
        gles.UniformMatrix2fv(location, count, transpose, value)
    })
}
fn glUniformMatrix3fv(
    env: &mut Environment,
    location: GLint,
    count: GLsizei,
    transpose: GLboolean,
    value: ConstPtr<GLfloat>,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        let value = mem.ptr_at(value, uniform_float_count(count, 9));
        gles.UniformMatrix3fv(location, count, transpose, value)
    })
}
fn glUniformMatrix4fv(
    env: &mut Environment,
    location: GLint,
    count: GLsizei,
    transpose: GLboolean,
    value: ConstPtr<GLfloat>,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        let value = mem.ptr_at(value, uniform_float_count(count, 16));
        gles.UniformMatrix4fv(location, count, transpose, value)
    })
}

// Vertex attributes

fn glEnableVertexAttribArray(env: &mut Environment, index: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.EnableVertexAttribArray(index)
    })
}
fn glDisableVertexAttribArray(env: &mut Environment, index: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.DisableVertexAttribArray(index)
    })
}
fn glVertexAttribPointer(
    env: &mut Environment,
    index: GLuint,
    size: GLint,
    type_: GLenum,
    normalized: GLboolean,
    stride: GLsizei,
    pointer: ConstVoidPtr,
) {
    with_ctx_and_mem(env, |gles, mem| unsafe {
        // Either an offset into the bound buffer, or a pointer to client-side
        // data in guest memory (which the driver reads at draw time).
        let pointer = translate_pointer_or_offset_to_host(gles, mem, pointer, ARRAY_BUFFER_BINDING);
        gles.VertexAttribPointer(index, size, type_, normalized, stride, pointer)
    })
}

// Blending

fn glBlendFuncSeparate(
    env: &mut Environment,
    src_rgb: GLenum,
    dst_rgb: GLenum,
    src_alpha: GLenum,
    dst_alpha: GLenum,
) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.BlendFuncSeparate(src_rgb, dst_rgb, src_alpha, dst_alpha)
    })
}

// Framebuffers and renderbuffers. These are the ES 2.0 core names of the
// functions that ES 1.1 only has as OES_framebuffer_object extension ones.

fn glGenFramebuffers(env: &mut Environment, n: GLsizei, framebuffers: MutPtr<GLuint>) {
    with_ctx_and_mem(env, |gles, mem| {
        let framebuffers = mem.ptr_at_mut(framebuffers, n.try_into().unwrap());
        unsafe { gles.GenFramebuffersOES(n, framebuffers) }
    })
}
fn glGenRenderbuffers(env: &mut Environment, n: GLsizei, renderbuffers: MutPtr<GLuint>) {
    with_ctx_and_mem(env, |gles, mem| {
        let renderbuffers = mem.ptr_at_mut(renderbuffers, n.try_into().unwrap());
        unsafe { gles.GenRenderbuffersOES(n, renderbuffers) }
    })
}
fn glIsFramebuffer(env: &mut Environment, framebuffer: GLuint) -> GLboolean {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.IsFramebufferOES(framebuffer)
    })
}
fn glIsRenderbuffer(env: &mut Environment, renderbuffer: GLuint) -> GLboolean {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.IsRenderbufferOES(renderbuffer)
    })
}
fn glBindFramebuffer(env: &mut Environment, target: GLenum, framebuffer: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.BindFramebufferOES(target, framebuffer)
    })
}
fn glBindRenderbuffer(env: &mut Environment, target: GLenum, renderbuffer: GLuint) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.BindRenderbufferOES(target, renderbuffer)
    })
}
fn glRenderbufferStorage(
    env: &mut Environment,
    target: GLenum,
    internalformat: GLenum,
    width: GLsizei,
    height: GLsizei,
) {
    // apply scale hack: give the app a larger framebuffer than it asked for
    let factor = env.options.scale_hack.get() as GLsizei;
    let (width, height) = (width * factor, height * factor);
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.RenderbufferStorageOES(target, internalformat, width, height)
    })
}
fn glFramebufferRenderbuffer(
    env: &mut Environment,
    target: GLenum,
    attachment: GLenum,
    renderbuffertarget: GLenum,
    renderbuffer: GLuint,
) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.FramebufferRenderbufferOES(target, attachment, renderbuffertarget, renderbuffer)
    })
}
fn glFramebufferTexture2D(
    env: &mut Environment,
    target: GLenum,
    attachment: GLenum,
    textarget: GLenum,
    texture: GLuint,
    level: i32,
) {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.FramebufferTexture2DOES(target, attachment, textarget, texture, level)
    })
}
fn glGetFramebufferAttachmentParameteriv(
    env: &mut Environment,
    target: GLenum,
    attachment: GLenum,
    pname: GLenum,
    params: MutPtr<GLint>,
) {
    with_ctx_and_mem(env, |gles, mem| {
        let params = mem.ptr_at_mut(params, 1);
        unsafe { gles.GetFramebufferAttachmentParameterivOES(target, attachment, pname, params) }
    })
}
fn glGetRenderbufferParameteriv(
    env: &mut Environment,
    target: GLenum,
    pname: GLenum,
    params: MutPtr<GLint>,
) {
    let factor = env.options.scale_hack.get() as GLint;
    with_ctx_and_mem(env, |gles, mem| {
        let params = mem.ptr_at_mut(params, 1);
        unsafe { gles.GetRenderbufferParameterivOES(target, pname, params) };
        // apply scale hack: scale down the reported size of the framebuffer,
        // assuming the framebuffer's true size is larger than it should be
        if pname == gles11::RENDERBUFFER_WIDTH_OES || pname == gles11::RENDERBUFFER_HEIGHT_OES {
            unsafe { params.write_unaligned(params.read_unaligned() / factor) }
        }
    })
}
fn glCheckFramebufferStatus(env: &mut Environment, target: GLenum) -> GLenum {
    with_ctx_and_mem(env, |gles, _mem| unsafe {
        gles.CheckFramebufferStatusOES(target)
    })
}
fn glDeleteFramebuffers(env: &mut Environment, n: GLsizei, framebuffers: ConstPtr<GLuint>) {
    with_ctx_and_mem(env, |gles, mem| {
        let framebuffers = mem.ptr_at(framebuffers, n.try_into().unwrap());
        unsafe { gles.DeleteFramebuffersOES(n, framebuffers) }
    })
}
fn glDeleteRenderbuffers(env: &mut Environment, n: GLsizei, renderbuffers: ConstPtr<GLuint>) {
    with_ctx_and_mem(env, |gles, mem| {
        let renderbuffers = mem.ptr_at(renderbuffers, n.try_into().unwrap());
        unsafe { gles.DeleteRenderbuffersOES(n, renderbuffers) }
    })
}
fn glGenerateMipmap(env: &mut Environment, target: GLenum) {
    with_ctx_and_mem(env, |gles, _mem| unsafe { gles.GenerateMipmapOES(target) })
}

pub const FUNCTIONS: FunctionExports = &[
    // Shaders and programs
    export_c_func!(glCreateShader(_)),
    export_c_func!(glDeleteShader(_)),
    export_c_func!(glShaderSource(_, _, _, _)),
    export_c_func!(glCompileShader(_)),
    export_c_func!(glGetShaderiv(_, _, _)),
    export_c_func!(glGetShaderInfoLog(_, _, _, _)),
    export_c_func!(glCreateProgram()),
    export_c_func!(glDeleteProgram(_)),
    export_c_func!(glAttachShader(_, _)),
    export_c_func!(glDetachShader(_, _)),
    export_c_func!(glLinkProgram(_)),
    export_c_func!(glValidateProgram(_)),
    export_c_func!(glUseProgram(_)),
    export_c_func!(glGetProgramiv(_, _, _)),
    export_c_func!(glGetProgramInfoLog(_, _, _, _)),
    export_c_func!(glBindAttribLocation(_, _, _)),
    export_c_func!(glGetAttribLocation(_, _)),
    export_c_func!(glGetUniformLocation(_, _)),
    // Uniforms
    export_c_func!(glUniform1i(_, _)),
    export_c_func!(glUniform1f(_, _)),
    export_c_func!(glUniform2f(_, _, _)),
    export_c_func!(glUniform3f(_, _, _, _)),
    export_c_func!(glUniform4f(_, _, _, _, _)),
    export_c_func!(glUniform1fv(_, _, _)),
    export_c_func!(glUniform2fv(_, _, _)),
    export_c_func!(glUniform3fv(_, _, _)),
    export_c_func!(glUniform4fv(_, _, _)),
    export_c_func!(glUniformMatrix2fv(_, _, _, _)),
    export_c_func!(glUniformMatrix3fv(_, _, _, _)),
    export_c_func!(glUniformMatrix4fv(_, _, _, _)),
    // Vertex attributes
    export_c_func!(glEnableVertexAttribArray(_)),
    export_c_func!(glDisableVertexAttribArray(_)),
    export_c_func!(glVertexAttribPointer(_, _, _, _, _, _)),
    // Blending
    export_c_func!(glBlendFuncSeparate(_, _, _, _)),
    // Framebuffers and renderbuffers
    export_c_func!(glGenFramebuffers(_, _)),
    export_c_func!(glGenRenderbuffers(_, _)),
    export_c_func!(glIsFramebuffer(_)),
    export_c_func!(glIsRenderbuffer(_)),
    export_c_func!(glBindFramebuffer(_, _)),
    export_c_func!(glBindRenderbuffer(_, _)),
    export_c_func!(glRenderbufferStorage(_, _, _, _)),
    export_c_func!(glFramebufferRenderbuffer(_, _, _, _)),
    export_c_func!(glFramebufferTexture2D(_, _, _, _, _)),
    export_c_func!(glGetFramebufferAttachmentParameteriv(_, _, _, _)),
    export_c_func!(glGetRenderbufferParameteriv(_, _, _)),
    export_c_func!(glCheckFramebufferStatus(_)),
    export_c_func!(glDeleteFramebuffers(_, _)),
    export_c_func!(glDeleteRenderbuffers(_, _)),
    export_c_func!(glGenerateMipmap(_)),
];
