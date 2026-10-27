use egui::ScreenshotCallback;
use epaint::ColorImage;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;
use wgpu::{BindGroupLayout, MultisampleState, StoreOp};

use crate::SurfaceTarget;

/// A texture and a buffer for reading the rendered frame back to the cpu.
///
/// The texture is required since [`wgpu::TextureUsages::COPY_SRC`] is not an allowed
/// flag for the surface texture on all platforms. This means that anytime we want to
/// capture the frame, we first render it to this texture, and then we can copy it to
/// both the surface texture (via a render pass) and the buffer (via a texture to buffer copy),
/// from where we can pull it back
/// to the cpu.
///
/// Since the frame is rendered to this texture and not to the surface, a capture needs no
/// surface texture at all: an occluded or minimized window, which has none to give, can still
/// be captured.
pub struct CaptureState {
    padding: BufferPadding,
    pub texture: wgpu::Texture,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl CaptureState {
    /// `size` and `format` are those of the surface we would otherwise render to.
    pub fn new(device: &wgpu::Device, size: wgpu::Extent3d, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("texture_copy.wgsl"));

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("texture_copy"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let bind_group_layout = pipeline.get_bind_group_layout(0);

        let (texture, padding, bind_group) =
            Self::create_texture(device, size, format, &bind_group_layout);

        Self {
            padding,
            texture,
            pipeline,
            bind_group,
        }
    }

    fn create_texture(
        device: &wgpu::Device,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
        layout: &BindGroupLayout,
    ) -> (wgpu::Texture, BufferPadding, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("egui_screen_capture_texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let padding = BufferPadding::new(size.width);

        let view = texture.create_view(&Default::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
            label: None,
        });

        (texture, padding, bind_group)
    }

    /// Updates the [`CaptureState`] if the size of the surface has changed
    pub fn update(&mut self, device: &wgpu::Device, size: wgpu::Extent3d) {
        if self.texture.size() != size {
            let format = self.texture.format();
            let (new_texture, padding, bind_group) = Self::create_texture(
                device,
                size,
                format,
                &self.pipeline.get_bind_group_layout(0),
            );
            self.texture = new_texture;
            self.padding = padding;
            self.bind_group = bind_group;
        }
    }

    /// Handles copying from the [`CaptureState`] texture to the buffer, and to the surface
    /// texture when there is one to present to.
    /// Pass the returned buffer to [`CaptureState::read_screen_rgba`] to read the data back to the cpu.
    pub fn copy_textures(
        &mut self,
        device: &wgpu::Device,
        output_frame: Option<&wgpu::SurfaceTexture>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> wgpu::Buffer {
        debug_assert!(
            output_frame.is_none_or(|frame| frame.texture.size() == self.texture.size()),
            "Texture sizes must match, `CaptureState::update` was probably not called"
        );

        // It would be more efficient to reuse the Buffer, e.g. via some kind of ring buffer, but
        // for most screenshot use cases this should be fine. When taking many screenshots (e.g. for a video)
        // it might make sense to revisit this and implement a more efficient solution.
        #[allow(clippy::allow_attributes, clippy::arc_with_non_send_sync)] // For wasm
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui_screen_capture_buffer"),
            size: (self.padding.padded_bytes_per_row * self.texture.height()) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let padding = self.padding;
        let tex = &mut self.texture;

        let tex_extent = tex.size();

        encoder.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padding.padded_bytes_per_row),
                    rows_per_image: None,
                },
            },
            tex_extent,
        );

        // Blit to the surface, for the frame that is actually shown. A hidden window has no
        // surface texture to blit to, and nothing to show: the capture above is all we want.
        if let Some(output_frame) = output_frame {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("texture_copy"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_frame.texture.create_view(&Default::default()),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        buffer
    }

    /// Handles copying from the [`CaptureState`] texture to the surface texture and the cpu
    /// This function is non-blocking and will send the data to the given sender when it's ready.
    /// Pass in the buffer returned from [`CaptureState::copy_textures`].
    /// Make sure to call this after the encoder has been submitted.
    pub fn read_screen_rgba(&self, buffer: wgpu::Buffer, callbacks: Vec<ScreenshotCallback>) {
        #[allow(clippy::allow_attributes, clippy::arc_with_non_send_sync)] // For wasm
        let buffer = Arc::new(buffer);
        let buffer_clone = Arc::clone(&buffer);
        let buffer_slice = buffer_clone.slice(..);
        let format = self.texture.format();
        let tex_extent = self.texture.size();
        let padding = self.padding;
        let to_rgba = match format {
            wgpu::TextureFormat::Rgba8Unorm => [0, 1, 2, 3],
            wgpu::TextureFormat::Bgra8Unorm => [2, 1, 0, 3],
            _ => {
                log::error!(
                    "Screen can't be captured unless the surface format is Rgba8Unorm or Bgra8Unorm. Current surface format is {format:?}"
                );
                return;
            }
        };
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            if let Err(err) = result {
                log::error!("Failed to map buffer for reading: {err}");
                return;
            }
            let buffer_slice = buffer.slice(..);

            let mut pixels = Vec::with_capacity((tex_extent.width * tex_extent.height) as usize);
            let mapped_range = match buffer_slice.get_mapped_range() {
                Ok(range) => range,
                Err(err) => {
                    log::error!("Failed to get mapped range for reading: {err}");
                    return;
                }
            };
            for padded_row in mapped_range.chunks(padding.padded_bytes_per_row as usize) {
                let row = &padded_row[..padding.unpadded_bytes_per_row as usize];
                for color in row.chunks(4) {
                    pixels.push(epaint::Color32::from_rgba_premultiplied(
                        color[to_rgba[0]],
                        color[to_rgba[1]],
                        color[to_rgba[2]],
                        color[to_rgba[3]],
                    ));
                }
            }
            drop(mapped_range);
            buffer.unmap();

            let image = Arc::new(ColorImage::new(
                [tex_extent.width as usize, tex_extent.height as usize],
                pixels,
            ));
            for callback in callbacks {
                callback.complete(Arc::clone(&image));
            }
        });
    }
}

#[derive(Copy, Clone)]
struct BufferPadding {
    unpadded_bytes_per_row: u32,
    padded_bytes_per_row: u32,
}

impl BufferPadding {
    fn new(width: u32) -> Self {
        let bytes_per_pixel = core::mem::size_of::<u32>() as u32;
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let padded_bytes_per_row =
            wgpu::util::align_to(unpadded_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        Self {
            unpadded_bytes_per_row,
            padded_bytes_per_row,
        }
    }
}

/// Write captured pixels to a PNG tagged with a cICP chunk describing the
/// [`SurfaceTarget`] the capture came from.
///
/// Captured pixels leave egui-wgpu as an untagged [`ColorImage`] through
/// [`egui::ScreenshotCallback`], so whoever saves them to a file is the one
/// who can tag them. Pass the `SurfaceTarget` of the capturing viewport —
/// [`Painter::surface_decision`](crate::winit::Painter::surface_decision)
/// returns it. Without a tag, a Display-P3-referred dump reads as sRGB and
/// its colors shift (the mustard-vs-red trap).
///
/// # Errors
/// Propagates filesystem and PNG-encoding errors.
pub fn write_capture_png(
    image: &ColorImage,
    path: impl AsRef<Path>,
    target: SurfaceTarget,
) -> std::io::Result<()> {
    let file = std::fs::File::create(path.as_ref())?;
    let mut encoder = png::Encoder::new(
        BufWriter::new(file),
        image.width() as u32,
        image.height() as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    // cICP must come before PLTE and IDAT (PNG third edition), so right after
    // the header is the earliest legal spot for it.
    writer
        .write_chunk(png::chunk::cICP, &target.cicp_code_points())
        .map_err(std::io::Error::other)?;
    writer
        .write_image_data(bytemuck::cast_slice(image.pixels()))
        .map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn target(color_space: wgpu::SurfaceColorSpace) -> SurfaceTarget {
        SurfaceTarget {
            format: wgpu::TextureFormat::Rgba8Unorm,
            color_space,
        }
    }

    #[test]
    fn cicp_points_match_color_space() {
        let expected: &[(wgpu::SurfaceColorSpace, [u8; 4])] = &[
            (wgpu::SurfaceColorSpace::Auto, [1, 13, 0, 1]),
            (wgpu::SurfaceColorSpace::Srgb, [1, 13, 0, 1]),
            (wgpu::SurfaceColorSpace::ExtendedSrgbLinear, [1, 8, 0, 1]),
            (wgpu::SurfaceColorSpace::DisplayP3, [12, 13, 0, 1]),
            (wgpu::SurfaceColorSpace::Bt2100Pq, [9, 16, 0, 1]),
            (wgpu::SurfaceColorSpace::Bt2100Hlg, [9, 18, 0, 1]),
            (wgpu::SurfaceColorSpace::ExtendedSrgb, [1, 13, 0, 1]),
            (wgpu::SurfaceColorSpace::ExtendedDisplayP3, [12, 13, 0, 1]),
        ];
        for (color_space, points) in expected {
            assert_eq!(&target(*color_space).cicp_code_points(), points);
        }
    }

    fn test_image() -> ColorImage {
        let pixels = (0..3 * 2)
            .map(|i| {
                let i = i as u8;
                epaint::Color32::from_rgba_premultiplied(200 + i, i * 30, 255 - i * 10, 255)
            })
            .collect();
        ColorImage::new([3, 2], pixels)
    }

    #[test]
    fn write_capture_png_tags_and_preserves_pixels() {
        let dir = tempfile::tempdir().expect("failed to create a tempdir");
        let image = test_image();

        write_capture_png(
            &image,
            dir.path().join("capture.png"),
            target(wgpu::SurfaceColorSpace::DisplayP3),
        )
        .expect("failed to write the tagged capture PNG");

        let file = std::fs::File::open(dir.path().join("capture.png"))
            .expect("failed to open the written capture PNG");
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .expect("failed to read the capture PNG header");
        let mut buffer = vec![
            0;
            reader
                .output_buffer_size()
                .expect("png reports buffer size")
        ];
        let output = reader
            .next_frame(&mut buffer)
            .expect("failed to decode the capture PNG");

        let cicp = reader
            .info()
            .coding_independent_code_points
            .expect("the capture PNG must carry a cICP chunk");
        assert_eq!(cicp.color_primaries, 12);
        assert_eq!(cicp.transfer_function, 13);
        assert_eq!(cicp.matrix_coefficients, 0);
        assert!(cicp.is_video_full_range_image);

        assert_eq!(
            &buffer[..output.buffer_size()],
            bytemuck::cast_slice(image.pixels())
        );
    }

    /// End-to-end through the capture path: render into the capture texture,
    /// copy, read back, and write the tagged PNG from the screenshot
    /// callback.
    ///
    /// Adapter-requiring: machines without any wgpu adapter skip with a
    /// message, mirroring the pipeline-cache test.
    #[test]
    fn capture_readback_writes_tagged_png() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(adapter) => adapter,
                Err(err) => {
                    #[expect(clippy::print_stderr)]
                    {
                        eprintln!(
                            "skipping capture_readback_writes_tagged_png: no wgpu adapter: {err}"
                        );
                    }
                    return;
                }
            };

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("egui-wgpu capture tag test"),
            ..Default::default()
        }))
        .expect("failed to request a wgpu device for the capture tag test");
        device.on_uncaptured_error(Arc::new(|err| {
            panic!("wgpu device error during the capture tag test: {err}");
        }));

        let size = wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        };
        let mut capture_state = CaptureState::new(&device, size, wgpu::TextureFormat::Rgba8Unorm);

        // Paint a gradient into the capture texture so the readback has
        // known, position-dependent content.
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("capture tag test gradient"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(
                r"
                @vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    var pos = array<vec2<f32>, 3>(
                        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
                    return vec4<f32>(pos[i], 0.0, 1.0);
                }
                @fragment fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
                    return vec4<f32>(frag.x / 4.0, frag.y / 4.0, 0.25, 1.0);
                }
                ",
            )),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("capture tag test gradient"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let view = capture_state.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("capture tag test"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("capture tag test gradient"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }

        let buffer = capture_state.copy_textures(&device, None, &mut encoder);
        queue.submit([encoder.finish()]);

        let dir = tempfile::tempdir().expect("failed to create a tempdir");
        let (tx, rx) = mpsc::channel::<()>();
        let path = dir.path().join("capture.png");
        let callback = ScreenshotCallback::new(move |image| {
            write_capture_png(&image, &path, target(wgpu::SurfaceColorSpace::DisplayP3))
                .expect("failed to write the tagged capture PNG");
            tx.send(()).expect("test channel closed early");
        });
        capture_state.read_screen_rgba(buffer, vec![callback]);

        // `read_screen_rgba` maps the buffer asynchronously; pump the device
        // until the callback runs.
        let deadline = std::time::Instant::now() + core::time::Duration::from_secs(30);
        loop {
            device
                .poll(wgpu::PollType::Poll)
                .expect("GPU poll failed while waiting for the capture readback");
            if rx
                .recv_timeout(core::time::Duration::from_millis(100))
                .is_ok()
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "screenshot callback never ran"
            );
        }

        let file = std::fs::File::open(dir.path().join("capture.png"))
            .expect("failed to open the captured PNG");
        let mut reader = png::Decoder::new(std::io::BufReader::new(file))
            .read_info()
            .expect("failed to read the captured PNG header");
        let mut buffer = vec![
            0;
            reader
                .output_buffer_size()
                .expect("png reports buffer size")
        ];
        let output = reader
            .next_frame(&mut buffer)
            .expect("failed to decode the captured PNG");
        let cicp = reader
            .info()
            .coding_independent_code_points
            .expect("the captured PNG must carry a cICP chunk");
        assert_eq!(
            [
                cicp.color_primaries,
                cicp.transfer_function,
                cicp.matrix_coefficients
            ],
            [12, 13, 0]
        );
        assert!(cicp.is_video_full_range_image);
        assert_eq!(output.width, 4);
        assert_eq!(output.height, 4);
    }
}
