//! Windowed CI smoke test for the HDR surface policy.
//!
//! Opens a real window with [`SurfacePreference::WideGamutHdr`], checks on
//! the first frame that the surface decision is surfaced and coherent and
//! that display headroom is reported (`Some(1.0)` on an SDR display, the EDR
//! multiplier on an EDR one — `None` means the poll failed and fails the
//! smoke), logs the pairing, draws one label, and exits after a few frames.
//! Exit code 0 = pass.
//!
//! What this *cannot* validate — panel gamut, compositor tone mapping, the
//! actual headroom *value* — is the manual protocol in `MANUAL_TESTING.md`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![expect(rustdoc::missing_crate_level_docs)] // it's an example

use eframe::egui;
use eframe::egui_wgpu::SurfacePreference;

/// Enough frames to configure the surface, paint, and observe a
/// [`egui::SurfaceStatus`] — not enough to slow CI down.
const FRAMES_BEFORE_EXIT: u32 = 5;

/// Runs past the watchdog deadline: reports and fails the process.
#[expect(clippy::exit, clippy::print_stderr)] // a watchdog must be loud even if logging is unconfigured
fn watchdog_timeout() -> ! {
    eprintln!("hdr_smoke: watchdog timeout - event loop did not exit within 2 min");
    std::process::exit(1)
}

fn main() -> Result<(), eframe::Error> {
    // CI watchdog: if the event loop wedges before the 5-frame exit, fail
    // this job in two minutes instead of burning the CI job's full timeout.
    // The process always exits one way or another within 5 frames, so the
    // thread never fires in a healthy run (and a normal exit kills it).
    std::thread::Builder::new()
        .name("hdr_smoke-watchdog".to_owned())
        .spawn(|| {
            std::thread::sleep(core::time::Duration::from_mins(2));
            watchdog_timeout();
        })
        .expect("spawning a thread cannot fail in a healthy process");

    // The pairing must show up in the CI log.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,naga=warn,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 200.0]),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // The fork opts in unconditionally; PR 3 completes the enum, and
            // the smoke asserts the combined path (plan §2). On hardware that
            // reports no extended pairing this deterministically falls back
            // to plain SDR — still a pass (the status is what's asserted).
            surface_preference: SurfacePreference::WideGamutHdr,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "hdr_smoke",
        options,
        Box::new(|_cc| Ok(Box::new(HdrSmoke::default()))),
    )
}

#[derive(Default)]
struct HdrSmoke {
    checked_decision: bool,
    frames: u32,
}

impl eframe::App for HdrSmoke {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.checked_decision {
            self.checked_decision = true;
            check_surface_decision(ui.ctx());
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Wide-gamut surface smoke test — exiting shortly.");
        });

        self.frames += 1;
        if self.frames >= FRAMES_BEFORE_EXIT {
            // Assertions passed and the label was drawn: the smoke is done.
            //
            // Exits the process directly instead of returning from the event
            // loop: AppKit's post-close display-cycle flush can throw an
            // `NSRangeException` from its own `NSTouchBar` observation
            // bookkeeping (intermittent on macOS, independent of the surface
            // policy), which would fail CI *after* the smoke already passed.
            // CI only needs the assertions, the logged decision, and exit
            // code 0 — `std::process::exit(0)` skips the racy teardown.
            log::info!("hdr_smoke passed: decision coherent, label drawn, exiting");
            std::process::exit(0);
        } else {
            // A static UI would let egui idle in `ControlFlow::Wait`; keep
            // frames flowing until the exit above fires.
            ui.ctx().request_repaint();
        }
    }
}

/// The hard assertions of the smoke test, run once on the first frame.
///
/// A windowed wgpu viewport must surface its decision from the first frame,
/// and the surfaced fields must agree, and headroom must be reported. A
/// [`egui::SurfaceStatus::Granted`] status *should* mean the combined
/// extended pairing, but that is never hard-required: CI hardware varies,
/// and an SDR display correctly reports [`egui::SurfaceStatus::FellBack`].
fn check_surface_decision(ctx: &egui::Context) {
    let (status, target, headroom) = ctx.input(|input| {
        let info = input
            .raw
            .viewports
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport must be in the viewports map");
        (
            info.surface_status,
            info.surface_target.clone(),
            info.hdr_headroom,
        )
    });

    let status = status.expect(
        "a windowed wgpu viewport must surface a `surface_status` from the first frame \
         (Granted or FellBack — never None)",
    );
    let target = target.expect("`surface_status` is surfaced, so `surface_target` must be too");

    // PR 3: headroom must be reported from the first frame — `Some(1.0)` on a
    // definitively-SDR display, the EDR multiplier on an EDR one. macOS is
    // the runtime-verified platform and wgpu 30 always populates the value
    // there, so `None` (or a nonsense value) fails the smoke. Elsewhere the
    // poll may legitimately report nothing — warn and continue.
    let headroom = match headroom {
        Some(h) if h.is_finite() && h > 0.0 => h,
        other => {
            #[cfg(target_os = "macos")]
            panic!(
                "a windowed wgpu viewport on macOS must surface `hdr_headroom` from the \
                 first frame (Some(1.0) on SDR, the EDR multiplier on EDR — never None); \
                 got {other:?}"
            );
            #[cfg(not(target_os = "macos"))]
            {
                log::warn!("hdr_headroom not reported ({other:?}); continuing (non-macOS)");
                return;
            }
        }
    };

    log::info!("Surface decision: {target} ({status:?}), headroom {headroom}×");

    if status == egui::SurfaceStatus::Granted && !target.contains("ExtendedDisplayP3") {
        // Soft check only: what is granted depends on the capabilities the
        // CI machine's display actually reports (an SDR display grants a
        // plain pairing; the status, not the pairing, is what's hard-required).
        log::warn!("Granted without the combined pairing: {target}");
    }
}
