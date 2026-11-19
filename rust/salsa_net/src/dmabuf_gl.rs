#![allow(unsafe_code)]
#![allow(non_camel_case_types)]
#![allow(clippy::missing_transmute_annotations)]

use core::ffi::{c_char, c_int, c_void};
use core::ptr;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;

use crate::pw_wire::{
    DRM_MOD_INVALID, VF_ABGR, VF_ARGB, VF_BGRA, VF_BGRX, VF_I420, VF_NV12, VF_RGBA, VF_RGBX,
    VF_XBGR, VF_XRGB, VF_YUY2,
};
use crate::unix_sys::{dl_close, dl_open_any, dl_sym};

const O_RDWR_CLOEXEC: c_int = 2 | 0o2000000;

const EGL_NONE: i32 = 0x3038;
const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;
const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_LINEAR: i32 = 0x2601;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
const GL_FLOAT: u32 = 0x1406;

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// Maps a SPA video format enum into the corresponding little-endian Linux DRM FourCC.
pub fn spa_vfmt_to_drm_fourcc(fmt: u32) -> Option<u32> {
    match fmt {
        VF_BGRX => Some(fourcc(b'X', b'R', b'2', b'4')),
        VF_BGRA => Some(fourcc(b'A', b'R', b'2', b'4')),
        VF_RGBX => Some(fourcc(b'X', b'B', b'2', b'4')),
        VF_RGBA => Some(fourcc(b'A', b'B', b'2', b'4')),
        VF_XRGB => Some(fourcc(b'B', b'X', b'2', b'4')),
        VF_XBGR => Some(fourcc(b'R', b'X', b'2', b'4')),
        VF_ARGB => Some(fourcc(b'B', b'A', b'2', b'4')),
        VF_ABGR => Some(fourcc(b'R', b'A', b'2', b'4')),
        VF_YUY2 => Some(fourcc(b'Y', b'U', b'Y', b'V')),
        VF_NV12 => Some(fourcc(b'N', b'V', b'1', b'2')),
        VF_I420 => Some(fourcc(b'Y', b'U', b'1', b'2')),
        _ => None,
    }
}

type GbmCreateDeviceFn = unsafe extern "C" fn(fd: c_int) -> *mut c_void;
type GbmDeviceDestroyFn = unsafe extern "C" fn(gbm: *mut c_void);

type EglGetProcAddressFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;
type EglInitializeFn = unsafe extern "C" fn(dpy: *mut c_void, maj: *mut i32, min: *mut i32) -> u32;
type EglTerminateFn = unsafe extern "C" fn(dpy: *mut c_void) -> u32;
type EglBindApiFn = unsafe extern "C" fn(api: u32) -> u32;
type EglChooseConfigFn =
    unsafe extern "C" fn(dpy: *mut c_void, attr: *const i32, cfg: *mut *mut c_void, sz: i32, n: *mut i32) -> u32;
type EglCreateContextFn =
    unsafe extern "C" fn(dpy: *mut c_void, cfg: *mut c_void, share: *mut c_void, attr: *const i32) -> *mut c_void;
type EglDestroyContextFn = unsafe extern "C" fn(dpy: *mut c_void, ctx: *mut c_void) -> u32;
type EglMakeCurrentFn =
    unsafe extern "C" fn(dpy: *mut c_void, draw: *mut c_void, read: *mut c_void, ctx: *mut c_void) -> u32;
type EglGetCurrentContextFn = unsafe extern "C" fn() -> *mut c_void;
type EglGetPlatformDisplayExtFn =
    unsafe extern "C" fn(platform: u32, native_display: *mut c_void, attr: *const i32) -> *mut c_void;
type EglCreateImageKhrFn =
    unsafe extern "C" fn(dpy: *mut c_void, ctx: *mut c_void, target: u32, buf: *mut c_void, attr: *const i32) -> *mut c_void;
type EglDestroyImageKhrFn = unsafe extern "C" fn(dpy: *mut c_void, image: *mut c_void) -> u32;
type GlEglImageTargetTexture2dOesFn = unsafe extern "C" fn(target: u32, image: *mut c_void);

type GlGenFn = unsafe extern "C" fn(n: i32, ids: *mut u32);
type GlDelFn = unsafe extern "C" fn(n: i32, ids: *const u32);
type GlBindFn = unsafe extern "C" fn(target: u32, id: u32);
type GlTexParamiFn = unsafe extern "C" fn(target: u32, pname: u32, param: i32);
type GlTexImage2dFn = unsafe extern "C" fn(
    target: u32,
    level: i32,
    internal: i32,
    w: i32,
    h: i32,
    border: i32,
    fmt: u32,
    tp: u32,
    px: *const c_void,
);
type GlFramebufferTexture2dFn =
    unsafe extern "C" fn(target: u32, attach: u32, textarget: u32, tex: u32, level: i32);
type GlCheckFramebufferStatusFn = unsafe extern "C" fn(target: u32) -> u32;
type GlCreateShaderFn = unsafe extern "C" fn(tp: u32) -> u32;
type GlDeleteShaderFn = unsafe extern "C" fn(sh: u32);
type GlShaderSourceFn = unsafe extern "C" fn(sh: u32, cnt: i32, src: *const *const c_char, len: *const i32);
type GlCompileShaderFn = unsafe extern "C" fn(sh: u32);
type GlGetShaderivFn = unsafe extern "C" fn(sh: u32, pname: u32, p: *mut i32);
type GlCreateProgramFn = unsafe extern "C" fn() -> u32;
type GlDeleteProgramFn = unsafe extern "C" fn(prog: u32);
type GlAttachShaderFn = unsafe extern "C" fn(prog: u32, sh: u32);
type GlLinkProgramFn = unsafe extern "C" fn(prog: u32);
type GlGetProgramivFn = unsafe extern "C" fn(prog: u32, pname: u32, p: *mut i32);
type GlUseProgramFn = unsafe extern "C" fn(prog: u32);
type GlGetAttribLocationFn = unsafe extern "C" fn(prog: u32, name: *const c_char) -> i32;
type GlEnableVertexAttribArrayFn = unsafe extern "C" fn(idx: u32);
type GlVertexAttribPointerFn = unsafe extern "C" fn(idx: u32, sz: i32, tp: u32, norm: u8, stride: i32, ptr: *const c_void);
type GlViewportFn = unsafe extern "C" fn(x: i32, y: i32, w: i32, h: i32);
type GlDrawArraysFn = unsafe extern "C" fn(mode: u32, first: i32, count: i32);
type GlFinishFn = unsafe extern "C" fn();
type GlReadPixelsFn =
    unsafe extern "C" fn(x: i32, y: i32, w: i32, h: i32, fmt: u32, tp: u32, data: *mut c_void);

struct GlApi {
    lib_gbm: *mut c_void,
    lib_egl: *mut c_void,
    lib_gles: *mut c_void,
    gbm_create_device: GbmCreateDeviceFn,
    gbm_device_destroy: GbmDeviceDestroyFn,
    egl_initialize: EglInitializeFn,
    egl_terminate: EglTerminateFn,
    egl_bind_api: EglBindApiFn,
    egl_choose_config: EglChooseConfigFn,
    egl_create_context: EglCreateContextFn,
    egl_destroy_context: EglDestroyContextFn,
    egl_make_current: EglMakeCurrentFn,
    egl_get_current_context: EglGetCurrentContextFn,
    egl_get_platform_display: EglGetPlatformDisplayExtFn,
    egl_create_image: EglCreateImageKhrFn,
    egl_destroy_image: EglDestroyImageKhrFn,
    gl_egl_image_tex: GlEglImageTargetTexture2dOesFn,
    gl_gen_textures: GlGenFn,
    gl_del_textures: GlDelFn,
    gl_bind_texture: GlBindFn,
    gl_tex_parami: GlTexParamiFn,
    gl_tex_image_2d: GlTexImage2dFn,
    gl_gen_fbos: GlGenFn,
    gl_del_fbos: GlDelFn,
    gl_bind_fbo: GlBindFn,
    gl_fbo_tex: GlFramebufferTexture2dFn,
    gl_chk_fbo: GlCheckFramebufferStatusFn,
    gl_create_shader: GlCreateShaderFn,
    gl_del_shader: GlDeleteShaderFn,
    gl_shader_src: GlShaderSourceFn,
    gl_compile_shader: GlCompileShaderFn,
    gl_get_shaderiv: GlGetShaderivFn,
    gl_create_program: GlCreateProgramFn,
    gl_del_program: GlDeleteProgramFn,
    gl_attach_shader: GlAttachShaderFn,
    gl_link_program: GlLinkProgramFn,
    gl_get_programiv: GlGetProgramivFn,
    gl_use_program: GlUseProgramFn,
    gl_get_attrib: GlGetAttribLocationFn,
    gl_en_attrib: GlEnableVertexAttribArrayFn,
    gl_vert_ptr: GlVertexAttribPointerFn,
    gl_viewport: GlViewportFn,
    gl_draw_arrays: GlDrawArraysFn,
    gl_finish: GlFinishFn,
    gl_read_pixels: GlReadPixelsFn,
}

impl Drop for GlApi {
    fn drop(&mut self) {
        unsafe {
            dl_close(self.lib_gles);
            dl_close(self.lib_egl);
            dl_close(self.lib_gbm);
        }
    }
}

unsafe fn load_sym<T: Copy>(h: *mut c_void, name: &str) -> Option<T> {
    let p = unsafe { dl_sym(h, name) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { core::mem::transmute_copy(&p) })
}

unsafe fn load_egl_ext<T: Copy>(gpa: EglGetProcAddressFn, name: &str) -> Option<T> {
    let c = CString::new(name).ok()?;
    let p = unsafe { gpa(c.as_ptr()) };
    if p.is_null() {
        return None;
    }
    Some(unsafe { core::mem::transmute_copy(&p) })
}

impl GlApi {
    fn load() -> Option<Self> {
        unsafe {
            let lib_gbm = dl_open_any(&["libgbm.so.1", "libgbm.so"]);
            let lib_egl = dl_open_any(&["libEGL.so.1", "libEGL.so"]);
            let lib_gles = dl_open_any(&["libGLESv2.so.2", "libGLESv2.so"]);
            if lib_gbm.is_null() || lib_egl.is_null() || lib_gles.is_null() {
                dl_close(lib_gles);
                dl_close(lib_egl);
                dl_close(lib_gbm);
                return None;
            }
            let gpa: EglGetProcAddressFn = match load_sym(lib_egl, "eglGetProcAddress") {
                Some(f) => f,
                None => {
                    dl_close(lib_gles);
                    dl_close(lib_egl);
                    dl_close(lib_gbm);
                    return None;
                }
            };
            let api = (|| -> Option<Self> {
                Some(Self {
                    lib_gbm,
                    lib_egl,
                    lib_gles,
                    gbm_create_device: load_sym(lib_gbm, "gbm_create_device")?,
                    gbm_device_destroy: load_sym(lib_gbm, "gbm_device_destroy")?,
                    egl_initialize: load_sym(lib_egl, "eglInitialize")?,
                    egl_terminate: load_sym(lib_egl, "eglTerminate")?,
                    egl_bind_api: load_sym(lib_egl, "eglBindAPI")?,
                    egl_choose_config: load_sym(lib_egl, "eglChooseConfig")?,
                    egl_create_context: load_sym(lib_egl, "eglCreateContext")?,
                    egl_destroy_context: load_sym(lib_egl, "eglDestroyContext")?,
                    egl_make_current: load_sym(lib_egl, "eglMakeCurrent")?,
                    egl_get_current_context: load_sym(lib_egl, "eglGetCurrentContext")?,
                    egl_get_platform_display: load_egl_ext(gpa, "eglGetPlatformDisplayEXT")?,
                    egl_create_image: load_egl_ext(gpa, "eglCreateImageKHR")?,
                    egl_destroy_image: load_egl_ext(gpa, "eglDestroyImageKHR")?,
                    gl_egl_image_tex: load_egl_ext(gpa, "glEGLImageTargetTexture2DOES")?,
                    gl_gen_textures: load_sym(lib_gles, "glGenTextures")?,
                    gl_del_textures: load_sym(lib_gles, "glDeleteTextures")?,
                    gl_bind_texture: load_sym(lib_gles, "glBindTexture")?,
                    gl_tex_parami: load_sym(lib_gles, "glTexParameteri")?,
                    gl_tex_image_2d: load_sym(lib_gles, "glTexImage2D")?,
                    gl_gen_fbos: load_sym(lib_gles, "glGenFramebuffers")?,
                    gl_del_fbos: load_sym(lib_gles, "glDeleteFramebuffers")?,
                    gl_bind_fbo: load_sym(lib_gles, "glBindFramebuffer")?,
                    gl_fbo_tex: load_sym(lib_gles, "glFramebufferTexture2D")?,
                    gl_chk_fbo: load_sym(lib_gles, "glCheckFramebufferStatus")?,
                    gl_create_shader: load_sym(lib_gles, "glCreateShader")?,
                    gl_del_shader: load_sym(lib_gles, "glDeleteShader")?,
                    gl_shader_src: load_sym(lib_gles, "glShaderSource")?,
                    gl_compile_shader: load_sym(lib_gles, "glCompileShader")?,
                    gl_get_shaderiv: load_sym(lib_gles, "glGetShaderiv")?,
                    gl_create_program: load_sym(lib_gles, "glCreateProgram")?,
                    gl_del_program: load_sym(lib_gles, "glDeleteProgram")?,
                    gl_attach_shader: load_sym(lib_gles, "glAttachShader")?,
                    gl_link_program: load_sym(lib_gles, "glLinkProgram")?,
                    gl_get_programiv: load_sym(lib_gles, "glGetProgramiv")?,
                    gl_use_program: load_sym(lib_gles, "glUseProgram")?,
                    gl_get_attrib: load_sym(lib_gles, "glGetAttribLocation")?,
                    gl_en_attrib: load_sym(lib_gles, "glEnableVertexAttribArray")?,
                    gl_vert_ptr: load_sym(lib_gles, "glVertexAttribPointer")?,
                    gl_viewport: load_sym(lib_gles, "glViewport")?,
                    gl_draw_arrays: load_sym(lib_gles, "glDrawArrays")?,
                    gl_finish: load_sym(lib_gles, "glFinish")?,
                    gl_read_pixels: load_sym(lib_gles, "glReadPixels")?,
                })
            })();
            if api.is_none() {
                dl_close(lib_gles);
                dl_close(lib_egl);
                dl_close(lib_gbm);
            }
            api
        }
    }
}

struct GlDeviceCtx {
    api: GlApi,
    _drm_fd: OwnedFd,
    gbm: *mut c_void,
    dpy: *mut c_void,
    ctx: *mut c_void,
    prog: u32,
    pos_loc: i32,
    src_tex: u32,
    dst_tex: u32,
    fbo: u32,
    dst_w: u32,
    dst_h: u32,
}

impl Drop for GlDeviceCtx {
    fn drop(&mut self) {
        unsafe {
            let _ = (self.api.egl_make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), self.ctx);
            if self.fbo != 0 {
                (self.api.gl_del_fbos)(1, &self.fbo);
            }
            if self.dst_tex != 0 {
                (self.api.gl_del_textures)(1, &self.dst_tex);
            }
            if self.src_tex != 0 {
                (self.api.gl_del_textures)(1, &self.src_tex);
            }
            if self.prog != 0 {
                (self.api.gl_del_program)(self.prog);
            }
            let _ = (self.api.egl_make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            (self.api.egl_destroy_context)(self.dpy, self.ctx);
            (self.api.egl_terminate)(self.dpy);
            (self.api.gbm_device_destroy)(self.gbm);
        }
    }
}

/// Descriptor for importing and downscaling a single-plane DMA-BUF frame into linear BGRx.
#[derive(Clone, Copy, Debug)]
pub struct DmabufFrameDesc {
    pub dmabuf_fd: c_int,
    pub sw: u32,
    pub sh: u32,
    pub offset: u32,
    pub stride: u32,
    pub spa_fmt: u32,
    pub modifier: Option<u64>,
    pub dw: u32,
    pub dh: u32,
}

impl GlDeviceCtx {
    fn try_open(render_path: &str) -> Option<Self> {
        let drm_file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_RDWR_CLOEXEC)
            .open(render_path)
            .ok()?;
        let drm_fd: OwnedFd = drm_file.into();
        let api = GlApi::load()?;

        unsafe {
            let gbm = (api.gbm_create_device)(drm_fd.as_raw_fd());
            if gbm.is_null() {
                return None;
            }
            let dpy = (api.egl_get_platform_display)(EGL_PLATFORM_GBM_KHR, gbm, ptr::null());
            if dpy.is_null() {
                (api.gbm_device_destroy)(gbm);
                return None;
            }
            let (mut maj, mut min) = (0i32, 0i32);
            if (api.egl_initialize)(dpy, &mut maj, &mut min) == 0 {
                (api.egl_terminate)(dpy);
                (api.gbm_device_destroy)(gbm);
                return None;
            }
            if (api.egl_bind_api)(EGL_OPENGL_ES_API) == 0 {
                (api.egl_terminate)(dpy);
                (api.gbm_device_destroy)(gbm);
                return None;
            }
            let cfg_attr = [EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
            let mut cfg: *mut c_void = ptr::null_mut();
            let mut ncfg = 0i32;
            if (api.egl_choose_config)(dpy, cfg_attr.as_ptr(), &mut cfg, 1, &mut ncfg) == 0 || ncfg < 1 || cfg.is_null() {
                (api.egl_terminate)(dpy);
                (api.gbm_device_destroy)(gbm);
                return None;
            }
            let ctx_attr = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let ctx = (api.egl_create_context)(dpy, cfg, ptr::null_mut(), ctx_attr.as_ptr());
            if ctx.is_null() || (api.egl_make_current)(dpy, ptr::null_mut(), ptr::null_mut(), ctx) == 0 {
                if !ctx.is_null() {
                    (api.egl_destroy_context)(dpy, ctx);
                }
                (api.egl_terminate)(dpy);
                (api.gbm_device_destroy)(gbm);
                return None;
            }

            let vsrc = c"attribute vec2 pos; varying vec2 vuv; void main(){ vuv = pos * 0.5 + 0.5; gl_Position = vec4(pos, 0.0, 1.0); }";
            let fsrc = c"#extension GL_OES_EGL_image_external : require\nprecision mediump float; uniform samplerExternalOES tex; varying vec2 vuv; void main(){ vec4 c = texture2D(tex, vuv); gl_FragColor = vec4(c.b, c.g, c.r, 1.0); }";
            let vs = compile_shader(&api, GL_VERTEX_SHADER, vsrc.as_ptr());
            let fs = compile_shader(&api, GL_FRAGMENT_SHADER, fsrc.as_ptr());
            let prog = if vs != 0 && fs != 0 {
                let p = (api.gl_create_program)();
                (api.gl_attach_shader)(p, vs);
                (api.gl_attach_shader)(p, fs);
                (api.gl_link_program)(p);
                let mut linked = 0i32;
                (api.gl_get_programiv)(p, GL_LINK_STATUS, &mut linked);
                if linked == 0 {
                    (api.gl_del_program)(p);
                    0
                } else {
                    p
                }
            } else {
                0
            };
            if vs != 0 {
                (api.gl_del_shader)(vs);
            }
            if fs != 0 {
                (api.gl_del_shader)(fs);
            }
            let pos_loc = if prog != 0 {
                (api.gl_get_attrib)(prog, c"pos".as_ptr())
            } else {
                -1
            };
            if prog == 0 || pos_loc < 0 {
                if prog != 0 {
                    (api.gl_del_program)(prog);
                }
                (api.egl_make_current)(dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
                (api.egl_destroy_context)(dpy, ctx);
                (api.egl_terminate)(dpy);
                (api.gbm_device_destroy)(gbm);
                return None;
            }

            let (mut src_tex, mut dst_tex, mut fbo) = (0u32, 0u32, 0u32);
            (api.gl_gen_textures)(1, &mut src_tex);
            (api.gl_gen_textures)(1, &mut dst_tex);
            (api.gl_gen_fbos)(1, &mut fbo);

            (api.gl_bind_texture)(GL_TEXTURE_EXTERNAL_OES, src_tex);
            (api.gl_tex_parami)(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (api.gl_tex_parami)(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (api.gl_tex_parami)(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (api.gl_tex_parami)(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            let _ = (api.egl_make_current)(dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());

            Some(Self {
                api,
                _drm_fd: drm_fd,
                gbm,
                dpy,
                ctx,
                prog,
                pos_loc,
                src_tex,
                dst_tex,
                fbo,
                dst_w: 0,
                dst_h: 0,
            })
        }
    }

    fn blit_to_bgrx(
        &mut self,
        desc: &DmabufFrameDesc,
        fourcc_code: u32,
        dst_bgrx: &mut [u8],
    ) -> bool {
        let (sw, sh, dw, dh) = (desc.sw, desc.sh, desc.dw, desc.dh);
        let need = (dw as usize).saturating_mul(dh as usize).saturating_mul(4);
        if desc.dmabuf_fd < 0
            || sw == 0
            || sh == 0
            || dw == 0
            || dh == 0
            || desc.stride == 0
            || dst_bgrx.len() < need
        {
            return false;
        }
        unsafe {
            if (self.api.egl_get_current_context)() != self.ctx
                && (self.api.egl_make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), self.ctx) == 0
            {
                return false;
            }
            let mut attrs = [
                EGL_WIDTH,
                sw as i32,
                EGL_HEIGHT,
                sh as i32,
                EGL_LINUX_DRM_FOURCC_EXT,
                fourcc_code as i32,
                EGL_DMA_BUF_PLANE0_FD_EXT,
                desc.dmabuf_fd,
                EGL_DMA_BUF_PLANE0_OFFSET_EXT,
                desc.offset as i32,
                EGL_DMA_BUF_PLANE0_PITCH_EXT,
                desc.stride as i32,
                EGL_NONE,
                EGL_NONE,
                EGL_NONE,
                EGL_NONE,
                EGL_NONE,
            ];
            if let Some(modifier) = desc.modifier.filter(|&m| m != DRM_MOD_INVALID) {
                attrs[12] = EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT;
                attrs[13] = (modifier & 0xffff_ffff) as i32;
                attrs[14] = EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT;
                attrs[15] = (modifier >> 32) as i32;
                attrs[16] = EGL_NONE;
            }
            let img = (self.api.egl_create_image)(
                self.dpy,
                ptr::null_mut(),
                EGL_LINUX_DMA_BUF_EXT,
                ptr::null_mut(),
                attrs.as_ptr(),
            );
            if img.is_null() {
                let _ = (self.api.egl_make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
                return false;
            }

            if self.dst_w != dw || self.dst_h != dh {
                (self.api.gl_bind_texture)(GL_TEXTURE_2D, self.dst_tex);
                (self.api.gl_tex_image_2d)(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as i32,
                    dw as i32,
                    dh as i32,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    ptr::null(),
                );
                (self.api.gl_bind_fbo)(GL_FRAMEBUFFER, self.fbo);
                (self.api.gl_fbo_tex)(
                    GL_FRAMEBUFFER,
                    GL_COLOR_ATTACHMENT0,
                    GL_TEXTURE_2D,
                    self.dst_tex,
                    0,
                );
                if (self.api.gl_chk_fbo)(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
                    let _ = (self.api.egl_destroy_image)(self.dpy, img);
                    let _ = (self.api.egl_make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
                    return false;
                }
                self.dst_w = dw;
                self.dst_h = dh;
            } else {
                (self.api.gl_bind_fbo)(GL_FRAMEBUFFER, self.fbo);
            }

            (self.api.gl_bind_texture)(GL_TEXTURE_EXTERNAL_OES, self.src_tex);
            (self.api.gl_egl_image_tex)(GL_TEXTURE_EXTERNAL_OES, img);
            (self.api.gl_viewport)(0, 0, dw as i32, dh as i32);
            (self.api.gl_use_program)(self.prog);
            let quad: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
            (self.api.gl_en_attrib)(self.pos_loc as u32);
            (self.api.gl_vert_ptr)(
                self.pos_loc as u32,
                2,
                GL_FLOAT,
                0,
                0,
                quad.as_ptr().cast::<c_void>(),
            );
            (self.api.gl_draw_arrays)(GL_TRIANGLE_STRIP, 0, 4);
            (self.api.gl_finish)();
            (self.api.gl_read_pixels)(
                0,
                0,
                dw as i32,
                dh as i32,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                dst_bgrx.as_mut_ptr().cast::<c_void>(),
            );
            let _ = (self.api.egl_destroy_image)(self.dpy, img);
            let _ = (self.api.egl_make_current)(self.dpy, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            true
        }
    }
}

unsafe impl Send for DmabufGlBlitter {}

unsafe fn compile_shader(api: &GlApi, tp: u32, src: *const c_char) -> u32 {
    let sh = unsafe { (api.gl_create_shader)(tp) };
    if sh == 0 {
        return 0;
    }
    unsafe {
        (api.gl_shader_src)(sh, 1, &src, ptr::null());
        (api.gl_compile_shader)(sh);
        let mut ok = 0i32;
        (api.gl_get_shaderiv)(sh, GL_COMPILE_STATUS, &mut ok);
        if ok == 0 {
            (api.gl_del_shader)(sh);
            return 0;
        }
    }
    sh
}

/// Lazily initializes a headless Mesa GBM/GLES2 context to detile and downscale
/// GPU-opaque DMA-BUF frames into linear preview SHM only when preview is active.
pub struct DmabufGlBlitter {
    dev: Option<GlDeviceCtx>,
    active_idx: Option<u32>,
    init_attempted: bool,
    fallback_probed: bool,
}

impl Default for DmabufGlBlitter {
    fn default() -> Self {
        Self::new()
    }
}

impl DmabufGlBlitter {
    pub fn new() -> Self {
        Self {
            dev: None,
            active_idx: None,
            init_attempted: false,
            fallback_probed: false,
        }
    }

    /// Imports `desc.dmabuf_fd` via `EGL_LINUX_DMA_BUF_EXT` and writes packed `[B, G, R, 255]`
    /// pixels (`desc.dw x desc.dh`) into `dst_bgrx`.
    pub fn blit_to_bgrx(&mut self, desc: &DmabufFrameDesc, dst_bgrx: &mut [u8]) -> bool {
        let Some(fourcc_code) = spa_vfmt_to_drm_fourcc(desc.spa_fmt) else {
            return false;
        };
        if self.dev.is_none() && !self.init_attempted {
            self.init_attempted = true;
            for idx in 128..132 {
                let path = format!("/dev/dri/renderD{idx}");
                if let Some(ctx) = GlDeviceCtx::try_open(&path) {
                    self.dev = Some(ctx);
                    self.active_idx = Some(idx);
                    break;
                }
            }
        }
        let Some(ctx) = self.dev.as_mut() else {
            return false;
        };
        if ctx.blit_to_bgrx(desc, fourcc_code, dst_bgrx) {
            return true;
        }
        if !self.fallback_probed {
            self.fallback_probed = true;
            for idx in 128..132 {
                if self.active_idx == Some(idx) {
                    continue;
                }
                let path = format!("/dev/dri/renderD{idx}");
                if let Some(mut alt) = GlDeviceCtx::try_open(&path) {
                    if alt.blit_to_bgrx(desc, fourcc_code, dst_bgrx) {
                        self.dev = Some(alt);
                        self.active_idx = Some(idx);
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    #[test]
    fn spa_formats_map_to_expected_drm_fourcc() {
        assert_eq!(spa_vfmt_to_drm_fourcc(VF_BGRX), Some(fourcc(b'X', b'R', b'2', b'4')));
        assert_eq!(spa_vfmt_to_drm_fourcc(VF_BGRA), Some(fourcc(b'A', b'R', b'2', b'4')));
        assert_eq!(spa_vfmt_to_drm_fourcc(VF_RGBX), Some(fourcc(b'X', b'B', b'2', b'4')));
        assert_eq!(spa_vfmt_to_drm_fourcc(VF_RGBA), Some(fourcc(b'A', b'B', b'2', b'4')));
        assert_eq!(spa_vfmt_to_drm_fourcc(0), None);
    }

    #[test]
    fn gbm_dmabuf_gl_blitter_decodes_when_render_node_accessible() {
        let Ok(drm_file) = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_RDWR_CLOEXEC)
            .open("/dev/dri/renderD128")
        else {
            return;
        };
        let lib_gbm = unsafe { dl_open_any(&["libgbm.so.1", "libgbm.so"]) };
        if lib_gbm.is_null() {
            return;
        }
        type GbmCreateBoWithModsFn = unsafe extern "C" fn(
            *mut c_void,
            u32,
            u32,
            u32,
            *const u64,
            u32,
        ) -> *mut c_void;
        type GbmBoVoidFn = unsafe extern "C" fn(*mut c_void);
        type GbmBoFdFn = unsafe extern "C" fn(*mut c_void) -> c_int;
        type GbmBoU32Fn = unsafe extern "C" fn(*mut c_void) -> u32;
        type GbmBoModFn = unsafe extern "C" fn(*mut c_void) -> u64;
        type GbmBoOffFn = unsafe extern "C" fn(*mut c_void, c_int) -> u32;
        unsafe {
            let gbm_create: Option<GbmCreateDeviceFn> = load_sym(lib_gbm, "gbm_create_device");
            let gbm_destroy: Option<GbmDeviceDestroyFn> = load_sym(lib_gbm, "gbm_device_destroy");
            let bo_mods: Option<GbmCreateBoWithModsFn> =
                load_sym(lib_gbm, "gbm_bo_create_with_modifiers");
            let bo_destroy: Option<GbmBoVoidFn> = load_sym(lib_gbm, "gbm_bo_destroy");
            let bo_fd: Option<GbmBoFdFn> = load_sym(lib_gbm, "gbm_bo_get_fd");
            let bo_stride: Option<GbmBoU32Fn> = load_sym(lib_gbm, "gbm_bo_get_stride");
            let bo_mod: Option<GbmBoModFn> = load_sym(lib_gbm, "gbm_bo_get_modifier");
            let bo_off: Option<GbmBoOffFn> = load_sym(lib_gbm, "gbm_bo_get_offset");
            let (Some(gbm_create), Some(gbm_destroy), Some(bo_mods), Some(bo_destroy), Some(bo_fd), Some(bo_stride), Some(bo_mod), Some(bo_off)) =
                (gbm_create, gbm_destroy, bo_mods, bo_destroy, bo_fd, bo_stride, bo_mod, bo_off)
            else {
                dl_close(lib_gbm);
                return;
            };
            let gbm = gbm_create(drm_file.as_raw_fd());
            if gbm.is_null() {
                dl_close(lib_gbm);
                return;
            }
            let mut m = 0x0200_0000_0008_2305u64;
            let mut bo = bo_mods(gbm, 128, 128, fourcc(b'X', b'R', b'2', b'4'), &m, 1);
            if bo.is_null() {
                m = 0;
                bo = bo_mods(gbm, 128, 128, fourcc(b'X', b'R', b'2', b'4'), &m, 1);
            }
            if !bo.is_null() {
                let raw_fd = bo_fd(bo);
                if raw_fd >= 0 {
                    let owned_dmabuf = OwnedFd::from_raw_fd(raw_fd);
                    let desc = DmabufFrameDesc {
                        dmabuf_fd: owned_dmabuf.as_raw_fd(),
                        sw: 128,
                        sh: 128,
                        offset: bo_off(bo, 0),
                        stride: bo_stride(bo),
                        spa_fmt: VF_BGRX,
                        modifier: Some(bo_mod(bo)),
                        dw: 64,
                        dh: 64,
                    };
                    let mut out = vec![0u8; 64 * 64 * 4];
                    let mut blitter = DmabufGlBlitter::new();
                    assert!(blitter.blit_to_bgrx(&desc, &mut out));
                    assert_eq!(out[3], 255);
                }
                bo_destroy(bo);
            }
            gbm_destroy(gbm);
            dl_close(lib_gbm);
        }
    }
}
