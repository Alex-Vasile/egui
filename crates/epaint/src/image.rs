use ecolor::linear_f32_from_linear_u8;
use emath::Vec2;

use crate::{Color32, textures::TextureOptions};
use alloc::sync::Arc;

/// An image stored in RAM.
///
/// To load an image file, see [`ColorImage::from_rgba_unmultiplied`].
///
/// See also: [`ColorImage`], [`FloatImage`].
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ImageData {
    /// RGBA image with 8-bit texels (`Color32`: gamma-encoded, premultiplied,
    /// sRGB-referred).
    Color(Arc<ColorImage>),

    /// High-precision RGBA image with 16-bit float texels ([`FloatImage`]).
    ///
    /// The float path carries the HDR / wide-gamut content: the pixels follow
    /// the [`FloatImage`] contract, and the upload to the GPU is gated by the
    /// content↔surface pairing (see `FloatImage`'s documentation).
    Float(Arc<FloatImage>),
}

impl ImageData {
    /// Image resolution in texels.
    #[inline]
    pub fn resolution(&self) -> [usize; 2] {
        match self {
            Self::Color(image) => image.resolution,
            Self::Float(image) => image.resolution,
        }
    }

    #[deprecated = "Renamed 'resolution'"]
    pub fn size(&self) -> [usize; 2] {
        self.resolution()
    }

    /// Image width in texels.
    pub fn width(&self) -> usize {
        self.resolution()[0]
    }

    /// Image height in texels.
    pub fn height(&self) -> usize {
        self.resolution()[1]
    }

    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Color(_) => 4,
            Self::Float(_) => 8, // 4 × f16
        }
    }
}

// ----------------------------------------------------------------------------

/// A 2D RGBA color image in RAM.
#[derive(Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ColorImage {
    /// width, height in texels.
    resolution: [usize; 2],

    /// Size of the original SVG image (if any), or just the texel resolution of the image.
    source_size: Vec2,

    /// The pixels, row by row, from top to bottom.
    pixels: Vec<Color32>,
}

impl ColorImage {
    /// Create an image from the given pixels, row by row, from top to bottom.
    ///
    /// Panics if `resolution[0] * resolution[1] != pixels.len()`.
    pub fn new(resolution: [usize; 2], pixels: Vec<Color32>) -> Self {
        assert!(
            resolution[0] * resolution[1] == pixels.len(),
            "resolution: {resolution:?}, pixels.len(): {}",
            pixels.len()
        );
        Self {
            resolution,
            source_size: Vec2::new(resolution[0] as f32, resolution[1] as f32),
            pixels,
        }
    }

    /// Create an image filled with the given color.
    pub fn filled(resolution: [usize; 2], color: Color32) -> Self {
        Self::new(resolution, vec![color; resolution[0] * resolution[1]])
    }

    /// Create a [`ColorImage`] from flat un-multiplied RGBA data.
    ///
    /// This is usually what you want to use after having loaded an image file.
    ///
    /// Panics if `resolution[0] * resolution[1] * 4 != rgba.len()`.
    ///
    /// ## Example using the [`image`](crates.io/crates/image) crate:
    /// ``` ignore
    /// fn load_image_from_path(path: &std::path::Path) -> Result<egui::ColorImage, image::ImageError> {
    ///     let image = image::io::Reader::open(path)?.decode()?;
    ///     let resolution = [image.width() as _, image.height() as _];
    ///     let image_buffer = image.to_rgba8();
    ///     let pixels = image_buffer.as_flat_samples();
    ///     Ok(egui::ColorImage::from_rgba_unmultiplied(
    ///         resolution,
    ///         pixels.as_slice(),
    ///     ))
    /// }
    ///
    /// fn load_image_from_memory(image_data: &[u8]) -> Result<ColorImage, image::ImageError> {
    ///     let image = image::load_from_memory(image_data)?;
    ///     let resolution = [image.width() as _, image.height() as _];
    ///     let image_buffer = image.to_rgba8();
    ///     let pixels = image_buffer.as_flat_samples();
    ///     Ok(ColorImage::from_rgba_unmultiplied(
    ///         resolution,
    ///         pixels.as_slice(),
    ///     ))
    /// }
    /// ```
    pub fn from_rgba_unmultiplied(resolution: [usize; 2], rgba: &[u8]) -> Self {
        assert_eq!(
            resolution[0] * resolution[1] * 4,
            rgba.len(),
            "resolution: {:?}, rgba.len(): {}",
            resolution,
            rgba.len()
        );
        let pixels = rgba
            .chunks_exact(4)
            .map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        Self::new(resolution, pixels)
    }

    pub fn from_rgba_premultiplied(resolution: [usize; 2], rgba: &[u8]) -> Self {
        assert_eq!(
            resolution[0] * resolution[1] * 4,
            rgba.len(),
            "resolution: {:?}, rgba.len(): {}",
            resolution,
            rgba.len()
        );
        let pixels = rgba
            .chunks_exact(4)
            .map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        Self::new(resolution, pixels)
    }

    /// Create a [`ColorImage`] from flat opaque gray data.
    ///
    /// Panics if `resolution[0] * resolution[1] != gray.len()`.
    pub fn from_gray(resolution: [usize; 2], gray: &[u8]) -> Self {
        assert_eq!(
            resolution[0] * resolution[1],
            gray.len(),
            "resolution: {:?}, gray.len(): {}",
            resolution,
            gray.len()
        );
        let pixels = gray.iter().map(|p| Color32::from_gray(*p)).collect();
        Self::new(resolution, pixels)
    }

    /// Alternative method to `from_gray`.
    /// Create a [`ColorImage`] from iterator over flat opaque gray data.
    ///
    /// Panics if `resolution[0] * resolution[1] != gray_iter.len()`.
    #[doc(alias = "from_grey_iter")]
    pub fn from_gray_iter(resolution: [usize; 2], gray_iter: impl Iterator<Item = u8>) -> Self {
        let pixels: Vec<_> = gray_iter.map(Color32::from_gray).collect();
        assert_eq!(
            resolution[0] * resolution[1],
            pixels.len(),
            "resolution: {:?}, pixels.len(): {}",
            resolution,
            pixels.len()
        );
        Self::new(resolution, pixels)
    }

    /// A view of the underlying data as `&[u8]`
    #[cfg(feature = "bytemuck")]
    pub fn as_raw(&self) -> &[u8] {
        bytemuck::cast_slice(&self.pixels)
    }

    /// A view of the underlying data as `&mut [u8]`
    #[cfg(feature = "bytemuck")]
    pub fn as_raw_mut(&mut self) -> &mut [u8] {
        bytemuck::cast_slice_mut(&mut self.pixels)
    }

    /// Create a [`ColorImage`] from flat RGB data.
    ///
    /// This is what you want to use after having loaded an image file (and if
    /// you are ignoring the alpha channel - considering it to always be 0xff)
    ///
    /// Panics if `resolution[0] * resolution[1] * 3 != rgb.len()`.
    pub fn from_rgb(resolution: [usize; 2], rgb: &[u8]) -> Self {
        assert_eq!(
            resolution[0] * resolution[1] * 3,
            rgb.len(),
            "resolution: {:?}, rgb.len(): {}",
            resolution,
            rgb.len()
        );
        let pixels = rgb
            .chunks_exact(3)
            .map(|p| Color32::from_rgb(p[0], p[1], p[2]))
            .collect();
        Self::new(resolution, pixels)
    }

    /// An example color image, useful for tests.
    pub fn example() -> Self {
        let width = 128;
        let height = 64;
        let mut img = Self::filled([width, height], Color32::TRANSPARENT);
        for y in 0..height {
            for x in 0..width {
                let h = x as f32 / width as f32;
                let s = 1.0;
                let v = 1.0;
                let a = y as f32 / height as f32;
                img[(x, y)] = crate::Hsva { h, s, v, a }.into();
            }
        }
        img
    }

    /// Set the source size of e.g. the original SVG image.
    #[inline]
    pub fn with_source_size(mut self, source_size: Vec2) -> Self {
        self.source_size = source_size;
        self
    }

    /// Image resolution in texels.
    #[inline]
    pub fn resolution(&self) -> [usize; 2] {
        self.resolution
    }

    #[deprecated = "Renamed 'resolution'"]
    pub fn size(&self) -> [usize; 2] {
        self.resolution()
    }

    /// Image width in texels.
    #[inline]
    pub fn width(&self) -> usize {
        self.resolution[0]
    }

    /// Image height in texels.
    #[inline]
    pub fn height(&self) -> usize {
        self.resolution[1]
    }

    /// The pixels in the image, row-major.
    #[inline]
    pub fn pixels(&self) -> &[Color32] {
        &self.pixels
    }

    /// The pixels in the image, row-major.
    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [Color32] {
        &mut self.pixels
    }

    /// Size of the original SVG image (if any), or just the texel resolution of the image.
    #[inline]
    pub fn source_size(&self) -> Vec2 {
        self.source_size
    }

    /// Double the height until it is larger than `required_height`,
    /// filling new rows with transparent pixels.
    ///
    /// Returns `true` if the image grew.
    pub(crate) fn grow_to_min_height(&mut self, required_height: usize) -> bool {
        while required_height >= self.height() {
            self.resolution[1] *= 2;
        }

        let num_pixels = self.width() * self.height();
        if self.pixels.len() < num_pixels {
            self.pixels.resize(num_pixels, Color32::TRANSPARENT);
            true
        } else {
            false
        }
    }

    /// Create a new image from a patch of the current image.
    ///
    /// This method is especially convenient for screenshotting a part of the app
    /// since `region` can be interpreted as screen coordinates of the entire screenshot if `pixels_per_point` is provided for the native application.
    /// The floats of [`emath::Rect`] are cast to usize, rounding them down in order to interpret them as indices to the image data.
    ///
    /// Panics if `region.min.x > region.max.x || region.min.y > region.max.y`, or if a region larger than the image is passed.
    pub fn region(&self, region: &emath::Rect, pixels_per_point: Option<f32>) -> Self {
        let pixels_per_point = pixels_per_point.unwrap_or(1.0);
        let min_x = (region.min.x * pixels_per_point) as usize;
        let max_x = (region.max.x * pixels_per_point) as usize;
        let min_y = (region.min.y * pixels_per_point) as usize;
        let max_y = (region.max.y * pixels_per_point) as usize;
        assert!(
            min_x <= max_x && min_y <= max_y,
            "Screenshot region is invalid: {region:?}"
        );
        let width = max_x - min_x;
        let height = max_y - min_y;
        let mut output = Vec::with_capacity(width * height);
        let row_stride = self.resolution[0];

        for row in min_y..max_y {
            output.extend_from_slice(
                &self.pixels[row * row_stride + min_x..row * row_stride + max_x],
            );
        }
        Self::new([width, height], output)
    }

    /// Clone a sub-region as a new image.
    pub fn region_by_pixels(&self, [x, y]: [usize; 2], [w, h]: [usize; 2]) -> Self {
        assert!(
            x + w <= self.width(),
            "x + w should be <= self.width(), but x: {}, w: {}, width: {}",
            x,
            w,
            self.width()
        );
        assert!(
            y + h <= self.height(),
            "y + h should be <= self.height(), but y: {}, h: {}, height: {}",
            y,
            h,
            self.height()
        );

        let mut pixels = Vec::with_capacity(w * h);
        for y in y..y + h {
            let offset = y * self.width() + x;
            pixels.extend(&self.pixels[offset..(offset + w)]);
        }
        assert_eq!(
            pixels.len(),
            w * h,
            "pixels.len should be w * h, but got {}",
            pixels.len()
        );
        Self::new([w, h], pixels)
    }
}

impl core::ops::Index<(usize, usize)> for ColorImage {
    type Output = Color32;

    #[inline]
    fn index(&self, (x, y): (usize, usize)) -> &Color32 {
        let [w, h] = self.resolution;
        assert!(x < w && y < h, "x: {x}, y: {y}, w: {w}, h: {h}");
        &self.pixels[y * w + x]
    }
}

impl core::ops::IndexMut<(usize, usize)> for ColorImage {
    #[inline]
    fn index_mut(&mut self, (x, y): (usize, usize)) -> &mut Color32 {
        let [w, h] = self.resolution;
        assert!(x < w && y < h, "x: {x}, y: {y}, w: {w}, h: {h}");
        &mut self.pixels[y * w + x]
    }
}

impl From<ColorImage> for ImageData {
    #[inline(always)]
    fn from(image: ColorImage) -> Self {
        Self::Color(Arc::new(image))
    }
}

impl From<Arc<ColorImage>> for ImageData {
    #[inline]
    fn from(image: Arc<ColorImage>) -> Self {
        Self::Color(image)
    }
}

impl core::fmt::Debug for ColorImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ColorImage")
            .field("resolution", &self.resolution)
            .field("pixel-count", &self.pixels.len())
            .finish_non_exhaustive()
    }
}

// ----------------------------------------------------------------------------

/// The color primaries the content refers to.
///
/// Content tags its primaries so the upload path can decide what to do with
/// it on each granted surface (the content↔surface pairing table in
/// `egui-wgpu`): a tag the surface cannot honor is loudly rejected instead
/// of silently gamut-clipped.
///
/// `Srgb` is the implicit default: every [`ColorImage`] is sRGB-referred,
/// which is why the tag is optional there.
///
/// New primaries arrive as an enum variant + a pairing-table row + a
/// conversion matrix, in one change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Primaries {
    /// The sRGB primaries (IEC 61966-2-1), the implicit default for all
    /// existing content.
    #[default]
    Srgb,

    /// The Display P3 primaries (as used by DCI-P3-derived display color
    /// spaces, D65 white).
    DisplayP3,
}

// ----------------------------------------------------------------------------

/// A 2D RGBA image in RAM with 16-bit float texels: the HDR / wide-gamut
/// content type.
///
/// # Pixel contract
///
/// Each texel is **gamma-encoded in the declared `primaries`**,
/// **premultiplied in the encoded components** (`stored = a × enc(L)` —
/// exactly [`Color32`]'s convention, not `enc(a·L)`), **unclamped above 1.0**,
/// **odd-symmetric for negatives** (the same extension convention as IEC
/// 61966-2-1's extended range and CSS Color 4); **1.0 = SDR reference white**.
///
/// Display: a surface whose color space honors the declared primaries shows
/// the values as-is (the values ARE the surface encoding). Surfaces that
/// cannot honor the tag **loudly reject** the upload at paint time (the
/// standard placeholder renders and a warning logs) — egui never silently
/// gamut-clips or re-interprets tagged content **at upload time**. The
/// pairing is evaluated per *upload*, against the surface of the viewport
/// being painted: a texture already uploaded renders as-is in viewports
/// with a different granted space, and keeps rendering as-is if the granted
/// space later changes (e.g. a monitor move) — no delta is re-emitted, so
/// there is no re-evaluation and no warning either: the stored values are
/// then interpreted by whatever space the presenting surface has.
///
/// Storing values above 1.0 is allowed (the HDR extension lives in the
/// values); displaying them above SDR white is a later stage.
///
/// The RAM format is one [`half::f16`] per component — 1:1 with the
/// `Rgba16Float` GPU format, so uploads are zero-copy on the CPU side.
/// Helpers take `f32` at the edges and convert lossily (round-to-nearest).
#[derive(Clone, PartialEq)]
pub struct FloatImage {
    /// width, height in texels.
    resolution: [usize; 2],

    /// The primaries the (gamma-encoded) pixel values refer to.
    pub primaries: Primaries,

    /// The pixels, row by row, from top to bottom.
    pixels: Vec<[half::f16; 4]>,
}

impl FloatImage {
    /// Create an image from the given pixels, row by row, from top to bottom.
    ///
    /// Panics if `resolution[0] * resolution[1] != pixels.len()`.
    pub fn new(resolution: [usize; 2], primaries: Primaries, pixels: Vec<[half::f16; 4]>) -> Self {
        assert!(
            resolution[0] * resolution[1] == pixels.len(),
            "resolution: {resolution:?}, pixels.len(): {}",
            pixels.len()
        );
        Self {
            resolution,
            primaries,
            pixels,
        }
    }

    /// Create an image filled with the given color.
    ///
    /// The color is in contract layout: gamma-encoded in `primaries`,
    /// premultiplied in the encoded components.
    pub fn filled(resolution: [usize; 2], primaries: Primaries, rgba: [f32; 4]) -> Self {
        Self::new(
            resolution,
            primaries,
            vec![Self::texel(rgba); resolution[0] * resolution[1]],
        )
    }

    /// Create a [`FloatImage`] from flat RGBA data in contract layout:
    /// gamma-encoded in `primaries`, premultiplied in the encoded components,
    /// unclamped above 1.0, odd-symmetric for negatives.
    ///
    /// This is the boundary conversion for apps that already hold
    /// contract-layout pixels (e.g. rendered HDR content). If you hold
    /// linear-light values, encode them first (`ecolor::gamma_from_linear`).
    ///
    /// Panics if `resolution[0] * resolution[1] * 4 != rgba.len()`.
    pub fn from_rgba(resolution: [usize; 2], primaries: Primaries, rgba: &[f32]) -> Self {
        assert_eq!(
            resolution[0] * resolution[1] * 4,
            rgba.len(),
            "resolution: {:?}, rgba.len(): {}",
            resolution,
            rgba.len()
        );
        let pixels = rgba
            .chunks_exact(4)
            .map(|p| Self::texel([p[0], p[1], p[2], p[3]]))
            .collect();
        Self::new(resolution, primaries, pixels)
    }

    /// Lossily convert an 8-bit [`ColorImage`] into a [`FloatImage`].
    ///
    /// `Color32` texels are gamma-encoded, premultiplied in the encoded
    /// components and sRGB-referred — the float contract at 8-bit precision —
    /// so the conversion is a per-component rescale from `[0, 255]` to
    /// `[0.0, 1.0]` (lossy: it cannot recover the quantization). The result
    /// carries [`Primaries::Srgb`].
    pub fn from_color_image(image: &ColorImage) -> Self {
        let pixels = image
            .pixels()
            .iter()
            .map(|c| {
                [
                    half::f16::from_f32(f32::from(c.r()) / 255.0),
                    half::f16::from_f32(f32::from(c.g()) / 255.0),
                    half::f16::from_f32(f32::from(c.b()) / 255.0),
                    half::f16::from_f32(f32::from(c.a()) / 255.0),
                ]
            })
            .collect();
        Self::new(image.resolution(), Primaries::Srgb, pixels)
    }

    /// Image resolution in texels.
    #[inline]
    pub fn resolution(&self) -> [usize; 2] {
        self.resolution
    }

    /// Image width in texels.
    #[inline]
    pub fn width(&self) -> usize {
        self.resolution[0]
    }

    /// Image height in texels.
    #[inline]
    pub fn height(&self) -> usize {
        self.resolution[1]
    }

    /// The pixels in the image, row-major.
    #[inline]
    pub fn pixels(&self) -> &[[half::f16; 4]] {
        &self.pixels
    }

    /// The pixels in the image, row-major.
    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [[half::f16; 4]] {
        &mut self.pixels
    }

    /// A view of the underlying data as `&[u8]` (IEEE 754 binary16
    /// bit-patterns, ready for a zero-copy `Rgba16Float` upload).
    #[cfg(feature = "bytemuck")]
    pub fn as_raw(&self) -> &[u8] {
        bytemuck::cast_slice(&self.pixels)
    }

    /// Convert contract-layout `f32` RGBA into one f16 texel (lossy,
    /// round-to-nearest).
    #[inline]
    fn texel(rgba: [f32; 4]) -> [half::f16; 4] {
        [
            half::f16::from_f32(rgba[0]),
            half::f16::from_f32(rgba[1]),
            half::f16::from_f32(rgba[2]),
            half::f16::from_f32(rgba[3]),
        ]
    }
}

impl core::ops::Index<(usize, usize)> for FloatImage {
    type Output = [half::f16; 4];

    #[inline]
    fn index(&self, (x, y): (usize, usize)) -> &[half::f16; 4] {
        let [w, h] = self.resolution;
        assert!(x < w && y < h, "x: {x}, y: {y}, w: {w}, h: {h}");
        &self.pixels[y * w + x]
    }
}

impl core::ops::IndexMut<(usize, usize)> for FloatImage {
    #[inline]
    fn index_mut(&mut self, (x, y): (usize, usize)) -> &mut [half::f16; 4] {
        let [w, h] = self.resolution;
        assert!(x < w && y < h, "x: {x}, y: {y}, w: {w}, h: {h}");
        &mut self.pixels[y * w + x]
    }
}

impl From<FloatImage> for ImageData {
    #[inline]
    fn from(image: FloatImage) -> Self {
        Self::Float(Arc::new(image))
    }
}

impl From<Arc<FloatImage>> for ImageData {
    #[inline]
    fn from(image: Arc<FloatImage>) -> Self {
        Self::Float(image)
    }
}

impl core::fmt::Debug for FloatImage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FloatImage")
            .field("resolution", &self.resolution)
            .field("primaries", &self.primaries)
            .field("pixel-count", &self.pixels.len())
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for FloatImage {
    /// The `f16` texels are stored as their IEEE 754 binary16 **bit
    /// patterns** (`u16`), not via `half`'s `serde` feature: the bit-pattern
    /// representation is stable on the wire (it does not depend on how
    /// `half` chooses to represent the value) and round-trips exactly.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;

        let pixels: Vec<u16> = self
            .pixels
            .iter()
            .flat_map(|texel| texel.map(half::f16::to_bits))
            .collect();
        let mut state = serializer.serialize_struct("FloatImage", 3)?;
        state.serialize_field("resolution", &self.resolution)?;
        state.serialize_field("primaries", &self.primaries)?;
        state.serialize_field("pixels", &pixels)?;
        state.end()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for FloatImage {
    /// The inverse of [`serde::Serialize::serialize`] for this type;
    /// validates the pixel count.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        #[derive(serde::Deserialize)]
        struct Fields {
            resolution: [usize; 2],
            primaries: Primaries,
            pixels: Vec<u16>,
        }

        let fields = Fields::deserialize(deserializer)?;
        let [w, h] = fields.resolution;
        // Checked: a hostile payload can carry a huge resolution, and
        // wrapping arithmetic would admit it (length match modulo 2^N).
        let expected_len = w.checked_mul(h).and_then(|wh| wh.checked_mul(4));
        if expected_len != Some(fields.pixels.len()) {
            return Err(D::Error::invalid_length(
                fields.pixels.len(),
                &format!("{expected_len:?} (4 per texel of {w}x{h})").as_str(),
            ));
        }
        let pixels = fields
            .pixels
            .chunks_exact(4)
            .map(|texel| {
                [
                    half::f16::from_bits(texel[0]),
                    half::f16::from_bits(texel[1]),
                    half::f16::from_bits(texel[2]),
                    half::f16::from_bits(texel[3]),
                ]
            })
            .collect();
        Ok(Self::new(fields.resolution, fields.primaries, pixels))
    }
}

// ----------------------------------------------------------------------------

/// How to convert font coverage values into alpha and color values.
///
/// epaint stores all glyphs in the font atlas as white (with varying opacity),
/// so that egui can reuse the same glyph for different text colors
/// (with a simple color multiplication in the shader).
///
/// Because of this simplification, we need to apply a non-linear
/// ramp to the glyph colors before writing them into the font atlas,
/// as a way to compensate.
///
/// This whole thing is less than rigorous.
///
/// It would be better to either render all text colors into the font atlas
/// (which would require more atlas space, but would allow for more accurate rendering of colored text and emojis),
/// or do the color compensation in the shader, based on the active text color.
///
/// When experimenting, use <https://fonts.google.com/specimen/Ubuntu> to compare to a ground truth.
///
/// See <https://hikogui.org/2022/10/24/the-trouble-with-anti-aliasing.html> for related analysis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum FontColorTransferFunction {
    /// Use the raw RGBA values from the font rasterizer, without any conversion.
    ///
    /// This is the required mode for colored emojis etc.
    ///
    /// This mode looks good for black-on-white text, i.e. light mode.
    Off,

    /// `alpha = coverage^gamma`.
    ///
    /// Gamma=1 looks good for black-on-white text, i.e. light mode.
    Gamma(f32),

    /// `alpha = 2 * coverage - coverage^2`
    ///
    /// This looks good for white-on-black text, i.e. dark mode.
    ///
    /// Very similar to a gamma of 0.5, but produces sharper text.
    /// See <https://www.desmos.com/calculator/w0ndf5blmn> for a comparison to gamma=0.5.
    #[default]
    TwoCoverageMinusCoverageSq,
}

impl FontColorTransferFunction {
    /// A good-looking default for light mode (black-on-white text).
    pub const LIGHT_MODE_DEFAULT: Self = Self::Off;

    /// A good-looking default for dark mode (white-on-black text).
    pub const DARK_MODE_DEFAULT: Self = Self::TwoCoverageMinusCoverageSq;

    /// How to convert a white color written by the font rasterizer
    /// into a color to be written into the font atlas.
    #[inline(always)]
    pub fn to_atlas_color(self, input_color: Color32) -> Color32 {
        match self {
            Self::Off | Self::Gamma(1.0) => input_color,

            Self::Gamma(gamma) => {
                let coverage = linear_f32_from_linear_u8(input_color.a());
                let alpha = coverage.powf(gamma);
                Color32::from_white_alpha(ecolor::linear_u8_from_linear_f32(alpha))
            }

            Self::TwoCoverageMinusCoverageSq => {
                let coverage = linear_f32_from_linear_u8(input_color.a());
                let alpha = 2.0 * coverage - coverage * coverage;
                Color32::from_white_alpha(ecolor::linear_u8_from_linear_f32(alpha))
            }
        }
    }

    /// Convert coverage to alpha.
    #[inline(always)]
    pub fn alpha_from_coverage(self, coverage: f32) -> f32 {
        let coverage = coverage.clamp(0.0, 1.0);
        match self {
            Self::Off | Self::Gamma(1.0) => coverage,
            Self::Gamma(gamma) => coverage.powf(gamma),
            Self::TwoCoverageMinusCoverageSq => 2.0 * coverage - coverage * coverage,
        }
    }

    #[inline(always)]
    pub fn color_from_coverage(self, coverage: f32) -> Color32 {
        let alpha = self.alpha_from_coverage(coverage);
        Color32::from_white_alpha(ecolor::linear_u8_from_linear_f32(alpha))
    }

    /// Convert this into the closest gamma exponent
    pub fn to_gamma(self) -> f32 {
        match self {
            Self::Off => 1.0,
            Self::Gamma(gamma) => gamma,
            Self::TwoCoverageMinusCoverageSq => 0.5, // approximately the same
        }
    }
}

// ----------------------------------------------------------------------------

/// A change to an image.
///
/// Either a whole new image, or an update to a rectangular region of it.
///
/// Does not implement `Eq`: float content (`ImageData::Float`) only offers
/// `PartialEq` (matching `ecolor::Rgba`'s precedent for float types).
#[derive(Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[must_use = "The painter must take care of this"]
pub struct ImageDelta {
    /// What to set the texture to.
    ///
    /// If [`Self::pos`] is `None`, this describes the whole texture.
    ///
    /// If [`Self::pos`] is `Some`, this describes a patch of the whole image starting at [`Self::pos`].
    pub image: ImageData,

    pub options: TextureOptions,

    /// If `None`, set the whole texture to [`Self::image`].
    ///
    /// If `Some(pos)`, update a sub-region of an already allocated texture with the patch in [`Self::image`].
    pub pos: Option<[usize; 2]>,
}

impl ImageDelta {
    /// Update the whole texture.
    pub fn full(image: impl Into<ImageData>, options: TextureOptions) -> Self {
        Self {
            image: image.into(),
            options,
            pos: None,
        }
    }

    /// Update a sub-region of an existing texture.
    pub fn partial(pos: [usize; 2], image: impl Into<ImageData>, options: TextureOptions) -> Self {
        Self {
            image: image.into(),
            options,
            pos: Some(pos),
        }
    }

    /// Is this affecting the whole texture?
    /// If `false`, this is a partial (sub-region) update.
    pub fn is_whole(&self) -> bool {
        self.pos.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_image_helpers_match_resolution_and_layout() {
        let image = FloatImage::from_rgba([3, 2], Primaries::Srgb, &[1.0; 24]);
        assert_eq!(image.resolution(), [3, 2]);
        assert_eq!(image.width(), 3);
        assert_eq!(image.height(), 2);
        assert_eq!(image.primaries, Primaries::Srgb);
        assert_eq!(image.pixels().len(), 6);
        assert_eq!(image[(2, 1)], [half::f16::from_f32(1.0); 4]);

        let filled = FloatImage::filled([2, 2], Primaries::DisplayP3, [0.5, 0.25, -1.0, 1.0]);
        assert_eq!(filled.primaries, Primaries::DisplayP3);
        assert_eq!(filled[(1, 1)][0], half::f16::from_f32(0.5));
        assert_eq!(filled[(1, 1)][1], half::f16::from_f32(0.25));
        // Unclamped above/below: the contract passes values through.
        assert_eq!(filled[(0, 0)][2], half::f16::from_f32(-1.0));
        assert_eq!(filled[(0, 0)][3], half::f16::from_f32(1.0));

        let from_flat = FloatImage::from_rgba(
            [1, 2],
            Primaries::Srgb,
            &[
                2.0, -2.0, 0.0, 1.0, // row 0
                0.5, 0.5, 0.5, 0.5, // row 1
            ],
        );
        assert_eq!(from_flat[(0, 0)][0], half::f16::from_f32(2.0));
        assert_eq!(from_flat[(0, 0)][1], half::f16::from_f32(-2.0));
        assert_eq!(from_flat[(0, 1)][3], half::f16::from_f32(0.5));
    }

    /// `from_color_image`: the u8 encoded components rescale to `[0, 1]`,
    /// and the premultiply rule survives the conversion — `ColorImage` stores
    /// `stored = a × enc(L)` (gamma-space premultiplied, exactly the float
    /// contract), so the converted pixels must still satisfy it.
    #[test]
    fn from_color_image_rescales_and_keeps_premultiply() {
        let image =
            ColorImage::from_rgba_unmultiplied([2, 1], &[255, 0, 0, 128, 64, 128, 192, 255]);
        let float = FloatImage::from_color_image(&image);

        assert_eq!(float.primaries, Primaries::Srgb);

        // Red pixel: r = enc(1.0) × a = 255 × 128/255 = 128 stored.
        let red = float[(0, 0)];
        assert_eq!(red[0], half::f16::from_f32(128.0 / 255.0));
        assert_eq!(red[1], half::f16::from_f32(0.0));
        assert_eq!(red[3], half::f16::from_f32(128.0 / 255.0));

        // Opaque pixel: premultiply is identity at a = 1.
        let mixed = float[(1, 0)];
        assert_eq!(mixed[0], half::f16::from_f32(64.0 / 255.0));
        assert_eq!(mixed[1], half::f16::from_f32(128.0 / 255.0));
        assert_eq!(mixed[2], half::f16::from_f32(192.0 / 255.0));
        assert_eq!(mixed[3], half::f16::from_f32(1.0));

        // `ColorImage::filled` takes `Color32::TRANSPARENT` by default
        // (already premultiplied): every stored component is 0.
        let transparent =
            FloatImage::from_color_image(&ColorImage::filled([1, 1], Color32::TRANSPARENT));
        assert_eq!(transparent[(0, 0)], [half::f16::from_f32(0.0); 4]);
    }

    /// `ImageData::Float` round-trips through serde exactly, and the wire
    /// representation is the f16 bit pattern (stable across `half` versions).
    #[cfg(feature = "serde")]
    #[test]
    fn float_image_serde_round_trip_and_bit_pattern_stability() {
        let image = FloatImage::from_rgba(
            [2, 1],
            Primaries::DisplayP3,
            // 1.0, -1.0, f16-max, and a negative subnormal: exact in f16.
            &[1.0, -1.0, 65504.0, 0.5, 0.25, -6.0e-8, 2.0, 0.0],
        );
        let data = ImageData::Float(Arc::new(image));

        let json = serde_json::to_string(&data).unwrap();
        // `ImageData` has no `Debug` impl (pre-existing) — compare via `==`.
        assert!(serde_json::from_str::<ImageData>(&json).unwrap() == data);

        // Representation stability: the pixels field is the raw `u16` bits.
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let pixels = value["Float"]["pixels"].as_array().unwrap();
        let expected_bits: Vec<u16> = [1.0, -1.0, 65504.0, 0.5, 0.25, -6.0e-8, 2.0, 0.0]
            .map(half::f16::from_f32)
            .map(half::f16::to_bits)
            .to_vec();
        let wire: Vec<u16> = pixels.iter().map(|p| p.as_u64().unwrap() as u16).collect();
        assert_eq!(wire, expected_bits);
        // And the primaries tag travels with the image.
        assert_eq!(value["Float"]["primaries"].as_str().unwrap(), "DisplayP3");
    }

    /// The `Eq` drop compiles as a `PartialEq`-only surface: `ImageData`
    /// (including float content), `ImageDelta` and `TexturesDelta` compare
    /// but make no `Eq` promise (matching `ecolor::Rgba`).
    #[test]
    fn image_types_remain_partial_eq_after_eq_drop() {
        let float = ImageData::Float(Arc::new(FloatImage::filled(
            [1, 1],
            Primaries::Srgb,
            [1.0, 0.0, 0.0, 1.0],
        )));
        let color = ImageData::Color(Arc::new(ColorImage::filled([1, 1], Color32::WHITE)));

        // `ImageData` has no `Debug` impl (pre-existing) — compare via `==`.
        assert!(float.clone() == float);
        assert!(float != color);

        let delta_a = ImageDelta::full(float.clone(), TextureOptions::LINEAR);
        let delta_b = ImageDelta::full(float, TextureOptions::LINEAR);
        assert!(delta_a == delta_b);

        let mut textures_a = crate::textures::TexturesDelta::default();
        textures_a.push(crate::TextureId::Managed(1), delta_a);
        let mut textures_b = crate::textures::TexturesDelta::default();
        textures_b.push(crate::TextureId::Managed(1), delta_b);
        assert_eq!(textures_a, textures_b); // `TexturesDelta` has a `Debug` impl
        // `TexturesDelta` debug-panics when dropped with unapplied deltas.
        textures_a.clear();
        textures_b.clear();
    }
}
