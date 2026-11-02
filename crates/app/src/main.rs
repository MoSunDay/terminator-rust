//! terminator-rust: a terminator-style terminal multiplexer front-end.

mod actions;
mod app_icon;
mod attention;
mod input;
mod ipc;
mod persist;
mod render;
mod session_map;
mod state;
mod terminal_theme;
mod ui;
mod windows;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use egui::ViewportBuilder;
use log::warn;

use crate::state::{data, fresh_keep_prefs, fresh_state, AppState, Data};

/// Monotonic process origin shared by every pacing measurement.
static PROCESS_START: OnceLock<Instant> = OnceLock::new();
/// Start of the last ROOT pass, in ms since [`PROCESS_START`].
static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);
/// egui's `predicted_dt` (microseconds), refreshed at the start of every
/// root pass; pre-seeded with the fallback so an early wake paces sanely.
static PREDICTED_DT_US: AtomicU64 = AtomicU64::new(render::stats::PREDICTED_DT_FALLBACK_US);

/// The process-start instant (first use wins).
fn process_start() -> &'static Instant {
    PROCESS_START.get_or_init(Instant::now)
}

/// Milliseconds since [`process_start`].
fn since_start_ms() -> u64 {
    process_start().elapsed().as_millis().min(u64::MAX as u128) as u64
}

struct Terminator {
    data: Data,
    path: std::path::PathBuf,
    /// Control socket (UDS) when startup succeeded; None = disabled.
    ipc: Option<ipc::server::Ipc>,
}

impl Terminator {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::fonts::install(&cc.egui_ctx);
        let path = persist::state_path();
        let mut st = launch_state(&path);
        // Compositor-less environments (Xvfb e2e, bare WMs) cannot blend a
        // translucent window: TERMINATOR_OPAQUE=1 pins full opacity so
        // pixels stay deterministic. Startup-only; the inspector slider
        // still works afterwards.
        if std::env::var_os("TERMINATOR_OPAQUE").as_deref() == Some(std::ffi::OsStr::new("1")) {
            st.settings.opacity = 1.0;
        }
        let reg_path = remote::default_registry_path();
        let registry = remote::load_registry(&reg_path);
        // The render loop is reactive (no repaint tick), so pty output and
        // IPC requests must wake it from their threads. Secondary
        // viewports render inside the root pass, so waking ROOT refreshes
        // every window. egui 0.36 `request_repaint_after`: a delay of
        // exactly ZERO arms `outstanding = 1`, which makes the pass
        // starter fire the callback again - every pty output burst would
        // render TWICE; a NON-ZERO delay repaints once. `predicted_dt`
        // (~16.7ms) is subtracted from the delay AFTER that decision, so
        // the old fixed 1ms actually saturated to zero: immediate, single
        // pass, and no coalescing whatsoever. A delay ABOVE predicted_dt
        // is a REAL wait, so cadence::output_wake_paced waits out the
        // pacing floor (TERMINATOR_FRAME_FLOOR_MS, default 16ms ~ 60fps)
        // since the last frame before asking again - batches of output
        // chunks coalesce into one frame instead of one frame each, with
        // headroom left on high-refresh displays and Xvfb.
        let mut d = data(st, registry);
        let wake_ctx = cc.egui_ctx.clone();
        d.sess.wake = Some(Arc::new(move || {
            // Time since the last root pass started, ms-quantized (fine
            // for a 16ms floor; see the statics above).
            let since_last_frame = Duration::from_millis(since_start_ms())
                .saturating_sub(Duration::from_millis(LAST_FRAME_MS.load(Ordering::Relaxed)));
            let predicted = Duration::from_micros(PREDICTED_DT_US.load(Ordering::Relaxed).max(1));
            // Output pacing floor: the base 16ms coalescing window,
            // raised by focus (unfocused 50ms) and - unless an explicit
            // TERMINATOR_FRAME_FLOOR_MS pins e2e cadence - by the
            // adaptive CPU budget (render/frame_cpu.rs): a measured pass
            // may not cost more than TERMINATOR_CPU_TARGET_PCT of a core
            // per second of output. Human interaction opens a 900ms
            // 60fps window, so only unattended streams are throttled.
            let base = render::stats::output_floor();
            let floor = if render::stats::governor_enabled() {
                render::cadence::budgeted_floor(
                    base,
                    render::frame_cpu::ema_us(),
                    render::stats::cpu_target_pct(),
                    render::frame_cpu::interactive(),
                )
            } else {
                render::cadence::pacing_floor(base)
            };
            let delay = render::cadence::output_wake_paced(since_last_frame, predicted, floor);
            render::cadence::request_root_after(&wake_ctx, delay)
        }));
        Self {
            data: d,
            path,
            ipc: ipc::server::start(&cc.egui_ctx),
        }
    }

    fn save_if_dirty(&mut self) {
        if !self.data.dirty {
            return;
        }
        if let Err(e) = persist::save(&self.path, &self.data.st) {
            warn!("save state: {e}");
        }
        self.data.dirty = false;
    }
}

impl eframe::App for Terminator {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Frame instrumentation anchors: `t0` times the WHOLE root pass
        // (every window renders inside it) and only surfaces with
        // TERMINATOR_FRAME_STATS=1; the two atomics feed the pty wake
        // pacing (last pass start + egui's frame-time prediction).
        // Sample the CPU the PREVIOUS pass spent (tessellation and submit
        // happen after `App::ui` returns, so a sample at the end would miss
        // them). The EMA drives the output frame budget (frame_cpu.rs).
        render::frame_cpu::tick();
        let t0 = Instant::now();
        let ctx = ui.ctx().clone();
        LAST_FRAME_MS.store(since_start_ms(), Ordering::Relaxed);
        PREDICTED_DT_US.store(
            render::stats::predicted_dt_us(ctx.input(|i| i.predicted_dt)),
            Ordering::Relaxed,
        );
        // Any real input (key, pointer, wheel, resize, focus-delivered
        // event) is human interaction: keep output frames at 60fps for the
        // next 900ms instead of letting the CPU budget stretch them.
        if ctx.input(|i| !i.events.is_empty() || i.pointer.any_down()) {
            render::frame_cpu::mark_interaction();
        }
        // Root viewport focus/minimized drive the wake-path pacing floors
        // (unfocused floor, hidden heartbeat); refresh before any pump of
        // this frame can schedule the next one.
        let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
        render::cadence::observe(focused, minimized);
        // Keep the WM max-size constraint glued to the real GPU limit
        // (root viewport; secondaries guard inside their own pass).
        render::surface_guard::apply(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_if_dirty();
        }

        let theme = self.data.st.theme_name.clone();
        let font_size = self.data.st.settings.font_size;
        ui::style::sync(&ctx, &theme, font_size, &mut self.data.ui);

        // The control socket and per-window passes all act on the window
        // the user is focused on (set inside windows::render).
        self.data.st.active = self.data.st.focus;
        if let Some(i) = self.ipc.as_mut() {
            ipc::server::drain(i, &mut self.data);
        }

        // Root window: a plain render pass into the root viewport.
        // Frame-scoped bookkeeping, NOT per window pass (W windows used to
        // mean W pumps and theme syncs per frame): keep VT colors aligned
        // with the painted theme, then drain every session's pty output
        // once, before any pass renders a frame snapshot.
        let pal = render::colors::palette_of(&theme);
        terminal_theme::sync(&mut self.data.sess, &pal, self.data.st.settings.bg_color);
        let mut noticed = false;
        for pane in session_map::pump_all(&mut self.data.sess) {
            self.data.ui.notices.insert(pane);
            noticed = true;
        }
        if noticed {
            // A notification is user-facing: do not budget its frame away.
            render::frame_cpu::mark_interaction();
        }
        windows::render(ui, &mut self.data, 0);
        ui::inspector::show(&ctx, &mut self.data, 0);
        // Registered AFTER the inspector so the modal sits on top of it.
        ui::close_dialog::show(&ctx, &mut self.data, 0);

        // Secondary windows: immediate viewports = real OS windows, one
        // render pass each (their keyboard input arrives in their own
        // pass inside windows::render). Removal shrinks the list; the loop
        // re-checks the slot so no window is skipped.
        windows::render_secondaries(&ctx, &mut self.data);
        // Panes spawned by this frame's ensure_sessions get their theme
        // defaults before the next frame's pump parses their first output.
        terminal_theme::sync(&mut self.data.sess, &pal, self.data.st.settings.bg_color);
        self.data.shapes.end_frame();
        self.data.st.active = self.data.st.focus;

        self.save_if_dirty();

        // Closing the last pane/tab of the last window flips ui.quitting:
        // ask the WM to close every frame until it lands (the close path
        // re-enters ui() once to save; ensure_sessions must not resurrect
        // a tab in between). Secondaries closing never sets this.
        if self.data.ui.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Single exit point (this function has no early returns - keep it
        // that way): the pass duration reaches the frame-stats gate last,
        // after everything measurable rendered.
        render::stats::record(t0.elapsed());
    }

    /// Transparent clear: the window is borderless and the panel fills are
    /// alpha-blended by `settings.opacity`, so the compositor shows the
    /// desktop through. Pixels are fully covered by the panels either way.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }
}

/// Startup state: a fresh window with a NEW tab by default (terminals
/// don't resurrect their last session on open); theme + settings still
/// carry over from the saved state. TERMINATOR_RESTORE=1 keeps the saved
/// windows/tabs instead (e2e presets and users who want it).
fn launch_state(path: &std::path::Path) -> AppState {
    let saved = persist::load(path).unwrap_or_else(fresh_state);
    if std::env::var_os("TERMINATOR_RESTORE").as_deref() == Some(std::ffi::OsStr::new("1")) {
        return saved;
    }
    fresh_keep_prefs(saved)
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Decode the embedded logo up front (one shared raster for every
    // viewport); a decode failure only costs the custom icon, never the
    // app - app_icon warns and the builders fall back to the default.
    if let Some(icon) = app_icon::icon() {
        log::debug!("window icon: {}x{} rgba", icon.width, icon.height);
    }
    let options = eframe::NativeOptions {
        // The embedded logo (assets/logo, gen-logo.py) becomes the window
        // icon: eframe publishes it as X11 _NET_WM_ICON / the macOS dock
        // icon (see app_icon - secondaries need their own copy).
        viewport: app_icon::with_icon(
            ViewportBuilder::default()
                .with_title("terminator-rust")
                .with_inner_size([1200.0, 800.0])
                // WM-level cap: a window whose PHYSICAL size exceeds the
                // GPU max texture extent aborts the process inside wgpu
                // surface configure (8192 on software adapters; 5K at 2x
                // scale already crosses it). surface_guard::apply
                // refines this to the real adapter limit each frame.
                .with_max_inner_size(render::surface_guard::safe_cap_points(
                    render::surface_guard::FALLBACK_MAX_TEXTURE_SIDE,
                    1.0,
                ))
                // Borderless by design (Chrome-style): the tab strip IS the
                // title bar - dragging the bare chrome moves the window via
                // ViewportCommand::StartDrag (winit's cross-platform
                // drag_window); closing via Ctrl+Shift+Q or the last pane.
                // `with_transparent` stays for window opacity + pane glass.
                .with_decorations(false)
                .with_transparent(true),
        ),
        // Dithering is a full-screen per-pixel hash in the wgpu present
        // path: pure cost for a terminal UI, and it is the documented
        // source of the "1-off column" noise in pixel-gate screenshots
        // (llvmpipe/NVIDIA present dithers fixed screen columns). Keep it
        // OFF; a pixel gate that starts flaking on 1-off deviations is a
        // regression here, not something to loosen.
        dithering: false,
        ..Default::default()
    };
    eframe::run_native(
        "terminator-rust",
        options,
        Box::new(|cc| Ok(Box::new(Terminator::new(cc)))),
    )
}
