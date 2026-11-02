#![expect(clippy::unwrap_used)]
#![expect(unsafe_code)]

use core::mem::offset_of;
use std::{collections::HashMap, collections::HashSet, sync::Arc};

use egui::{
    emath::Rect,
    epaint::{Mesh, PaintCallbackInfo, Primitive, Vertex},
};
use glow::HasContext as _;

use crate::check_for_gl_error;
use crate::misc_util::{compile_shader, link_program};
use crate::shader_version::ShaderVersion;
use crate::vao;

/// Re-exported [`glow::Context`].
pub use glow::Context;

const VERT_SRC: &str = include_str!("shader/vertex.glsl");
const FRAG_SRC: &str = include_str!("shader/fragment.glsl");

/// The one warning a glow painter emits when it meets an HDR vertex-color
/// mesh. It names the consequence (the range-reduced twin is drawn) and the
/// escape hatch (the wgpu backend).
const HDR_MESH_WARNING: &str = "egui_glow: this frame contains an HDR vertex-color mesh. \
     The glow backend is SDR by design, so it draws the mesh's range-reduced SDR projection \
     (a uniform linear-light scale that preserves hue) instead of true HDR values. Use the \
     wgpu backend for the extended range, or gate the feature on `Frame::wgpu_render_state()`.";

/// The warn-once state machine for HDR meshes.
///
/// Extracted from [`Painter::paint_mesh`] so the "one warning per painter,
/// none for an SDR-only frame" rule is testable without a GL context.
#[derive(Clone, Copy, Debug, Default)]
struct HdrMeshWarning {
    /// Set by the first HDR mesh this painter sees.
    warned: bool,
}

impl HdrMeshWarning {
    /// Emit [`HDR_MESH_WARNING`] if `mesh` needs one and this painter has not
    /// warned yet. Returns whether a warning was emitted.
    fn warn_once(&mut self, mesh: &Mesh) -> bool {
        if mesh.is_hdr() && !self.warned {
            self.warned = true;
            log::warn!("{HDR_MESH_WARNING}");
            true
        } else {
            false
        }
    }
}

trait TextureFilterExt {
    fn glow_code(&self, mipmap: Option<egui::TextureFilter>) -> u32;
}

impl TextureFilterExt for egui::TextureFilter {
    fn glow_code(&self, mipmap: Option<egui::TextureFilter>) -> u32 {
        match (self, mipmap) {
            (Self::Linear, None) => glow::LINEAR,
            (Self::Nearest, None) => glow::NEAREST,
            (Self::Linear, Some(Self::Linear)) => glow::LINEAR_MIPMAP_LINEAR,
            (Self::Nearest, Some(Self::Linear)) => glow::NEAREST_MIPMAP_LINEAR,
            (Self::Linear, Some(Self::Nearest)) => glow::LINEAR_MIPMAP_NEAREST,
            (Self::Nearest, Some(Self::Nearest)) => glow::NEAREST_MIPMAP_NEAREST,
        }
    }
}

trait TextureWrapModeExt {
    fn glow_code(&self) -> u32;
}

impl TextureWrapModeExt for egui::TextureWrapMode {
    fn glow_code(&self) -> u32 {
        match self {
            Self::ClampToEdge => glow::CLAMP_TO_EDGE,
            Self::Repeat => glow::REPEAT,
            Self::MirroredRepeat => glow::MIRRORED_REPEAT,
        }
    }
}

#[derive(Debug)]
pub struct PainterError(String);

impl core::error::Error for PainterError {}

impl core::fmt::Display for PainterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "OpenGL: {}", self.0)
    }
}

impl From<String> for PainterError {
    #[inline]
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// An OpenGL painter using [`glow`].
///
/// This is responsible for painting egui and managing egui textures.
/// You can access the underlying [`glow::Context`] with [`Self::gl`].
///
/// This struct must be destroyed with [`Painter::destroy`] before dropping, to ensure OpenGL
/// objects have been properly deleted and are not leaked.
///
/// NOTE: all egui viewports share the same painter.
pub struct Painter {
    gl: Arc<glow::Context>,

    max_texture_side: usize,

    program: glow::Program,
    u_screen_size: glow::UniformLocation,
    u_sampler: glow::UniformLocation,
    is_webgl_1: bool,
    vao: crate::vao::VertexArrayObject,
    srgb_textures: bool,
    supports_srgb_framebuffer: bool,
    vbo: glow::Buffer,
    element_array_buffer: glow::Buffer,

    textures: HashMap<egui::TextureId, glow::Texture>,

    /// Texture ids whose float-content upload was already warned about
    /// (warn once per texture id; glow stays SDR by design — plan §9).
    float_content_warned: HashSet<egui::TextureId>,

    /// Whether an HDR vertex-color mesh was already warned about.
    ///
    /// Unlike textures, a mesh has no id to key on (it is per-frame), so this
    /// is a single flag: one line per painter, no per-frame spam.
    hdr_mesh_warning: HdrMeshWarning,

    next_native_tex_id: u64,

    /// Stores outdated OpenGL textures that are yet to be deleted
    textures_to_destroy: Vec<glow::Texture>,

    /// Used to make sure we are destroyed correctly.
    destroyed: bool,
}

/// A callback function that can be used to compose an [`egui::PaintCallback`] for custom rendering
/// with [`glow`].
///
/// The callback is passed, the [`egui::PaintCallbackInfo`] and the [`Painter`] which can be used to
/// access the OpenGL context.
///
/// # Example
///
/// See the [`custom3d_glow`](https://github.com/emilk/egui/blob/main/crates/egui_demo_app/src/apps/custom3d_wgpu.rs) demo source for a detailed usage example.
pub struct CallbackFn {
    f: Box<dyn Fn(PaintCallbackInfo, &Painter) + Sync + Send>,
}

impl CallbackFn {
    pub fn new<F: Fn(PaintCallbackInfo, &Painter) + Sync + Send + 'static>(callback: F) -> Self {
        let f = Box::new(callback);
        Self { f }
    }
}

impl Painter {
    /// Create painter.
    ///
    /// Set `pp_fb_extent` to the framebuffer size to enable `sRGB` support on OpenGL ES and WebGL.
    ///
    /// Set `shader_prefix` if you want to turn on shader workaround e.g. `"#define APPLY_BRIGHTENING_GAMMA\n"`
    /// (see <https://github.com/emilk/egui/issues/794>).
    ///
    /// # Errors
    /// will return `Err` below cases
    /// * failed to compile shader
    /// * failed to create postprocess on webgl with `sRGB` support
    /// * failed to create buffer
    pub fn new(
        gl: Arc<glow::Context>,
        shader_prefix: &str,
        shader_version: Option<ShaderVersion>,
        dithering: bool,
    ) -> Result<Self, PainterError> {
        profiling::function_scope!();
        crate::check_for_gl_error_even_in_release!(&gl, "before Painter::new");

        // some useful debug info. all three of them are present in gl 1.1.
        unsafe {
            let version = gl.get_parameter_string(glow::VERSION);
            let renderer = gl.get_parameter_string(glow::RENDERER);
            let vendor = gl.get_parameter_string(glow::VENDOR);
            log::debug!(
                "\nopengl version: {version}\nopengl renderer: {renderer}\nopengl vendor: {vendor}"
            );
        }

        #[cfg(not(target_arch = "wasm32"))]
        if gl.version().major < 2 {
            // this checks on desktop that we are not using opengl 1.1 microsoft sw rendering context.
            // ShaderVersion::get fn will segfault due to SHADING_LANGUAGE_VERSION (added in gl2.0)
            return Err(PainterError("egui_glow requires opengl 2.0+. ".to_owned()));
        }

        let max_texture_side = unsafe { gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) } as usize;
        let shader_version = shader_version.unwrap_or_else(|| ShaderVersion::get(&gl));
        let is_webgl_1 = shader_version == ShaderVersion::Es100;
        let shader_version_declaration = shader_version.version_declaration();
        log::debug!("Shader header: {shader_version_declaration:?}.");

        let supported_extensions = gl.supported_extensions();
        log::trace!("OpenGL extensions: {supported_extensions:?}");
        let srgb_textures = false; // egui wants normal sRGB-unaware textures

        let supports_srgb_framebuffer = !cfg!(target_arch = "wasm32")
            && supported_extensions.iter().any(|extension| {
                // {GL,GLX,WGL}_ARB_framebuffer_sRGB, …
                extension.ends_with("ARB_framebuffer_sRGB")
            });
        log::debug!("SRGB framebuffer Support: {supports_srgb_framebuffer}");

        unsafe {
            let vert = compile_shader(
                &gl,
                glow::VERTEX_SHADER,
                &format!(
                    "{}\n#define NEW_SHADER_INTERFACE {}\n{}\n{}",
                    shader_version_declaration,
                    shader_version.is_new_shader_interface() as i32,
                    shader_prefix,
                    VERT_SRC
                ),
            )?;
            let frag = compile_shader(
                &gl,
                glow::FRAGMENT_SHADER,
                &format!(
                    "{}\n#define NEW_SHADER_INTERFACE {}\n#define DITHERING {}\n{}\n{}",
                    shader_version_declaration,
                    shader_version.is_new_shader_interface() as i32,
                    dithering as i32,
                    shader_prefix,
                    FRAG_SRC
                ),
            )?;
            let program = link_program(&gl, [vert, frag].iter())?;
            gl.detach_shader(program, vert);
            gl.detach_shader(program, frag);
            gl.delete_shader(vert);
            gl.delete_shader(frag);
            let u_screen_size = gl.get_uniform_location(program, "u_screen_size").unwrap();
            let u_sampler = gl.get_uniform_location(program, "u_sampler").unwrap();

            let vbo = gl.create_buffer()?;

            let a_pos_loc = gl.get_attrib_location(program, "a_pos").unwrap();
            let a_tc_loc = gl.get_attrib_location(program, "a_tc").unwrap();
            let a_srgba_loc = gl.get_attrib_location(program, "a_srgba").unwrap();

            let stride = core::mem::size_of::<Vertex>() as i32;
            let buffer_infos = vec![
                vao::BufferInfo {
                    location: a_pos_loc,
                    vector_size: 2,
                    data_type: glow::FLOAT,
                    normalized: false,
                    stride,
                    offset: offset_of!(Vertex, pos) as i32,
                },
                vao::BufferInfo {
                    location: a_tc_loc,
                    vector_size: 2,
                    data_type: glow::FLOAT,
                    normalized: false,
                    stride,
                    offset: offset_of!(Vertex, uv) as i32,
                },
                vao::BufferInfo {
                    location: a_srgba_loc,
                    vector_size: 4,
                    data_type: glow::UNSIGNED_BYTE,
                    normalized: false,
                    stride,
                    offset: offset_of!(Vertex, color) as i32,
                },
            ];
            let vao = crate::vao::VertexArrayObject::new(&gl, vbo, buffer_infos);

            let element_array_buffer = gl.create_buffer()?;

            crate::check_for_gl_error_even_in_release!(&gl, "after Painter::new");

            Ok(Self {
                gl,
                max_texture_side,
                program,
                u_screen_size,
                u_sampler,
                is_webgl_1,
                vao,
                srgb_textures,
                supports_srgb_framebuffer,
                vbo,
                element_array_buffer,
                textures: Default::default(),
                float_content_warned: HashSet::new(),
                hdr_mesh_warning: HdrMeshWarning::default(),
                next_native_tex_id: 1 << 32,
                textures_to_destroy: Vec::new(),
                destroyed: false,
            })
        }
    }

    /// Access the shared glow context.
    pub fn gl(&self) -> &Arc<glow::Context> {
        &self.gl
    }

    pub fn max_texture_side(&self) -> usize {
        self.max_texture_side
    }

    /// The framebuffer we use as an intermediate render target,
    /// or `None` if we are painting to the screen framebuffer directly.
    ///
    /// This is the framebuffer that is bound when [`egui::Shape::Callback`] is called,
    /// and is where any callbacks should ultimately render onto.
    ///
    /// So if in a [`egui::Shape::Callback`] you need to use an offscreen FBO, you should
    /// then restore to this afterwards with
    /// `gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());`
    #[expect(clippy::unused_self)]
    pub fn intermediate_fbo(&self) -> Option<glow::Framebuffer> {
        // We don't currently ever render to an offscreen buffer,
        // but we may want to start to in order to do anti-aliasing on web, for instance.
        None
    }

    unsafe fn prepare_painting(
        &mut self,
        [width_in_pixels, height_in_pixels]: [u32; 2],
        pixels_per_point: f32,
    ) {
        unsafe {
            self.gl.enable(glow::SCISSOR_TEST);
            // egui outputs mesh in both winding orders
            self.gl.disable(glow::CULL_FACE);
            self.gl.disable(glow::DEPTH_TEST);

            self.gl.color_mask(true, true, true, true);

            self.gl.enable(glow::BLEND);
            self.gl
                .blend_equation_separate(glow::FUNC_ADD, glow::FUNC_ADD);
            self.gl.blend_func_separate(
                // egui outputs colors with premultiplied alpha:
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
                // Less important, but this is technically the correct alpha blend function
                // when you want to make use of the framebuffer alpha (for screenshots, compositing, etc).
                glow::ONE_MINUS_DST_ALPHA,
                glow::ONE,
            );

            if self.supports_srgb_framebuffer {
                self.gl.disable(glow::FRAMEBUFFER_SRGB);
                check_for_gl_error!(&self.gl, "FRAMEBUFFER_SRGB");
            }

            let width_in_points = width_in_pixels as f32 / pixels_per_point;
            let height_in_points = height_in_pixels as f32 / pixels_per_point;

            self.gl
                .viewport(0, 0, width_in_pixels as i32, height_in_pixels as i32);
            self.gl.use_program(Some(self.program));

            self.gl
                .uniform_2_f32(Some(&self.u_screen_size), width_in_points, height_in_points);
            self.gl.uniform_1_i32(Some(&self.u_sampler), 0);
            self.gl.active_texture(glow::TEXTURE0);

            self.vao.bind(&self.gl);
            self.gl
                .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.element_array_buffer));
        }

        check_for_gl_error!(&self.gl, "prepare_painting");
    }

    pub fn clear(&self, screen_size_in_pixels: [u32; 2], clear_color: [f32; 4]) {
        clear(&self.gl, screen_size_in_pixels, clear_color);
    }

    /// You are expected to have cleared the color buffer before calling this.
    pub fn paint_and_update_textures(
        &mut self,
        screen_size_px: [u32; 2],
        pixels_per_point: f32,
        clipped_primitives: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
    ) {
        profiling::function_scope!();

        #[expect(clippy::iter_over_hash_type)] // Order doesn't matter here
        for (id, image_deltas) in textures_delta.set.drain() {
            for image_delta in image_deltas {
                self.set_texture(id, &image_delta);
            }
        }

        self.paint_primitives(screen_size_px, pixels_per_point, clipped_primitives);

        #[expect(clippy::iter_over_hash_type)] // Order doesn't matter here
        for id in textures_delta.free.drain() {
            self.free_texture(id);
        }
    }

    /// Main entry-point for painting a frame.
    ///
    /// You should call `target.clear_color(..)` before
    /// and `target.finish()` after this.
    ///
    /// The following OpenGL features will be set:
    /// - Scissor test will be enabled
    /// - Cull face will be disabled
    /// - Blend will be enabled
    ///
    /// The scissor area and blend parameters will be changed.
    ///
    /// As well as this, the following objects will be unset:
    /// - Vertex Buffer
    /// - Element Buffer
    /// - Texture (and active texture will be set to 0)
    /// - Program
    ///
    /// Please be mindful of these effects when integrating into your program, and also be mindful
    /// of the effects your program might have on this code. Look at the source if in doubt.
    pub fn paint_primitives(
        &mut self,
        screen_size_px: [u32; 2],
        pixels_per_point: f32,
        clipped_primitives: &[egui::ClippedPrimitive],
    ) {
        profiling::function_scope!();
        self.assert_not_destroyed();

        unsafe { self.prepare_painting(screen_size_px, pixels_per_point) };

        for egui::ClippedPrimitive {
            clip_rect,
            primitive,
        } in clipped_primitives
        {
            set_clip_rect(&self.gl, screen_size_px, pixels_per_point, *clip_rect);

            match primitive {
                Primitive::Mesh(mesh) => {
                    self.paint_mesh(mesh);
                }
                Primitive::Callback(callback) => {
                    if callback.rect.is_positive() {
                        profiling::scope!("callback");

                        let info = egui::PaintCallbackInfo {
                            viewport: callback.rect,
                            clip_rect: *clip_rect,
                            pixels_per_point,
                            screen_size_px,
                        };

                        let viewport_px = info.viewport_in_pixels();
                        unsafe {
                            self.gl.viewport(
                                viewport_px.left_px,
                                viewport_px.from_bottom_px,
                                viewport_px.width_px,
                                viewport_px.height_px,
                            );
                        }

                        if let Some(callback) = callback.callback.downcast_ref::<CallbackFn>() {
                            (callback.f)(info, self);
                        } else {
                            log::warn!(
                                "Warning: Unsupported render callback. Expected egui_glow::CallbackFn"
                            );
                        }

                        check_for_gl_error!(&self.gl, "callback");

                        // Restore state:
                        unsafe { self.prepare_painting(screen_size_px, pixels_per_point) };
                    }
                }
            }
        }

        unsafe {
            self.vao.unbind(&self.gl);
            self.gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);

            self.gl.disable(glow::SCISSOR_TEST);

            check_for_gl_error!(&self.gl, "painting");
        }
    }

    #[inline(never)] // Easier profiling
    fn paint_mesh(&mut self, mesh: &Mesh) {
        debug_assert!(mesh.is_valid(), "Mesh is not valid");

        self.hdr_mesh_warning.warn_once(mesh);

        if let Some(texture) = self.texture(mesh.texture_id) {
            unsafe {
                self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
                self.gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    bytemuck::cast_slice(&mesh.vertices),
                    glow::STREAM_DRAW,
                );

                self.gl
                    .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.element_array_buffer));
                self.gl.buffer_data_u8_slice(
                    glow::ELEMENT_ARRAY_BUFFER,
                    bytemuck::cast_slice(&mesh.indices),
                    glow::STREAM_DRAW,
                );

                self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            }

            unsafe {
                self.gl.draw_elements(
                    glow::TRIANGLES,
                    mesh.indices.len() as i32,
                    glow::UNSIGNED_INT,
                    0,
                );
            }

            check_for_gl_error!(&self.gl, "paint_mesh");
        } else {
            log::warn!("Failed to find texture {:?}", mesh.texture_id);
        }
    }

    // ------------------------------------------------------------------------

    pub fn set_texture(&mut self, tex_id: egui::TextureId, delta: &egui::epaint::ImageDelta) {
        profiling::function_scope!();

        self.assert_not_destroyed();

        // glow stays SDR by design (plan §9): float content has no runtime
        // support here. Warn once per texture id and skip the upload — the
        // standard missing-texture behavior renders instead.
        let egui::ImageData::Color(image) = &delta.image else {
            if self.float_content_warned.insert(tex_id) {
                log::warn!(
                    "egui texture {tex_id:?}: float content (ImageData::Float) is not supported \
                     by the glow painter — skipping the upload (the image will not render)"
                );
            }
            return;
        };
        let data: &[u8] = bytemuck::cast_slice(image.pixels());

        let glow_texture = *self
            .textures
            .entry(tex_id)
            .or_insert_with(|| unsafe { self.gl.create_texture().unwrap() });
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(glow_texture));
        }

        self.upload_texture_srgb(delta.pos, image.resolution(), delta.options, data);
    }

    fn upload_texture_srgb(
        &mut self,
        pos: Option<[usize; 2]>,
        [w, h]: [usize; 2],
        options: egui::TextureOptions,
        data: &[u8],
    ) {
        profiling::function_scope!();
        assert_eq!(
            data.len(),
            w * h * 4,
            "Mismatch between texture size and texel count, by {}",
            data.len() % (w * h * 4)
        );
        assert!(
            w <= self.max_texture_side && h <= self.max_texture_side,
            "Got a texture image of size {}x{}, but the maximum supported texture side is only {}",
            w,
            h,
            self.max_texture_side
        );

        unsafe {
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                options.magnification.glow_code(None) as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                options.minification.glow_code(options.mipmap_mode) as i32,
            );

            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                options.wrap_mode.glow_code() as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                options.wrap_mode.glow_code() as i32,
            );
            check_for_gl_error!(&self.gl, "tex_parameter");

            let (internal_format, src_format) = if self.is_webgl_1 {
                let format = if self.srgb_textures {
                    glow::SRGB_ALPHA
                } else {
                    glow::RGBA
                };
                (format, format)
            } else if self.srgb_textures {
                (glow::SRGB8_ALPHA8, glow::RGBA)
            } else {
                (glow::RGBA8, glow::RGBA)
            };

            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);

            let level = 0;
            if let Some([x, y]) = pos {
                profiling::scope!("gl.tex_sub_image_2d");
                self.gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    level,
                    x as _,
                    y as _,
                    w as _,
                    h as _,
                    src_format,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(data)),
                );
                check_for_gl_error!(&self.gl, "tex_sub_image_2d");
            } else {
                let border = 0;
                profiling::scope!("gl.tex_image_2d");
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    level,
                    internal_format as _,
                    w as _,
                    h as _,
                    border,
                    src_format,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(data)),
                );
                check_for_gl_error!(&self.gl, "tex_image_2d");
            }

            if options.mipmap_mode.is_some() {
                self.gl.generate_mipmap(glow::TEXTURE_2D);
                check_for_gl_error!(&self.gl, "generate_mipmap");
            }
        }
    }

    pub fn free_texture(&mut self, tex_id: egui::TextureId) {
        if let Some(old_tex) = self.textures.remove(&tex_id) {
            unsafe { self.gl.delete_texture(old_tex) };
        }
        // A freed texture id may come back with different content; let the
        // next skipped float upload for it warn again (mirrors the wgpu
        // renderer's re-arm-on-free).
        self.float_content_warned.remove(&tex_id);
    }

    /// Get the [`glow::Texture`] bound to a [`egui::TextureId`].
    pub fn texture(&self, texture_id: egui::TextureId) -> Option<glow::Texture> {
        self.textures.get(&texture_id).copied()
    }

    pub fn register_native_texture(&mut self, native: glow::Texture) -> egui::TextureId {
        self.assert_not_destroyed();
        let id = egui::TextureId::User(self.next_native_tex_id);
        self.next_native_tex_id += 1;
        self.textures.insert(id, native);
        id
    }

    pub fn replace_native_texture(&mut self, id: egui::TextureId, replacing: glow::Texture) {
        if let Some(old_tex) = self.textures.insert(id, replacing) {
            self.textures_to_destroy.push(old_tex);
        }
    }

    pub fn read_screen_rgba(&self, [w, h]: [u32; 2]) -> egui::ColorImage {
        profiling::function_scope!();

        let mut pixels = vec![0_u8; (w * h * 4) as usize];
        unsafe {
            self.gl.read_pixels(
                0,
                0,
                w as _,
                h as _,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        let mut flipped = Vec::with_capacity((w * h * 4) as usize);
        for row in pixels.chunks_exact((w * 4) as usize).rev() {
            flipped.extend_from_slice(bytemuck::cast_slice(row));
        }
        egui::ColorImage::new([w as usize, h as usize], flipped)
    }

    pub fn read_screen_rgb(&self, [w, h]: [u32; 2]) -> Vec<u8> {
        profiling::function_scope!();
        let mut pixels = vec![0_u8; (w * h * 3) as usize];
        unsafe {
            self.gl.read_pixels(
                0,
                0,
                w as _,
                h as _,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        pixels
    }

    unsafe fn destroy_gl(&self) {
        unsafe {
            self.gl.delete_program(self.program);
            #[expect(clippy::iter_over_hash_type)]
            for tex in self.textures.values() {
                self.gl.delete_texture(*tex);
            }
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_buffer(self.element_array_buffer);
            for t in &self.textures_to_destroy {
                self.gl.delete_texture(*t);
            }
        }
    }

    /// This function must be called before [`Painter`] is dropped, as [`Painter`] has some OpenGL objects
    /// that should be deleted.
    pub fn destroy(&mut self) {
        if !self.destroyed {
            unsafe {
                self.destroy_gl();
            }
            self.destroyed = true;
        }
    }

    fn assert_not_destroyed(&self) {
        assert!(!self.destroyed, "the egui glow has already been destroyed!");
    }
}

pub fn clear(gl: &glow::Context, screen_size_in_pixels: [u32; 2], clear_color: [f32; 4]) {
    profiling::function_scope!();
    unsafe {
        gl.disable(glow::SCISSOR_TEST);

        gl.viewport(
            0,
            0,
            screen_size_in_pixels[0] as i32,
            screen_size_in_pixels[1] as i32,
        );
        gl.clear_color(
            clear_color[0],
            clear_color[1],
            clear_color[2],
            clear_color[3],
        );
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        if !self.destroyed {
            log::warn!(
                "You forgot to call destroy() on the egui glow painter. Resources will leak!"
            );
        }
    }
}

fn set_clip_rect(
    gl: &glow::Context,
    [width_px, height_px]: [u32; 2],
    pixels_per_point: f32,
    clip_rect: Rect,
) {
    // Transform clip rect to physical pixels:
    let clip_min_x = pixels_per_point * clip_rect.min.x;
    let clip_min_y = pixels_per_point * clip_rect.min.y;
    let clip_max_x = pixels_per_point * clip_rect.max.x;
    let clip_max_y = pixels_per_point * clip_rect.max.y;

    // Round to integer:
    let clip_min_x = clip_min_x.round() as i32;
    let clip_min_y = clip_min_y.round() as i32;
    let clip_max_x = clip_max_x.round() as i32;
    let clip_max_y = clip_max_y.round() as i32;

    // Clamp:
    let clip_min_x = clip_min_x.clamp(0, width_px as i32);
    let clip_min_y = clip_min_y.clamp(0, height_px as i32);
    let clip_max_x = clip_max_x.clamp(clip_min_x, width_px as i32);
    let clip_max_y = clip_max_y.clamp(clip_min_y, height_px as i32);

    unsafe {
        gl.scissor(
            clip_min_x,
            height_px as i32 - clip_max_y,
            clip_max_x - clip_min_x,
            clip_max_y - clip_min_y,
        );
    }
}

/// What these tests can and cannot prove about the glow seam.
///
/// Proven here: the byte layout `Painter::new` declares to the VAO (stride 20,
/// colour at offset 16); that an HDR mesh's uploaded colour bytes are the
/// sidecar's projection — the twin glow draws — and that SDR meshes are
/// untouched; and that `HdrMeshWarning::warn_once` warns exactly once per
/// painter and never for SDR.
///
/// Inspection-only: the call site that routes a mesh into that warning,
/// `self.hdr_mesh_warning.warn_once(mesh)` in `Painter::paint_mesh`. A GL
/// context needs a real driver and a window, and none exists in-tree, so
/// `paint_mesh` cannot be driven from a test — deleting the call leaves every
/// test below green. There is deliberately no fake GL harness: it would assert
/// on the harness rather than on the call. The call is kept correct by review.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Once, OnceLock};

    use egui::{
        Color32,
        emath::{pos2, vec2},
        epaint::HdrVertexColor,
        epaint::mutex::{Mutex, MutexGuard},
    };

    fn sdr_mesh() -> Mesh {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0)),
            Color32::from_rgb(10, 20, 30),
        );
        mesh
    }

    fn hdr_mesh() -> Mesh {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect_hdr(
            Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0)),
            HdrVertexColor::from_rgba_premultiplied([8.0, 1.0, 0.5, 1.0]),
        );
        mesh
    }

    /// Decode the colour the GL attribute would read for vertex `index`,
    /// through the exact byte layout `Painter::new` declares to the VAO.
    fn drawn_color(uploaded: &[u8], index: usize) -> Color32 {
        let at = index * core::mem::size_of::<Vertex>() + offset_of!(Vertex, color);
        Color32::from_rgba_premultiplied(
            uploaded[at],
            uploaded[at + 1],
            uploaded[at + 2],
            uploaded[at + 3],
        )
    }

    /// A `log` recorder that keeps every warning, so the warn-once rule is
    /// asserted on the emitted line itself rather than on the flag.
    struct CaptureLogger;

    static LINES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    static INSTALL: Once = Once::new();

    fn lines() -> &'static Mutex<Vec<String>> {
        LINES.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// Serializes the tests that read the shared capture, so a sibling's
    /// warning cannot be counted.
    fn capture_lock() -> MutexGuard<'static, ()> {
        LOG_LOCK.get_or_init(|| Mutex::new(())).lock()
    }

    impl log::Log for CaptureLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Warn
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                lines().lock().push(record.args().to_string());
            }
        }

        fn flush(&self) {}
    }

    fn install_logger() {
        INSTALL.call_once(|| {
            let _ = log::set_boxed_logger(Box::new(CaptureLogger));
            log::set_max_level(log::LevelFilter::Warn);
        });
    }

    /// Exactly one warning per painter, and none at all for an SDR-only frame.
    #[test]
    fn hdr_meshes_warn_once_and_sdr_never_does() {
        install_logger();
        let _guard = capture_lock();
        lines().lock().clear();

        let mut state = HdrMeshWarning::default();
        let sdr = sdr_mesh();
        assert!(!state.warn_once(&sdr), "an SDR mesh must not warn");
        assert!(!state.warn_once(&sdr));

        let hdr = hdr_mesh();
        assert!(state.warn_once(&hdr), "the first HDR mesh warns");
        assert!(!state.warn_once(&hdr), "and never twice per painter");
        assert!(!state.warn_once(&sdr), "an SDR mesh afterwards stays quiet");

        let captured = lines().lock().clone();
        assert_eq!(captured.len(), 1, "exactly one warning, got {captured:?}");
        assert!(
            captured[0].contains("HDR vertex-color mesh"),
            "the warning names the cause: {captured:?}"
        );
        assert!(
            captured[0].contains("range-reduced SDR projection"),
            "and the consequence: {captured:?}"
        );
        assert!(
            captured[0].contains("wgpu") && captured[0].contains("wgpu_render_state"),
            "and the escape hatch: {captured:?}"
        );
    }

    /// A fresh painter is a fresh warning budget: the flag lives on the
    /// painter, not in a global.
    #[test]
    fn the_warning_budget_is_per_painter() {
        install_logger();
        let _guard = capture_lock();
        let hdr = hdr_mesh();

        let mut first = HdrMeshWarning::default();
        assert!(first.warn_once(&hdr));

        let mut second = HdrMeshWarning::default();
        assert!(
            second.warn_once(&hdr),
            "a new painter warns again — no global one-shot"
        );
    }

    /// The glow painter uploads `&mesh.vertices` verbatim and the VAO reads
    /// the colour at `offset_of!(Vertex, color)` with stride
    /// `size_of::<Vertex>()`. Pin that layout, and assert that what lands at
    /// that offset for an HDR mesh is the sidecar's projection — the twin glow
    /// draws — and that an SDR mesh is untouched.
    #[test]
    fn glow_draws_the_hdr_twin_and_leaves_sdr_bytes_alone() {
        let stride = core::mem::size_of::<Vertex>();
        assert_eq!(stride, 20, "glow uploads a 20-byte stride");
        assert_eq!(
            offset_of!(Vertex, color),
            16,
            "the attribute offset `Painter::new` declares"
        );

        let hdr = hdr_mesh();
        assert!(hdr.is_hdr());
        let uploaded: &[u8] = bytemuck::cast_slice(&hdr.vertices);
        assert_eq!(uploaded.len(), stride * hdr.vertices.len());
        let sidecar = hdr.hdr_colors.as_ref().expect("sidecar");
        for (index, hdr_color) in sidecar.iter().enumerate() {
            assert_eq!(
                drawn_color(uploaded, index),
                hdr_color.projected_color32(),
                "vertex {index}: glow must draw the twin"
            );
        }
        // The twin is not the galley/SDR white: this is a real range reduction.
        assert_ne!(sidecar[0].projected_color32(), Color32::WHITE);

        let sdr = sdr_mesh();
        assert!(!sdr.is_hdr());
        assert!(sdr.hdr_colors.is_none(), "no sidecar is staged for SDR");
        let uploaded: &[u8] = bytemuck::cast_slice(&sdr.vertices);
        assert_eq!(uploaded.len(), stride * sdr.vertices.len());
        for index in 0..sdr.vertices.len() {
            assert_eq!(
                drawn_color(uploaded, index),
                sdr.vertices[index].color,
                "vertex {index}: the SDR path is byte-identical"
            );
        }
    }
}
