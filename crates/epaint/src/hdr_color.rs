//! Per-vertex color values that are not capped at SDR white.
//!
//! The module is private; the public surface is [`HdrVertexColor`], whose
//! documentation carries the content contract.

use ecolor::{
    Color32, Rgba, gamma_from_linear, gamma_u8_from_linear_f32, linear_u8_from_linear_f32,
};

/// A per-vertex HDR color: four gamma-encoded, premultiplied binary16 values.
///
/// This is the value type of the per-mesh HDR sidecar
/// ([`Mesh::hdr_colors`](crate::Mesh::hdr_colors)). It carries the same content
/// contract as [`FloatImage`](crate::FloatImage):
///
/// - **Encoded, never linear.** Stored components are gamma-encoded in the
///   paint space (the transfer function of
///   [`gamma_from_linear`](ecolor::gamma_from_linear)), exactly like
///   [`Color32`]. The shader's `color × texel` multiply therefore stays valid
///   at any magnitude.
/// - **Premultiplied in the encoded components**: `stored = a × enc(L)`.
/// - **Unclamped above 1.0**; `1.0` is the SDR reference white.
/// - **Odd-symmetric for negatives**, so a negative value is the exact
///   negative of its magnitude and survives a linear (scRGB) decode.
/// - **Alpha is coverage, always `0..=1`.** Feathering multiplies all four
///   components, which keeps premultiplication intact.
///
/// Vertex colors are *untagged* content: they are referred to the surface's
/// granted primaries, like all [`Color32`] chrome. A tagged
/// [`FloatImage`](crate::FloatImage) pairs strictly with a surface and can be
/// rejected; vertex colors never reject and never convert.
///
/// # Representation
///
/// The value is stored as four IEEE 754 binary16 bit patterns (`[u16; 4]`) so
/// that `PartialEq`, `Eq` and the serde derive need no NaN caveat: bit
/// equality is the right semantics for a render list. `half` never appears in
/// the public surface — the constructors and accessors are in `f32`.
///
/// Precision: an 11-bit mantissa is strictly finer than the 8 bits of
/// [`Color32`] for values in `0..=1` (worst-case relative error ~5e-4 against
/// ~4e-3), so lifting an existing [`Color32`] into this type is visually
/// exact. The format tops out at 65504, far above any realistic encoded
/// value (a 10⁶ linear value encodes to ~316).
///
/// Construct one with [`Self::from_rgba_premultiplied`] (already encoded and
/// premultiplied), [`Self::from_linear`] (straight-alpha linear light), or
/// from an SDR [`Color32`] / [`Rgba`] via `From`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))] // the wire form *is* `[u16; 4]`
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
#[repr(C)]
pub struct HdrVertexColor {
    /// Four IEEE 754 binary16 bit patterns, `[r, g, b, a]`.
    bits: [u16; 4],
}

impl HdrVertexColor {
    /// Fully transparent (and therefore invisible).
    pub const TRANSPARENT: Self = Self { bits: [0; 4] };

    /// Opaque SDR reference white, `(1, 1, 1, 1)`.
    pub const WHITE: Self = Self {
        bits: [half::f16::ONE.to_bits(); 4],
    };

    /// From already gamma-encoded, premultiplied components.
    ///
    /// This is the contract boundary: the caller is responsible for the
    /// [`stored = a × enc(L)`](Self) convention. Values are rounded to binary16
    /// and are *not* clamped, so values above 1.0 and negatives pass through.
    #[inline]
    pub fn from_rgba_premultiplied(rgba: [f32; 4]) -> Self {
        Self {
            bits: rgba.map(|component| half::f16::from_f32(component).to_bits()),
        }
    }

    /// From straight (unmultiplied) linear-light RGBA.
    ///
    /// Each color component is encoded with [`ecolor::gamma_from_linear`]
    /// (odd-symmetric, so negatives stay negatives) and *then* multiplied by
    /// the coverage `a`, i.e. `stored = a × enc(L)`. `a` itself is coverage and
    /// is not encoded.
    ///
    /// Note that "linear" alone does not settle premultiplication: an
    /// unmultiplied `a` here means a translucent value is stored
    /// premultiplied, which differs from encoding a premultiplied linear
    /// value for every translucent color.
    #[inline]
    pub fn from_linear([r, g, b, a]: [f32; 4]) -> Self {
        Self::from_rgba_premultiplied([
            a * gamma_from_linear(r),
            a * gamma_from_linear(g),
            a * gamma_from_linear(b),
            a,
        ])
    }

    /// The four stored components, lossily decoded back to `f32`.
    #[inline]
    #[must_use]
    pub fn to_array(self) -> [f32; 4] {
        self.bits.map(|bits| half::f16::from_bits(bits).to_f32())
    }

    /// White with the given coverage, `(a, a, a, a)`.
    ///
    /// The HDR counterpart of [`Color32::from_white_alpha`]: useful as an
    /// alpha-only mask that tints rather than replaces.
    #[inline]
    #[must_use]
    pub fn from_white_alpha(a: f32) -> Self {
        Self::from_rgba_premultiplied([a, a, a, a])
    }

    /// All four stored components multiplied by `factor`.
    ///
    /// Multiplying a premultiplied value by a coverage factor keeps
    /// premultiplication intact at any magnitude — the operation feathering
    /// applies. `factor == 1.0` is a lossless no-op (binary16 is exact for
    /// `× 1.0`).
    #[inline]
    #[must_use]
    pub fn scaled(self, factor: f32) -> Self {
        let [r, g, b, a] = self.to_array();
        Self::from_rgba_premultiplied([r * factor, g * factor, b * factor, a * factor])
    }

    /// The legal, premultiplied-`Color32` projection of `self` — the SDR twin
    /// an HDR mesh's [`Vertex::color`](crate::Vertex::color) carries, and the
    /// value the GL backend draws. A pure function of the stored bits.
    ///
    /// The projection is a **uniform scale in linear light**, never a
    /// per-channel clamp: every inter-channel ratio among the positive
    /// components is preserved, so hue and saturation survive range
    /// reduction. Per-channel clamping would shift saturated colors toward
    /// white. Values already inside the SDR range are unchanged (the scale is
    /// `1 / max(peak, 1.0)`), so lifting a legal `Color32` and projecting it
    /// back is stable.
    ///
    /// It is total:
    ///
    /// - `a <= 0` returns [`Color32::TRANSPARENT`]. Feather rings and cleared
    ///   text vertices land here, and dividing by zero would poison the
    ///   quantizer with NaN.
    /// - All-negative or exact black (`peak <= 0`) returns premultiplied black
    ///   at the incoming alpha — "as dark as SDR goes", still occluding an
    ///   opaque one.
    /// - Mixed-sign values are scaled by the positive peak and their negative
    ///   components drop to zero: no uniform positive scale can put a negative
    ///   component in `0..=1`.
    /// - Non-finite input cannot reach the quantizer: an `inf` peak makes the
    ///   scale `0`, and a `NaN` component is ignored by the `max` fold.
    ///
    /// To project a *feathered* value, [`scaled`](Self::scaled) it first: the
    /// reduction divides by the alpha, so scaling the value and scaling the
    /// alpha are equivalent (`x.scaled(c).projected_color32()`).
    #[must_use]
    pub fn projected_color32(&self) -> Color32 {
        use ecolor::linear_from_gamma;

        let [r, g, b, a] = self.to_array();

        if a <= 0.0 {
            return Color32::TRANSPARENT;
        }

        // Straight light: un-premultiply the *encoded* components, then
        // decode. The scale has to happen in linear light — scaling encoded
        // values is not a luminance change and compounds with the transfer
        // curve.
        let lin = [
            linear_from_gamma(r / a),
            linear_from_gamma(g / a),
            linear_from_gamma(b / a),
        ];
        // Ignore negatives: SDR cannot show them, and a negative peak would
        // flip the sign of the scale (an all-negative value would display as
        // white instead of black).
        let peak = lin.iter().fold(0.0_f32, |max, &c| max.max(c));
        if peak <= 0.0 {
            return Color32::from_rgba_premultiplied(0, 0, 0, linear_u8_from_linear_f32(a));
        }

        // Values already inside the SDR range are untouched; only the excess
        // is range-reduced, uniformly across channels.
        let k = 1.0 / peak.max(1.0);
        let scaled = lin.map(|c| (c * k).max(0.0));

        // Encode each straight component, *then* premultiply by the
        // (unencoded) coverage and quantize — `Color32`'s own conversion
        // order, which is what keeps `rgb <= a`.
        let quantize = |c: f32| ((gamma_u8_from_linear_f32(c) as f32 * a) + 0.5) as u8;
        Color32::from_rgba_premultiplied(
            quantize(scaled[0]),
            quantize(scaled[1]),
            quantize(scaled[2]),
            linear_u8_from_linear_f32(a),
        )
    }
}

impl From<Color32> for HdrVertexColor {
    /// Lossless-to-well-within-one-8-bit-step lift of an SDR color.
    ///
    /// A per-component `/255` rescale, exactly like
    /// [`FloatImage::from_color_image`](crate::FloatImage::from_color_image) —
    /// not a transfer-function round trip. The stored bytes are already
    /// encoded and premultiplied, so they are copied, not converted.
    #[inline]
    fn from(color: Color32) -> Self {
        Self::from_rgba_premultiplied([
            f32::from(color.r()) / 255.0,
            f32::from(color.g()) / 255.0,
            f32::from(color.b()) / 255.0,
            f32::from(color.a()) / 255.0,
        ])
    }
}

impl From<Rgba> for HdrVertexColor {
    /// From a linear-light, **premultiplied** [`Rgba`].
    ///
    /// The counterpart of `impl From<Rgba> for Color32`: unmultiply the
    /// coverage, encode each straight component, then premultiply again, i.e.
    /// `stored = a × enc(rgb / a)`. Encoding the premultiplied value in place
    /// would store `enc(a × L)`, which is a different (photometric) reading
    /// for every translucent color.
    #[inline]
    fn from(rgba: Rgba) -> Self {
        let [r, g, b, a] = rgba.to_array();
        if a == 0.0 {
            // Additive or fully transparent: there is no straight color to
            // recover, so encode the components as they are.
            Self::from_rgba_premultiplied([
                gamma_from_linear(r),
                gamma_from_linear(g),
                gamma_from_linear(b),
                0.0,
            ])
        } else {
            Self::from_rgba_premultiplied([
                gamma_from_linear(r / a) * a,
                gamma_from_linear(g / a) * a,
                gamma_from_linear(b / a) * a,
                a,
            ])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute error tolerance of the `Color32` lift: half an 8-bit step.
    const HALF_U8_STEP: f32 = 1.0 / 512.0;

    #[test]
    fn constants_are_the_contract_values() {
        assert_eq!(HdrVertexColor::TRANSPARENT.to_array(), [0.0; 4]);
        assert_eq!(HdrVertexColor::WHITE.to_array(), [1.0; 4]);
        assert_eq!(core::mem::size_of::<HdrVertexColor>(), 8);
    }

    #[test]
    fn round_trip_is_exact_on_f16_representable_inputs() {
        // Arbitrary f32 cannot round-trip through 11 bits; these values are
        // exactly representable, and a test that expected otherwise would
        // encode a lie.
        for value in [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, -2.0, 0.125] {
            let rgba = [value, -value, value, 1.0];
            let round_tripped = HdrVertexColor::from_rgba_premultiplied(rgba).to_array();
            assert_eq!(round_tripped, rgba, "round trip of {rgba:?}");
        }
    }

    #[test]
    fn negatives_and_above_one_are_preserved() {
        let color = HdrVertexColor::from_rgba_premultiplied([-2.0, 4.0, 6.0, 1.0]);
        assert_eq!(color.to_array(), [-2.0, 4.0, 6.0, 1.0]);
    }

    #[test]
    fn color32_lift_error_is_under_half_an_8_bit_step() {
        // Every value of every component, one channel at a time with the
        // others off their endpoints: the contract is "all 256 values per
        // component", so a `step_by` sample would not test it.
        const OTHERS: [u8; 4] = [37, 201, 128, 255];
        for value in 0..=255u8 {
            for channel in 0..4 {
                let mut rgba = OTHERS;
                rgba[channel] = value;
                let color = Color32::from_rgba_premultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
                let got = HdrVertexColor::from(color).to_array()[channel];
                let want = f32::from(value) / 255.0;
                assert!(
                    (got - want).abs() < HALF_U8_STEP,
                    "{color:?} channel {channel}: lifted {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn from_linear_encodes_then_premultiplies() {
        let [r, g, b, a] = [0.5, 0.25, 4.0, 0.5];
        let linear = HdrVertexColor::from_linear([r, g, b, a]);
        let expected = HdrVertexColor::from_rgba_premultiplied([
            a * gamma_from_linear(r),
            a * gamma_from_linear(g),
            a * gamma_from_linear(b),
            a,
        ]);
        assert_eq!(linear, expected);
        // The alpha is coverage, not encoded.
        assert_eq!(linear.to_array()[3], a);
        // The photometric misreading — encoding the product — differs.
        let photometric = gamma_from_linear(a * r);
        assert!((linear.to_array()[0] - photometric).abs() > 0.05);
    }

    #[test]
    fn from_linear_preserves_negative_odd_symmetry() {
        let positive = HdrVertexColor::from_linear([2.0, 2.0, 2.0, 1.0]);
        let negative = HdrVertexColor::from_linear([-2.0, -2.0, -2.0, 1.0]);
        let [r, g, b, a] = negative.to_array();
        let positive_rgb = positive.to_array();
        assert_eq!(a, 1.0);
        for (got, want) in core::iter::zip([r, g, b], &positive_rgb[..3]) {
            assert_eq!(got, -want);
        }
    }

    #[test]
    fn from_rgba_matches_color32_conversion() {
        // `impl From<Rgba> for Color32` is the tree's definition of
        // premultiplied-linear → encoded-premultiplied. Agree with it.
        let rgba = Rgba::from_rgba_unmultiplied(0.2, 0.7, 0.4, 0.5);
        let lifted = HdrVertexColor::from(rgba).to_array();
        let color32 = Color32::from(rgba);
        let expected = [
            f32::from(color32.r()) / 255.0,
            f32::from(color32.g()) / 255.0,
            f32::from(color32.b()) / 255.0,
        ];
        for (got, want) in core::iter::zip(&lifted[..3], expected) {
            assert!(
                (got - want).abs() < 2.0 / 255.0,
                "from(Rgba) {lifted:?} vs Color32 {expected:?}"
            );
        }
        assert!((lifted[3] - 0.5).abs() < 1.0 / 255.0);
    }

    #[test]
    fn feathering_multiply_keeps_premultiplication() {
        let color = HdrVertexColor::from_linear([4.0, 1.0, 0.25, 0.8]);
        let feather = color.scaled(0.5);
        let [r, g, b, a] = feather.to_array();
        let original = color.to_array();
        assert_eq!(a, original[3] * 0.5);
        for (got, want) in core::iter::zip([r, g, b], &original[..3]) {
            assert_eq!(got, want * 0.5);
        }
        // The ratios are untouched, so the feather's twin is the same hue at
        // half the coverage: a uniform linear-light scale in both cases.
        let feathered_twin = feather.projected_color32();
        let reference_twin = color.projected_color32();
        let [fr, fg, fb, fa] = feathered_twin.to_array();
        let [rr, rg, rb, ra] = reference_twin.to_array();
        assert_eq!(fa, ra / 2, "alpha halves");
        for (got, want) in [(fr, rr), (fg, rg), (fb, rb)] {
            assert!(
                got.abs_diff(want / 2) <= 1,
                "feathered twin {got} vs half of {want}"
            );
        }
    }

    /// Appends the alpha component to a three-channel array.
    trait MapAlpha {
        fn map_alpha(self, a: f32) -> [f32; 4];
    }

    impl MapAlpha for [f32; 3] {
        fn map_alpha(self, a: f32) -> [f32; 4] {
            [self[0], self[1], self[2], a]
        }
    }

    #[test]
    fn projection_zero_alpha_is_transparent() {
        let color = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 0.0]);
        assert_eq!(color.projected_color32(), Color32::TRANSPARENT);
        // Scaling to zero coverage lands in the same guard.
        let opaque = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
        assert_eq!(opaque.scaled(0.0).projected_color32(), Color32::TRANSPARENT);
    }

    #[test]
    fn projection_all_negative_is_black_at_its_alpha() {
        let color = HdrVertexColor::from_rgba_premultiplied([-4.0, -1.0, -0.5, 1.0]);
        assert_eq!(
            color.projected_color32(),
            Color32::from_rgba_premultiplied(0, 0, 0, 255)
        );

        // An opaque one must still occlude, so not TRANSPARENT.
        let translucent = HdrVertexColor::from_rgba_premultiplied([-4.0, -1.0, -0.5, 0.5]);
        let twin = translucent.projected_color32();
        assert_eq!(twin.to_array()[0..3], [0, 0, 0]);
        assert_eq!(twin.a(), 128);
    }

    #[test]
    fn projection_exact_black_keeps_its_alpha() {
        let color = HdrVertexColor::from_rgba_premultiplied([0.0, 0.0, 0.0, 1.0]);
        assert_eq!(color.projected_color32(), Color32::BLACK);
    }

    #[test]
    fn projection_preserves_channel_ratios() {
        // A linear-light 4 : 2 : 0.5 becomes 1 : 0.5 : 0.125, uniformly
        // scaled. The stored components are encoded, so the expected values
        // are the encodings of the scaled *linear* values.
        let linear = [4.0, 2.0, 0.5];
        let color =
            HdrVertexColor::from_rgba_premultiplied(linear.map(gamma_from_linear).map_alpha(1.0));
        let twin = color.projected_color32();
        let [r, g, b, a] = twin.to_array();
        assert_eq!(a, 255);
        assert_eq!(r, 255);
        assert_eq!(g, gamma_u8_from_linear_f32(0.5));
        assert_eq!(b, gamma_u8_from_linear_f32(0.125));
    }

    #[test]
    fn projection_never_emits_rgb_above_alpha() {
        for a in [0.0, 0.1, 0.5, 0.75, 1.0] {
            for rgb in [
                [4.0, 2.0, 0.5],
                [0.1, 0.2, 0.3],
                [8.0, 0.0, 2.0],
                [-1.0, 2.0, 9.0],
            ] {
                let color = HdrVertexColor::from_rgba_premultiplied([
                    rgb[0] * a,
                    rgb[1] * a,
                    rgb[2] * a,
                    a,
                ]);
                let twin = color.projected_color32();
                let [r, g, b, alpha] = twin.to_array();
                assert!(r <= alpha && g <= alpha && b <= alpha, "{twin:?} at a={a}");
            }
        }
    }

    #[test]
    fn projection_mixed_sign_drops_negatives() {
        let color = HdrVertexColor::from_rgba_premultiplied([4.0, -1.0, 0.2, 1.0]);
        let twin = color.projected_color32();
        let [r, g, b, _] = twin.to_array();
        assert_eq!(r, 255, "positive peak maps to white");
        assert_eq!(g, 0, "unrepresentable negative drops to black");
        assert!(b > 0, "0.2/4.0 is still positive");
    }

    #[test]
    fn projection_is_stable_across_a_lift_round_trip() {
        // A legal premultiplied color (rgb <= a) inside the SDR range: the
        // lift is exact within f16 and the projection must be the identity up
        // to 8-bit rounding — the `1 / max(peak, 1.0)` floor is what makes
        // sub-white values not brighten on the SDR twin.
        let color = Color32::from_rgba_unmultiplied(200, 100, 50, 128);
        let lifted = HdrVertexColor::from(color);
        let projected = lifted.projected_color32();
        let [r, g, b, a] = projected.to_array();
        for (got, want) in [
            (r, color.r()),
            (g, color.g()),
            (b, color.b()),
            (a, color.a()),
        ] {
            assert!(
                got.abs_diff(want) <= 1,
                "projection of the lift {projected:?} vs {color:?}"
            );
        }
    }

    #[test]
    fn projection_of_white_is_white() {
        assert_eq!(HdrVertexColor::WHITE.projected_color32(), Color32::WHITE);
        // A mid-gray SDR value must not brighten.
        let gray = Color32::from_gray(128);
        assert_eq!(HdrVertexColor::from(gray).projected_color32(), gray);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_is_the_bits() {
        let color = HdrVertexColor::from_rgba_premultiplied([4.0, -2.0, 0.5, 1.0]);
        let json = serde_json::to_string(&color).expect("serialize");
        // `serde(transparent)`: the wire form is the bare `[u16; 4]`.
        assert!(json.starts_with('['), "{json}");
        let bits: [u16; 4] = serde_json::from_str(&json).expect("deserialize as bits");
        assert_eq!(bits, color.bits);
        let back: HdrVertexColor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, color);
    }
}
