use core::{iter, mem::size_of};
use egui_wgpu::wgpu;
use egui_wgpu::wgpu::{Device, Extent3d, Queue, Texture};
use image::RgbaImage;

use crate::wgpu::WAIT_TIMEOUT;

/// Bytes per texel of the offscreen readback for 8-bit RGBA targets.
const U8_BYTES_PER_PIXEL: usize = size_of::<u32>();

/// The u8 snapshot image decoded from a rendered target.
///
/// 8-bit targets (`Rgba8Unorm`) are copied verbatim: the bytes are the
/// premultiplied sRGB pixels egui rendered, exactly as the goldens expect.
///
/// Float targets (`Rgba16Float` — the extended-range harness target) are
/// decoded losslessly via [`egui_wgpu::readback`] and **clamp-converted**
/// into the u8 snapshot domain (`v.clamp(0, 1) × 255` per channel): values
/// above 1.0 saturate to white. The u8 goldens are structurally blind to HDR
/// values anyway (2.0 and 10.0 clamp identically, plan §8); assertions on the
/// actual extended-range numbers belong to the float readback, not here.
pub(crate) fn texture_to_image(device: &Device, queue: &Queue, texture: &Texture) -> RgbaImage {
    if texture.format() == wgpu::TextureFormat::Rgba16Float {
        let bytes_per_row =
            wgpu::util::align_to(texture.width() * 8, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = copy_texture_to_buffer(device, queue, texture, bytes_per_row);
        // Maps the buffer itself (blocking).
        let floats = egui_wgpu::readback::texture_to_float_image(
            device,
            &buffer,
            [texture.width(), texture.height()],
            bytes_per_row,
        );
        let mut pixels = Vec::with_capacity(floats.len() * 4);
        for texel in &floats {
            for channel in texel {
                pixels.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        RgbaImage::from_raw(texture.width(), texture.height(), pixels)
            .expect("failed to create image")
    } else {
        let bytes_per_row = wgpu::util::align_to(
            texture.width() * U8_BYTES_PER_PIXEL as u32,
            wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
        );
        let buffer = copy_texture_to_buffer(device, queue, texture, bytes_per_row);
        let image = map_and_strip_u8(device, &buffer, texture, bytes_per_row);
        buffer.unmap();
        image
    }
}

/// Copies a texture into a fresh pitch-padded `COPY_DST | MAP_READ` buffer,
/// submits, and waits for the GPU. The buffer is left unmapped: the callers
/// do their own (blocking) map — [`egui_wgpu::readback`] for float targets,
/// [`map_and_strip_u8`] for 8-bit ones.
fn copy_texture_to_buffer(
    device: &Device,
    queue: &Queue,
    texture: &Texture,
    bytes_per_row: u32,
) -> wgpu::Buffer {
    let height = texture.height();
    let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Texture to bytes output buffer"),
        size: (bytes_per_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Texture to bytes encoder"),
    });

    // Copy the data from the texture to the buffer
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        Extent3d {
            width: texture.width(),
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(iter::once(encoder.finish()));

    // Note that we're not calling `.await` here; the mapping below polls.
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(WAIT_TIMEOUT),
        })
        .expect("Failed to poll device");

    output_buffer
}

/// Maps a u8 readback buffer (blocking) and strips the 256-byte row padding.
fn map_and_strip_u8(
    device: &Device,
    buffer: &wgpu::Buffer,
    texture: &Texture,
    bytes_per_row: u32,
) -> RgbaImage {
    let unpadded_bytes_per_row = texture.width() as usize * U8_BYTES_PER_PIXEL;

    let buffer_slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |v| drop(sender.send(v)));
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(WAIT_TIMEOUT),
        })
        .expect("Failed to poll device");
    receiver.recv().unwrap().unwrap();

    let data = buffer_slice
        .get_mapped_range()
        .expect("Failed to get mapped range");
    let data = data
        .chunks_exact(bytes_per_row as usize)
        .flat_map(|row| row.iter().take(unpadded_bytes_per_row))
        .copied()
        .collect::<Vec<_>>();

    RgbaImage::from_raw(texture.width(), texture.height(), data).expect("Failed to create image")
}
