// Color transfer functions shared between the production egui shader
// (`egui.wgsl`) and the CPU↔GPU round-trip test in `renderer.rs`.
//
// `Renderer::new` builds the production module by concatenating this file
// with `egui.wgsl`, and the round-trip test concatenates the same file with
// its own test entry points — both exercise the exact same WGSL source, so
// the tested functions cannot drift from the shipped ones.

// Both functions are the odd-symmetric (signed) extension of the sRGB
// transfer functions, matching `ecolor::linear_from_gamma` /
// `ecolor::gamma_from_linear` and the extended range of IEC 61966-2-1 /
// CSS Color 4: `f(x) = sign(x) · f(|x|)`.
//
// The `sign`/`abs` form is load-bearing: evaluating `pow` on a negative
// base is WGSL-undefined (NaN-prone), and on drivers that lower `select`
// to arithmetic blends that NaN can leak into the result through the
// discarded lane. Computing the power on `abs` and reapplying `sign`
// never evaluates `pow` on a negative value, so no input — including
// `-0.0` — can produce a NaN. For `x > 0` the computation is unchanged
// from the unsigned form: `sign(x)` is exactly `1.0`, and multiplying by
// exactly `1.0` is bit-exact. At `x = ±0.0` both forms yield a zero (the
// toe branch of `f(|x|)` is an exact zero and `sign(±0.0) · 0.0` is again
// a zero); the sign *bit* of that zero may differ between the forms — the
// unsigned form computed `-0.0 / 12.92 = -0.0` for `-0.0`, while this
// backend lowers `sign(-0.0)` to `+0.0` — but the values are numerically
// equal (`-0.0 == +0.0`), which is all float comparison observes.

// linear  from  sRGB gamma
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let sgn = sign(srgb);
    let mag = abs(srgb);
    let cutoff = mag < vec3<f32>(0.04045);
    let lower = mag / vec3<f32>(12.92);
    let higher = pow((mag + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return sgn * select(higher, lower, cutoff);
}

// sRGB gamma  from  linear
fn gamma_from_linear_rgb(rgb: vec3<f32>) -> vec3<f32> {
    let sgn = sign(rgb);
    let mag = abs(rgb);
    let cutoff = mag < vec3<f32>(0.0031308);
    let lower = mag * vec3<f32>(12.92);
    let higher = vec3<f32>(1.055) * pow(mag, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return sgn * select(higher, lower, cutoff);
}

// sRGBA gamma  from  linear
fn gamma_from_linear_rgba(linear_rgba: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(gamma_from_linear_rgb(linear_rgba.rgb), linear_rgba.a);
}
