//! The HDR vertex-color tier, end to end.
//!
//! Two layers of evidence:
//!
//! 1. **GPU numeric** (this file): real `Rgba16Float` offscreen targets,
//!    rendered through the production `egui-wgpu` renderer and read back
//!    losslessly with [`egui_wgpu::readback::texture_to_float_image`]. A u8
//!    snapshot cannot see above 1.0 by definition, so these assertions are the
//!    only place the extended range is proven.
//! 2. **CPU contract** (beside the code, in `epaint`): the value contract,
//!    size guards, serde round-trips, the sidecar invariant, the tessellator's
//!    three producers, the opacity rules and the rayon tier pre-split all live
//!    in the modules they exercise (`epaint/src/hdr_color.rs`,
//!    `mesh.rs`, `shapes/shape.rs`, `tessellator.rs`), which is the tree's
//!    convention for unit tests. The public-API rows that need no unit access
//!    are repeated at the bottom of this file.
//!
//! Every numeric test also exists as an MSAA-4× variant where multisample
//! resolution could plausibly change the value (the PR 3 trap).

#![cfg(feature = "wgpu")]

use egui::epaint::{HdrVertexColor, Mesh, Primitive, Shape, Vertex};
use egui::{Color32, Pos2, Rect, RichText, Ui, Vec2, pos2};
use egui_wgpu::wgpu;

const SIZE: u32 = 64;

/// Render one egui frame into a fresh `Rgba16Float` offscreen target and read
/// every texel back losslessly, with no surface color space set (the
/// `GammaEncoded` default).
///
/// `msaa` is baked into the pipeline, so it must match `RendererOptions`'
/// sample count; with more than one sample the pass renders into a
/// multi-sampled intermediate that resolves into the read-back target.
fn render_float(msaa: u32, ui_fn: impl FnMut(&mut Ui)) -> Vec<[f32; 4]> {
    render_float_on(None, msaa, ui_fn)
}

/// [`render_float`] with an explicit granted surface color space, which is
/// what selects the fragment entry point (`OutputEncoding::of`).
fn render_float_on(
    surface_color_space: Option<wgpu::SurfaceColorSpace>,
    msaa: u32,
    mut ui_fn: impl FnMut(&mut Ui),
) -> Vec<[f32; 4]> {
    let options = egui_wgpu::RendererOptions {
        msaa_samples: msaa,
        ..egui_wgpu::RendererOptions::PREDICTABLE
    };
    let render_state =
        egui_kittest::wgpu::create_render_state(egui_kittest::wgpu::default_wgpu_setup(), options);
    render_state
        .renderer
        .write()
        .set_active_surface_color_space(surface_color_space);

    // Any device error (validation or otherwise) fails the test: the tier
    // switches pipeline and vertex-buffer bindings, and those are exactly the
    // things wgpu validates at draw time.
    render_state
        .device
        .on_uncaptured_error(std::sync::Arc::new(|err| {
            panic!("wgpu device error during an HDR vertex-color test: {err}");
        }));

    let ctx = egui::Context::default();
    // A deterministic frame: one pass, screen-sized.
    ctx.set_pixels_per_point(1.0);
    let raw_input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(SIZE as f32))),
        ..Default::default()
    };
    let output = ctx.run_ui(raw_input, |ui| ui_fn(ui));
    let egui::FullOutput {
        shapes,
        textures_delta,
        ..
    } = output;

    // Apply the frame's texture deltas (the font atlas for the text test,
    // nothing for the mesh tests) exactly as a real integration would; the
    // delta type panics if dropped unapplied.
    {
        let mut renderer = render_state.renderer.write();
        let mut textures_delta = textures_delta;
        #[expect(clippy::iter_over_hash_type)] // Order doesn't matter here
        for (id, images) in textures_delta.set.drain() {
            for image in images {
                renderer.update_texture(&render_state.device, &render_state.queue, id, &image);
            }
        }
        #[expect(clippy::iter_over_hash_type)] // Order doesn't matter here
        for id in textures_delta.free.drain() {
            renderer.free_texture(&id);
        }
    }

    let paint_jobs = ctx.tessellate(shapes, 1.0);

    let size_in_pixels = [SIZE, SIZE];
    let screen = egui_wgpu::ScreenDescriptor {
        pixels_per_point: 1.0,
        size_in_pixels,
    };
    let target_format = wgpu::TextureFormat::Rgba16Float;

    let mut encoder = render_state
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hdr_vertex_colors encoder"),
        });

    let user_buffers = render_state.renderer.write().update_buffers(
        &render_state.device,
        &render_state.queue,
        &mut encoder,
        &paint_jobs,
        &screen,
        target_format,
    );

    let texture = render_state
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_vertex_colors target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    let target_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let (view, resolve_target) = if msaa > 1 {
        let intermediate = render_state
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("hdr_vertex_colors MSAA intermediate"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: msaa,
                dimension: wgpu::TextureDimension::D2,
                format: target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
        (
            intermediate.create_view(&wgpu::TextureViewDescriptor::default()),
            Some(&target_view),
        )
    } else {
        (target_view.clone(), None)
    };

    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hdr_vertex_colors pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            })
            .forget_lifetime();
        render_state
            .renderer
            .write()
            .render(&mut pass, &paint_jobs, &screen, target_format);
    }

    // 4 × f16 per texel; 64 × 8 = 512 bytes, already a multiple of wgpu's
    // 256-byte row alignment.
    let bytes_per_row = SIZE * 8;
    let readback_buffer = render_state.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hdr_vertex_colors readback"),
        size: u64::from(bytes_per_row * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );

    render_state.queue.submit(core::iter::chain(
        user_buffers,
        core::iter::once(encoder.finish()),
    ));

    egui_wgpu::readback::texture_to_float_image(
        &render_state.device,
        &readback_buffer,
        size_in_pixels,
        bytes_per_row,
    )
}

fn texel(texels: &[[f32; 4]], x: u32, y: u32) -> [f32; 4] {
    texels[(y * SIZE + x) as usize]
}

fn assert_rgba(actual: [f32; 4], expected: [f32; 4], what: &str) {
    for channel in 0..4 {
        assert!(
            (actual[channel] - expected[channel]).abs() <= 1.0 / 512.0,
            "{what}: channel {channel} was {}, expected {} (full texel {actual:?})",
            actual[channel],
            expected[channel],
        );
    }
}

/// [`assert_rgba`] with a caller-supplied absolute tolerance, for values whose
/// magnitude makes binary16's own rounding the floor (the decoded linear
/// path lands near 25.3 for an encoded 4.0).
fn assert_rgba_within(actual: [f32; 4], expected: [f32; 4], tolerance: f32, what: &str) {
    for channel in 0..4 {
        assert!(
            (actual[channel] - expected[channel]).abs() <= tolerance,
            "{what}: channel {channel} was {}, expected {} ± {tolerance} (full texel {actual:?})",
            actual[channel],
            expected[channel],
        );
    }
}

/// Fill the whole screen with a hand-built HDR mesh.
fn full_screen_hdr_mesh(color: HdrVertexColor) -> Shape {
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(SIZE as f32));
    let mut mesh = Mesh::default();
    mesh.add_colored_rect_hdr(rect, color);
    Shape::mesh(mesh)
}

/// A 4.0 vertex survives the fp16 pipeline and reads back as 4.0.
#[test]
fn hdr_mesh_vertex_of_four_reads_back_as_four() {
    let hdr = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
    let texels = render_float(1, |ui| {
        ui.painter().add(full_screen_hdr_mesh(hdr));
    });

    let center = texel(&texels, SIZE / 2, SIZE / 2);
    assert_rgba(center, [4.0, 4.0, 4.0, 1.0], "4.0 vertex");
    assert!(
        center[0] > 1.0,
        "the point of the tier: the value is above SDR white"
    );
}

/// The same value through the MSAA-4× resolve path.
#[test]
fn hdr_mesh_vertex_of_four_survives_msaa_4x() {
    let hdr = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
    let texels = render_float(4, |ui| {
        ui.painter().add(full_screen_hdr_mesh(hdr));
    });

    let center = texel(&texels, SIZE / 2, SIZE / 2);
    assert_rgba(center, [4.0, 4.0, 4.0, 1.0], "4.0 vertex at MSAA 4×");
}

/// The odd-symmetric negative extension survives to the readback.
#[test]
fn hdr_mesh_negative_vertex_reads_back_as_negative() {
    let hdr = HdrVertexColor::from_rgba_premultiplied([-2.0, -2.0, -2.0, 1.0]);
    let texels = render_float(1, |ui| {
        ui.painter().add(full_screen_hdr_mesh(hdr));
    });

    let center = texel(&texels, SIZE / 2, SIZE / 2);
    assert_rgba(center, [-2.0, -2.0, -2.0, 1.0], "-2.0 vertex");
    assert!(center[0] < 0.0, "negatives must not be clamped on wgpu");
}

/// The per-position HDR mode interpolates above 1.0: a 1.0 → 6.0 ramp's
/// midpoint is halfway, not clamped.
#[test]
fn hdr_uv_gradient_midpoint_is_3_5() {
    let texels = render_float(1, |ui| {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(SIZE as f32));
        ui.painter().add_hdr_uv(
            Shape::rect_filled(rect, 0.0, Color32::WHITE),
            |_bounds, pos| {
                let t = (pos.x - 0.0) / SIZE as f32;
                let value = 1.0 + 5.0 * t; // 1.0 at the left edge, 6.0 at the right
                HdrVertexColor::from_rgba_premultiplied([value, value, value, 1.0])
            },
        );
    });

    // Vertex values are 1.0 and 6.0 at x = 0 and x = 64; sampling the pixel
    // centre at x = 32 lands halfway, 3.5 (32.5/64 = 0.5078 → 3.539).
    let center = texel(&texels, SIZE / 2, SIZE / 2);
    let expected = 1.0 + 5.0 * ((SIZE / 2) as f32 + 0.5) / SIZE as f32;
    assert_rgba(
        center,
        [expected, expected, expected, 1.0],
        "1.0→6.0 ramp midpoint",
    );
    assert!(center[0] > 1.0, "the midpoint is above SDR white");
    assert!(center[0] < 6.0, "the midpoint is below the ramp's top");
}

/// The same ramp through the MSAA-4× resolve path.
#[test]
fn hdr_uv_gradient_midpoint_survives_msaa_4x() {
    let texels = render_float(4, |ui| {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(SIZE as f32));
        ui.painter().add_hdr_uv(
            Shape::rect_filled(rect, 0.0, Color32::WHITE),
            |_bounds, pos| {
                let t = (pos.x - 0.0) / SIZE as f32;
                let value = 1.0 + 5.0 * t;
                HdrVertexColor::from_rgba_premultiplied([value, value, value, 1.0])
            },
        );
    });

    let center = texel(&texels, SIZE / 2, SIZE / 2);
    let expected = 1.0 + 5.0 * ((SIZE / 2) as f32 + 0.5) / SIZE as f32;
    assert_rgba(
        center,
        [expected, expected, expected, 1.0],
        "1.0→6.0 ramp midpoint at MSAA 4×",
    );
}

/// HDR text is first-class: the glyph cores read above SDR white.
#[test]
fn hdr_label_glyph_cores_exceed_sdr_white() {
    let hdr = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
    let texels = render_float(1, |ui| {
        ui.hdr_label(hdr, RichText::new("W").size(48.0));
    });

    let peak = texels.iter().map(|texel| texel[0]).fold(0.0_f32, f32::max);
    assert!(
        peak > 2.0,
        "an HDR label at 4.0 must show glyph cores well above white, peak was {peak}"
    );
}

/// A frame with both tiers switches pipelines (and the slot-1 binding) in
/// both directions without a device error, and each mesh keeps its values.
#[test]
fn sdr_and_hdr_primitives_coexist_in_one_frame() {
    let hdr = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
    let texels = render_float(1, |ui| {
        let left = Rect::from_min_max(Pos2::ZERO, pos2(SIZE as f32 / 2.0, SIZE as f32));
        let right = Rect::from_min_max(
            pos2(SIZE as f32 / 2.0, 0.0),
            Vec2::splat(SIZE as f32).to_pos2(),
        );
        // SDR, HDR, SDR: the last mesh must not inherit the HDR pipeline.
        ui.painter().rect_filled(left, 0.0, Color32::WHITE);
        ui.painter().rect_filled_hdr(right, 0.0, hdr);
        ui.painter().rect_filled(
            Rect::from_min_max(
                pos2(0.0, SIZE as f32 - 4.0),
                Vec2::splat(SIZE as f32).to_pos2(),
            ),
            0.0,
            Color32::from_gray(128),
        );
    });

    assert_rgba(
        texel(&texels, SIZE / 4, SIZE / 4),
        [1.0, 1.0, 1.0, 1.0],
        "SDR half",
    );
    assert_rgba(
        texel(&texels, 3 * SIZE / 4, SIZE / 4),
        [4.0, 4.0, 4.0, 1.0],
        "HDR half",
    );
    // The SDR mesh drawn after the HDR one still uses the SDR pipeline and its
    // u8 color, not the stale sidecar.
    let bottom = texel(&texels, SIZE / 2, SIZE - 2);
    assert!(
        (bottom[0] - 128.0 / 255.0).abs() < 2.0 / 255.0,
        "the trailing SDR mesh must read its own u8 colour, was {bottom:?}"
    );
}

/// An HDR stroke draws: the wrapper recolours the path tessellation's coverage
/// and the interior reads above white.
#[test]
fn hdr_line_segment_draws_above_white() {
    let hdr = HdrVertexColor::from_rgba_premultiplied([3.0, 3.0, 3.0, 1.0]);
    let texels = render_float(1, |ui| {
        ui.painter().line_segment_hdr(
            [
                pos2(4.0, SIZE as f32 / 2.0),
                pos2(SIZE as f32 - 4.0, SIZE as f32 / 2.0),
            ],
            egui::Stroke::new(8.0, Color32::WHITE),
            hdr,
        );
    });

    let on_the_line = texel(&texels, SIZE / 2, SIZE / 2);
    assert!(
        on_the_line[0] > 2.0,
        "the stroke interior must read near 3.0, was {on_the_line:?}"
    );
    // Outside the stroke is still transparent, so the geometry did not spread.
    assert_eq!(texel(&texels, SIZE / 2, 2), [0.0, 0.0, 0.0, 0.0]);
}

/// §12: every numeric row must ride **both** fragment entry points. `Srgb`
/// selects `fs_main_gamma_framebuffer`, the scRGB space
/// `fs_main_linear_framebuffer`; the physical colour is identical, so the
/// linear read-back is the gamma-domain value decoded once. The expectation is
/// derived from [`OutputEncoding`], never hard-coded, so a table change moves
/// the test with it.
#[test]
fn both_fragment_entry_points_show_the_same_physical_color() {
    use egui_wgpu::OutputEncoding;

    let format = wgpu::TextureFormat::Rgba16Float;
    let gamma_space = wgpu::SurfaceColorSpace::Srgb;
    let linear_space = wgpu::SurfaceColorSpace::ExtendedSrgbLinear;
    assert_eq!(
        OutputEncoding::of(format, gamma_space),
        OutputEncoding::GammaEncoded,
        "the gamma entry point is the one the other rows already ride"
    );
    assert_eq!(
        OutputEncoding::of(format, linear_space),
        OutputEncoding::Linear,
        "and this space is what exercises the linear entry point"
    );

    // What the framebuffer stores for a gamma-domain value on `space`.
    let stored =
        |space: wgpu::SurfaceColorSpace, gamma: f32| match OutputEncoding::of(format, space) {
            OutputEncoding::GammaEncoded => gamma,
            OutputEncoding::Linear => egui::ecolor::linear_from_gamma(gamma),
        };

    for space in [gamma_space, linear_space] {
        // A 4.0 vertex.
        let hdr = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
        let texels = render_float_on(Some(space), 1, |ui| {
            ui.painter().add(full_screen_hdr_mesh(hdr));
        });
        let expected = stored(space, 4.0);
        assert_rgba_within(
            texel(&texels, SIZE / 2, SIZE / 2),
            [expected, expected, expected, 1.0],
            (expected * 1e-3).max(1.0 / 512.0),
            &format!("4.0 vertex on {space:?}"),
        );

        // The odd-symmetric negative: it must stay negative on both paths.
        let hdr = HdrVertexColor::from_rgba_premultiplied([-2.0, -2.0, -2.0, 1.0]);
        let texels = render_float_on(Some(space), 1, |ui| {
            ui.painter().add(full_screen_hdr_mesh(hdr));
        });
        let expected = stored(space, -2.0);
        assert_rgba_within(
            texel(&texels, SIZE / 2, SIZE / 2),
            [expected, expected, expected, 1.0],
            (expected.abs() * 1e-3).max(1.0 / 512.0),
            &format!("-2.0 vertex on {space:?}"),
        );

        // The ramp midpoint: interpolated above white, then encoded.
        let texels = render_float_on(Some(space), 1, |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(SIZE as f32));
            ui.painter().add_hdr_uv(
                Shape::rect_filled(rect, 0.0, Color32::WHITE),
                |_bounds, pos| {
                    let value = 1.0 + 5.0 * (pos.x / SIZE as f32);
                    HdrVertexColor::from_rgba_premultiplied([value, value, value, 1.0])
                },
            );
        });
        let midpoint = 1.0 + 5.0 * ((SIZE / 2) as f32 + 0.5) / SIZE as f32;
        let expected = stored(space, midpoint);
        assert_rgba_within(
            texel(&texels, SIZE / 2, SIZE / 2),
            [expected, expected, expected, 1.0],
            (expected.abs() * 1e-3).max(1.0 / 512.0),
            &format!("1.0→6.0 ramp midpoint on {space:?}"),
        );
    }

    // And the two paths really do differ: the linear one is the decoded value,
    // not a second gamma write.
    let gamma = stored(gamma_space, 4.0);
    let linear = stored(linear_space, 4.0);
    assert!(
        (gamma - linear).abs() > 1.0,
        "the entry points must not coincide: gamma {gamma}, linear {linear}"
    );
}

/// §12 GPU numeric + §14 6b "feathering through the mask": the wrapper scales
/// the coverage the tessellator produced, so the antialiased ring is a value
/// strictly between zero and the interior — a real midpoint, not a clamped
/// mask. Both the plain and the MSAA-4× resolve path.
///
/// A circle rather than a rect: `rect_filled` snaps integer-aligned edges to
/// the pixel grid, which leaves no partial pixel at all. A radius of 15.5 puts
/// the left boundary at x = 16.5, putting the 1-px AA ramp (`±0.5 px` around
/// the path) symmetrically across the boundary, so the pixel centred at 16.5
/// is half covered by construction rather than by luck.
#[test]
fn hdr_feathered_edge_midpoint_is_between_zero_and_the_interior() {
    for msaa in [1, 4] {
        let hdr = HdrVertexColor::from_rgba_premultiplied([4.0, 4.0, 4.0, 1.0]);
        let texels = render_float_on(None, msaa, |ui| {
            ui.painter().add_hdr(
                Shape::circle_filled(pos2(32.0, 32.0), 15.5, Color32::WHITE),
                hdr,
            );
        });

        let interior = texel(&texels, 32, 32);
        assert_rgba(
            interior,
            [4.0, 4.0, 4.0, 1.0],
            &format!("the interior at MSAA {msaa}×"),
        );

        let edge = texel(&texels, 16, 32);
        assert!(
            edge[0] > 0.05 && edge[0] < interior[0] - 0.05,
            "the antialiased edge must fade in HDR at MSAA {msaa}×: edge {edge:?}, \
             interior {interior:?}"
        );
        assert!(
            (edge[0] - 2.0).abs() < 0.6,
            "half coverage of a 4.0 value is ~2.0 at MSAA {msaa}×, was {}",
            edge[0]
        );
        // The alpha channel feathers with it: coverage, not a colour clamp.
        assert!(
            edge[3] > 0.2 && edge[3] < 0.8,
            "the edge's alpha is partial coverage at MSAA {msaa}×, was {}",
            edge[3]
        );
        // Outside the ring is transparent: the geometry did not spread.
        assert_eq!(
            texel(&texels, 8, 32)[3],
            0.0,
            "nothing outside the feathered edge at MSAA {msaa}×"
        );
    }
}

// ---------------------------------------------------------------------------
// Snapshots: the u8 proof of the twin
// ---------------------------------------------------------------------------

/// §12's Snapshot row. A u8 PNG cannot show above 1.0 by definition, so this
/// pins the *other* half of the contract: the HDR gradient draws its SDR twin,
/// range-reduced uniformly (hue preserved, not per-channel clamped).
#[cfg(feature = "snapshot")]
#[test]
fn hdr_gradient_snapshot() {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::splat(64.0))
        .build_ui(|ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(64.0));
            ui.painter().add_hdr_uv(
                Shape::rect_filled(rect, 0.0, Color32::WHITE),
                |_bounds, pos| {
                    // 6.0 → 0.5 in red, with green/blue ratios that survive
                    // the range reduction.
                    let t = pos.x / 64.0;
                    HdrVertexColor::from_linear([0.5 + 5.5 * t, 1.0, 2.0 - 1.5 * t, 1.0])
                },
            );
        });
    harness.snapshot("hdr_gradient");
}

/// §12's Snapshot row for text: the glyphs' twin, so an SDR-only backend's
/// HDR label is visibly the range-reduced colour rather than SDR white.
#[cfg(feature = "snapshot")]
#[test]
fn hdr_label_snapshot() {
    let hdr = HdrVertexColor::from_linear([4.0, 2.0, 0.5, 1.0]);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(120.0, 48.0))
        .build_ui(move |ui| {
            ui.hdr_label(hdr, RichText::new("HDR").size(32.0));
        });
    harness.snapshot("hdr_label");
}

// ---------------------------------------------------------------------------
// Public-API CPU rows (the unit-level rows live beside the code)
// ---------------------------------------------------------------------------

#[test]
fn projection_is_legal_and_preserves_ratios() {
    let color = HdrVertexColor::from_linear([4.0, 2.0, 0.5, 1.0]);
    let twin = color.projected_color32();
    let [r, g, b, a] = twin.to_array();
    assert!(r <= a && g <= a && b <= a);
    assert_eq!((r, g, b, a), (255, twin.g(), twin.b(), 255));
    assert!(g > b, "the 4:2:0.5 ratio survives the scale");
}

#[test]
fn shape_size_guard_still_holds() {
    assert_eq!(core::mem::size_of::<Shape>(), 64);
    assert_eq!(core::mem::size_of::<Vertex>(), 20);
}

#[test]
fn sidecar_invariant_and_promotion_are_public_contracts() {
    let mut mesh = Mesh::default();
    mesh.add_colored_rect(
        Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)),
        Color32::from_rgb(200, 100, 50),
    );
    assert!(!mesh.is_hdr());
    assert!(mesh.is_valid());

    let promoted = mesh.promote_to_hdr().len();
    assert_eq!(promoted, 4);
    assert_eq!(mesh.promote_to_hdr().len(), 4, "idempotent");
    assert!(mesh.is_valid());

    mesh.hdr_colors = Some(Vec::new());
    assert!(!mesh.is_valid(), "a short sidecar is invalid");
}

#[test]
fn gradient_rect_hdr_matches_the_sdr_gradient() {
    let colors = [
        Color32::from_rgba_unmultiplied(200, 100, 50, 200),
        Color32::from_rgba_unmultiplied(10, 20, 30, 100),
    ];
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0));
    let sdr = Shape::gradient_rect(rect, egui::Direction::LeftToRight, colors);
    let hdr = Shape::gradient_rect_hdr(
        rect,
        egui::Direction::LeftToRight,
        colors.map(HdrVertexColor::from),
    );

    let (Shape::Mesh(sdr), Shape::Mesh(hdr)) = (sdr, hdr) else {
        panic!("both gradients are meshes");
    };
    assert_eq!(sdr.indices, hdr.indices);
    for (sdr, hdr) in core::iter::zip(&sdr.vertices, &hdr.vertices) {
        assert_eq!(sdr.pos, hdr.pos);
        assert_eq!(sdr.uv, hdr.uv);
    }
}

#[test]
fn hdr_shape_is_its_own_primitive() {
    let ctx = egui::Context::default();
    let raw_input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(SIZE as f32))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw_input, |ui| {
        ui.painter().rect_filled(ui.max_rect(), 0.0, Color32::WHITE);
        ui.painter()
            .rect_filled_hdr(ui.max_rect(), 0.0, HdrVertexColor::WHITE);
    });
    // This test only inspects the geometry, so it never uploads.
    output.textures_delta.clear();
    let jobs = ctx.tessellate(output.shapes, 1.0);
    let tiers: Vec<bool> = jobs
        .iter()
        .filter_map(|job| match &job.primitive {
            Primitive::Mesh(mesh) => Some(mesh.is_hdr()),
            Primitive::Callback(_) => None,
        })
        .collect();
    assert_eq!(tiers, [false, true], "the tier joins the batching key");
}
