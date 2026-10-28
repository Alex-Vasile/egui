//! Reading rendered `Rgba16Float` frames back to the CPU as float texels.
//!
//! The 8-bit golden path lives in `egui_kittest` (clamp-to-white u8, snapshot
//! only); this module is the *numeric* readback: it preserves the values the
//! GPU actually stored — above 1.0, negative, subnormal — so tests can assert
//! on them, and so an extended-range capture can be inspected (plan §8 PR 3
//! deliverable 1). The IEEE 754 binary16 decoder is the single source of
//! truth shared by the egui-wgpu runtime tests and the capture path.

use std::sync::mpsc;

/// Decode IEEE 754 binary16 bits to `f32`.
///
/// Hand-rolled to keep the dependency surface unchanged (`half` is a
/// dev-dependency of `epaint` consumers, not of this crate); sanity-checked
/// by [`f16_bits_to_f32_known_values`] below.
pub(crate) fn f16_bits_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1f);
    let frac = u32::from(h & 0x03ff);
    let magnitude = match exp {
        0 if frac == 0 => 0, // ±0
        0 => {
            // Subnormal: normalize. `ilog2` is the index of the highest
            // set bit within the 10-bit fraction field.
            let msb = frac.ilog2();
            ((103 + msb) << 23) | ((frac << (23 - msb)) & 0x007f_ffff)
        }
        31 => 0x7f80_0000 | (frac << 13), // inf / NaN
        exp => ((exp + 112) << 23) | (frac << 13),
    };
    f32::from_bits(sign | magnitude)
}

/// Read back an `Rgba16Float` texture that has been copied into `buffer` as
/// float texels, row by row, top to bottom.
///
/// `bytes_per_row` is the pitch the copy used — wgpu requires it to be a
/// multiple of [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`], so it generally
/// contains padding the decode must skip (see `capture::BufferPadding` for
/// the pitch computation). Blocks until the data is mapped: intended for
/// tests and offline verification, never a per-frame path.
pub fn texture_to_float_image(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
    size: [u32; 2],
    bytes_per_row: u32,
) -> Vec<[f32; 4]> {
    let [width, height] = size;
    assert_eq!(
        u64::from(bytes_per_row * height),
        buffer.size(),
        "readback buffer size must match size × bytes_per_row"
    );

    let slice = buffer.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("readback receiver hung up");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(core::time::Duration::from_secs(10)),
        })
        .expect("GPU poll failed while waiting for the readback map");
    receiver
        .recv()
        .expect("readback sender hung up")
        .expect("failed to map the readback buffer");

    let data = slice
        .get_mapped_range()
        .expect("failed to map the readback buffer");
    let mut texels = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        let row_start = (row * bytes_per_row) as usize;
        for col in 0..width {
            let offset = row_start + (col * 8) as usize; // 4 × f16
            let texel: [u16; 4] = bytemuck::cast_slice(&data[offset..offset + 8])
                .try_into()
                .expect("each f16 texel is exactly eight bytes");
            texels.push([
                f16_bits_to_f32(texel[0]),
                f16_bits_to_f32(texel[1]),
                f16_bits_to_f32(texel[2]),
                f16_bits_to_f32(texel[3]),
            ]);
        }
    }
    texels
}

#[cfg(test)]
mod tests {
    use super::f16_bits_to_f32;

    #[test]
    fn f16_bits_to_f32_known_values() {
        assert_eq!(f16_bits_to_f32(0x3C00), 1.0);
        assert_eq!(f16_bits_to_f32(0xBC00), -1.0);
        assert_eq!(f16_bits_to_f32(0x0000), 0.0);
        assert_eq!(f16_bits_to_f32(0x0000).to_bits(), 0.0f32.to_bits());
        assert_eq!(f16_bits_to_f32(0x8000), 0.0);
        assert_eq!(f16_bits_to_f32(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(f16_bits_to_f32(0x8400), -2.0f32.powi(-14)); // sign + min normal
        assert_eq!(f16_bits_to_f32(0xC400), -4.0);
        assert_eq!(f16_bits_to_f32(0x0400), 2.0f32.powi(-14)); // smallest normal
        assert_eq!(f16_bits_to_f32(0x0001), 2.0f32.powi(-24)); // smallest subnormal
        assert_eq!(f16_bits_to_f32(0x03FF), 1023.0 * 2.0f32.powi(-24));
        assert_eq!(f16_bits_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_bits_to_f32(0x7C00), f32::INFINITY);
        assert!(f16_bits_to_f32(0x7E00).is_nan());
    }
}
