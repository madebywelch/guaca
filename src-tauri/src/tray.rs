//! The menu bar presence: the icon, and the panel it opens.
//!
//! The second file that knows Tauri exists, and it earns that the same way
//! `app.rs` does: everything above it is a plain library. What the icon says
//! and where the panel stands are decided in `menubar.rs`, which has no idea a
//! platform exists and is tested without one. This file draws those decisions
//! and owns the panel's window.
//!
//! Three things here are less obvious than they look.
//!
//! **The icon is drawn from what the window reports.** The tray process holds
//! no workspace: agents run in a host, and the window is the client of it. So
//! the window hands over a presence, coalesced on its side and again here, and
//! the icon is only ever as current as the last report.
//!
//! **The panel is a second client, not a second view.** It is the frontend
//! again, on `menubar.html`, and it reads the host the window is attached to
//! for itself. It is made, hidden, once the window says which host that is,
//! and reloaded when the window says a different one, so a click opens a page
//! that is already drawn rather than one that starts connecting when asked.
//!
//! **The panel is an ordinary window.** A menu bar panel on macOS is usually
//! an `NSPanel`, which takes the keyboard without bringing its app forward, and
//! Tauri makes no such window. Making this one into one means changing its
//! class under the windowing library, which objc2 documents as undefined
//! unless the new class descends from the old one, and `NSPanel` does not
//! descend from tao's window. So opening the panel brings Guaca forward, as
//! clicking any of its windows would, and closing it from the panel hands the
//! keyboard back by hiding the app when the window is not open. What that
//! costs: the panel cannot be drawn over another app's full-screen space.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent, Wry,
};
use tokio::sync::Notify;

use crate::domain::ids::AgentId;
use crate::menubar::{self, Frame, Glyph, Look, Presence, Screen};

/// The icon's own id, so `app.rs` can ask whether it is there.
///
/// That question is load-bearing: closing the window hides it instead of
/// quitting, and an app with no window and no menu bar icon is an app the
/// operator cannot see, cannot reach and cannot stop. Hiding is conditional on
/// this existing.
pub const TRAY_ID: &str = "guac.menubar";

/// The channel the panel asks the window to go somewhere on.
///
/// Its own channel rather than a variant of `UiEvent`. That one is the runtime
/// telling the UI what happened; this is one surface asking another to open a
/// channel, and folding the two together would put a case in the transcript's
/// event handling for something the runtime never emits.
pub const REVEAL: &str = "guac://reveal";

/// The panel's window label, and the page it draws.
const PANEL: &str = "menubar";
const PAGE: &str = "menubar.html";

/// The one item in the right-click menu that is not the platform's own.
const OPEN: &str = "guac.open";

/// How long a burst of reports becomes one redraw.
///
/// A cascade changes what the window reports several times a second, and each
/// is a real change. Coalescing is what keeps that from being the icon redrawn
/// on the main thread ten times a second.
const COALESCE: Duration = Duration::from_millis(300);

/// How soon after the panel put itself away a click on the icon is the click
/// that put it away.
///
/// A click on the icon while the panel is open is, first, a click outside the
/// panel. Where that takes the focus before the click itself arrives, the
/// panel has already hidden, and reading the click as "open" would bring it
/// straight back.
const REOPEN: Duration = Duration::from_millis(300);

/// Where the panel is asking the window to go.
///
/// Two destinations, and neither is the other's fallback: an agent is
/// `select`, which follows it into whatever crew it is in, and For you is the
/// desk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Reveal {
    ForYou,
    Agent { id: AgentId },
}

/// The workspace the window is attached to: where it answers and what opens it.
///
/// No `Debug`, because the token is a credential and a derived `Debug` is one
/// `{:?}` in a log line away from printing it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub origin: String,
    pub token: String,
}

/// What placing the panel needs to remember between one call and the next.
#[derive(Default)]
struct Spot {
    /// The icon, in pixels, as of the click that last opened the panel.
    icon: Option<Frame>,
    /// How tall the page last said it wants to be.
    wanted: Option<f64>,
    /// When the panel last put itself away on losing focus.
    blurred: Option<Instant>,
}

pub struct Tray {
    app: AppHandle,
    icon: TrayIcon<Wry>,
    drawn: Mutex<Look>,
    wake: Arc<Notify>,
    /// What the window last reported.
    fed: Mutex<Option<Presence>>,
    /// The host the window is attached to, which the panel shows too.
    host: Mutex<Option<Host>>,
    spot: Mutex<Spot>,
}

impl Tray {
    /// Puts the icon in the menu bar and starts keeping it current.
    pub fn install(app: &AppHandle) -> tauri::Result<Arc<Self>> {
        let look = Presence::default().look();
        let (image, template) = glyph(look.glyph);

        // The two things worth doing when the panel is not what is wanted: the
        // window, and quitting. The platform's own quit, so it behaves like
        // every other app's and picks up the accelerator the operator knows.
        let menu = Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, OPEN, "Open Guaca", true, None::<&str>)?,
                &PredefinedMenuItem::separator(app)?,
                &PredefinedMenuItem::quit(app, Some("Quit Guaca"))?,
            ],
        )?;

        let icon = TrayIconBuilder::with_id(TRAY_ID)
            .icon(image)
            // macOS tints a template image to match the menu bar, in either
            // appearance and while the bar is highlighted. Giving that up is
            // the price of the one glyph that has a color, and `Look` is what
            // decides which of the two this is.
            .icon_as_template(template)
            .title(look.title.clone().unwrap_or_default())
            .tooltip(&look.tooltip)
            .menu(&menu)
            // Left click is the panel. The menu is the right click.
            .show_menu_on_left_click(false)
            .build(app)?;

        let tray = Arc::new(Self {
            app: app.clone(),
            icon,
            drawn: Mutex::new(look),
            wake: Arc::new(Notify::new()),
            fed: Mutex::new(None),
            host: Mutex::new(None),
            spot: Mutex::new(Spot::default()),
        });

        {
            let clicks = tray.clone();
            tray.icon.on_menu_event(move |_app, event| {
                if event.id().as_ref() == OPEN {
                    clicks.open_window(None);
                }
            });
        }

        {
            let clicks = tray.clone();
            tray.icon.on_tray_icon_event(move |_icon, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } = event
                {
                    clicks.toggle(rect);
                }
            });
        }

        {
            let tray = tray.clone();
            let wake = tray.wake.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    wake.notified().await;
                    // The burst, not the first report in it. Everything that
                    // arrives while this sleeps is already counted, and
                    // `Notify` holds one permit, so the next pass runs
                    // immediately and coalesces the next burst.
                    tokio::time::sleep(COALESCE).await;

                    // Off the async pool. Every icon call hops to the main
                    // thread and blocks on the answer, so a redraw that ran
                    // here would hold an executor thread for as long as the
                    // main thread was busy.
                    let redraw = tray.clone();
                    if let Err(err) =
                        tauri::async_runtime::spawn_blocking(move || redraw.redraw()).await
                    {
                        tracing::warn!(%err, "the menu bar icon stopped keeping itself current");
                        return;
                    }
                }
            });
        }

        Ok(tray)
    }

    /// Takes what the window is showing, for the icon to draw.
    pub fn feed(&self, presence: Option<Presence>) {
        *self.fed.lock() = presence;
        self.wake.notify_one();
    }

    /// Makes the icon agree with the last report.
    fn redraw(&self) {
        let look = match self.fed.lock().as_ref() {
            Some(presence) => presence.look(),
            None => Presence::default().look(),
        };
        let mut drawn = self.drawn.lock();
        // Committed only when it all landed: recording a change that failed
        // leaves the icon stale for good, because the next pass compares
        // against what it thinks it wrote.
        if look != *drawn && self.wear(&look) {
            *drawn = look;
        }
    }

    /// The glyph, the count beside it, and the line on hover. True when all
    /// three landed.
    fn wear(&self, look: &Look) -> bool {
        let (image, template) = glyph(look.glyph);
        [
            // Both at once, because they disagree for one frame otherwise and
            // the frame they disagree on is a red glyph tinted back to
            // monochrome.
            self.icon.set_icon_with_as_template(Some(image), template),
            // Empty rather than `None`: a title that is set and then cleared
            // has to be cleared with something, and on every platform this is
            // the same call.
            self.icon.set_title(look.title.as_deref().or(Some(""))),
            self.icon.set_tooltip(Some(&look.tooltip)),
        ]
        .into_iter()
        // Every one attempted and every failure logged, rather than stopping at
        // the first: the glyph landing and the title not is the state that would
        // read as a bug, and it has to be visible in the log as one.
        .fold(true, |all, result| match result {
            Ok(()) => all,
            Err(err) => {
                tracing::debug!(%err, "could not change the menu bar icon");
                false
            }
        })
    }

    /// Takes the host the window is attached to, and makes the panel show it.
    ///
    /// The same host twice is nothing: the window reports it every time it
    /// loads, and reloading the panel for each would drop whatever the operator
    /// was typing in it.
    pub fn attach(self: &Arc<Self>, host: Host) {
        {
            let mut held = self.host.lock();
            if held.as_ref() == Some(&host) {
                return;
            }
            *held = Some(host);
        }
        match self.app.get_webview_window(PANEL) {
            // Reloaded rather than told: the page reads its host once, at
            // load, like the window does, so there is nothing to update in
            // place and nothing that can be half updated.
            Some(panel) => match panel.reload() {
                Ok(()) => {
                    tracing::info!("the menu bar panel is showing the window's new workspace")
                }
                Err(err) => {
                    tracing::warn!(%err, "the menu bar panel is still showing the last workspace")
                }
            },
            None => match self.build_panel() {
                Ok(()) => tracing::info!("the menu bar panel is ready for the window's workspace"),
                Err(err) => {
                    tracing::warn!(%err, "no menu bar panel this session; the icon opens the window")
                }
            },
        }
    }

    /// The host the panel is to show, or nothing before the window has said.
    pub fn host(&self) -> Option<Host> {
        self.host.lock().clone()
    }

    /// Makes the panel's window, hidden, drawing its page.
    fn build_panel(self: &Arc<Self>) -> tauri::Result<()> {
        let panel = WebviewWindowBuilder::new(&self.app, PANEL, WebviewUrl::App(PAGE.into()))
            .title("Guaca")
            .inner_size(menubar::PANEL_WIDTH, menubar::PANEL_FIRST)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .shadow(true)
            .always_on_top(true)
            .skip_taskbar(true)
            // On whichever desktop the operator is on when they click, rather
            // than dragging them back to the one it was first drawn on.
            .visible_on_all_workspaces(true)
            // The first click on a panel that has just appeared is a click on
            // what it shows, not a click spent bringing it forward.
            .accept_first_mouse(true)
            .visible(false)
            .focused(false)
            .build()?;

        let tray = Arc::clone(self);
        panel.on_window_event(move |event| {
            if let WindowEvent::Focused(false) = event {
                tray.blurred();
            }
        });
        Ok(())
    }

    /// The icon, clicked: the panel opens, or closes if it was open.
    fn toggle(&self, rect: tauri::Rect) {
        let Some(panel) = self.app.get_webview_window(PANEL) else {
            // No panel until the window has said which workspace it is
            // showing, and until then the window is where there is anything
            // to see.
            self.open_window(None);
            return;
        };
        if panel.is_visible().unwrap_or(false) {
            self.put_away(&panel, true);
            return;
        }
        if self.spot.lock().blurred.take().is_some_and(|at| at.elapsed() < REOPEN) {
            return;
        }

        self.spot.lock().icon = Some(pixels(rect));
        self.stand(&panel);
        // Brought back first if closing the panel hid the app. Hidden is a
        // state of the app rather than of a window, and a window shown inside
        // a hidden app stays out of sight.
        #[cfg(target_os = "macos")]
        if let Err(err) = self.app.show() {
            tracing::debug!(%err, "could not unhide Guaca");
        }
        for (what, result) in [("show", panel.show()), ("focus", panel.set_focus())] {
            if let Err(err) = result {
                tracing::debug!(%err, "could not {what} the menu bar panel");
            }
        }
    }

    /// The panel lost the focus, which means the operator is somewhere else.
    fn blurred(&self) {
        let Some(panel) = self.app.get_webview_window(PANEL) else { return };
        if !panel.is_visible().unwrap_or(false) {
            return;
        }
        self.spot.lock().blurred = Some(Instant::now());
        self.put_away(&panel, false);
    }

    /// Hides the panel.
    ///
    /// `hand_back` is for a close from the panel or the icon, where the
    /// operator was in the panel and nowhere else. Opening it brought Guaca
    /// forward, and with the window not open, putting the panel away would
    /// leave the operator's keyboard in an app showing nothing. Hiding the app
    /// hands it back to whatever had it. A panel closed by clicking somewhere
    /// else has already lost the keyboard to that somewhere, and needs nothing.
    fn put_away(&self, panel: &WebviewWindow, hand_back: bool) {
        if let Err(err) = panel.hide() {
            tracing::debug!(%err, "could not hide the menu bar panel");
        }
        #[cfg(target_os = "macos")]
        if hand_back
            && !main_window(&self.app).is_some_and(|window| window.is_visible().unwrap_or(false))
        {
            if let Err(err) = self.app.hide() {
                tracing::debug!(%err, "could not hand the keyboard back after the panel");
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = hand_back;
    }

    /// Closes the panel from inside it.
    pub fn close(&self) {
        if let Some(panel) = self.app.get_webview_window(PANEL) {
            self.put_away(&panel, true);
        }
    }

    /// The page's height, as the page measured it. The window follows it
    /// while it is open and takes it the next time it opens.
    pub fn fit(&self, height: f64) {
        if !height.is_finite() || height <= 0.0 {
            return;
        }
        tracing::debug!(height, "the menu bar panel measured itself");
        self.spot.lock().wanted = Some(height);
        if let Some(panel) = self.app.get_webview_window(PANEL) {
            if panel.is_visible().unwrap_or(false) {
                self.stand(&panel);
            }
        }
    }

    /// Puts the panel under the icon, as tall as the page asked to be.
    fn stand(&self, panel: &WebviewWindow) {
        let (icon, wanted) = {
            let spot = self.spot.lock();
            (spot.icon, spot.wanted.unwrap_or(menubar::PANEL_FIRST))
        };
        let Some(icon) = icon else { return };
        let Some(at) = menubar::place(icon, &screens(&self.app), wanted) else {
            tracing::debug!("no display claims the menu bar icon; the panel stays where it was");
            return;
        };
        for (what, result) in [
            ("size", panel.set_size(LogicalSize::new(at.width, at.height))),
            ("place", panel.set_position(LogicalPosition::new(at.x, at.y))),
        ] {
            if let Err(err) = result {
                tracing::debug!(%err, "could not {what} the menu bar panel");
            }
        }
    }

    /// Brings the window back, optionally somewhere in particular, and puts
    /// the panel away: the operator asked for the bigger of the two.
    ///
    /// Shown *and* unminimized *and* focused, because the window can be in any
    /// of the three states and only one of the three calls fixes each.
    pub fn open_window(&self, target: Option<Reveal>) {
        if let Some(panel) = self.app.get_webview_window(PANEL) {
            self.put_away(&panel, false);
        }
        let Some(window) = main_window(&self.app) else {
            tracing::warn!("no window to open from the menu bar");
            return;
        };
        #[cfg(target_os = "macos")]
        if let Err(err) = self.app.show() {
            tracing::debug!(%err, "could not unhide Guaca");
        }
        for (what, result) in [
            ("show", window.show()),
            ("unminimize", window.unminimize()),
            ("focus", window.set_focus()),
        ] {
            if let Err(err) = result {
                tracing::debug!(%err, "could not {what} the window");
            }
        }
        if let Some(target) = target {
            // Emitted after the window is up, so the transcript it scrolls is
            // one that is being drawn, and to the window alone: the panel has
            // nowhere to go.
            if let Err(err) = self.app.emit_to(window.label(), REVEAL, target) {
                tracing::debug!(%err, "could not ask the window to go anywhere");
            }
        }
    }
}

/// The window, whatever it is called, and never the panel.
fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.webview_windows().into_iter().find(|(label, _)| label != PANEL).map(|(_, window)| window)
}

/// The icon as the tray reported it, in pixels.
///
/// Every tray the platform layer supports reports pixels. A logical rectangle
/// read at a scale of one is what it already is, so either arrives as the same
/// thing `menubar::place` reads.
fn pixels(rect: tauri::Rect) -> Frame {
    let at = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    Frame { x: at.x, y: at.y, width: size.width, height: size.height }
}

/// Every display, in points, as `menubar::place` reads them.
///
/// Each display reports its pixels at its own scale, so each is read back to
/// points at that scale, which is the one coordinate space two displays at
/// different scales share.
fn screens(app: &AppHandle) -> Vec<Screen> {
    let monitors = match app.available_monitors() {
        Ok(monitors) => monitors,
        Err(err) => {
            tracing::debug!(%err, "could not list the displays");
            return Vec::new();
        }
    };
    monitors
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let at = monitor.position().to_logical::<f64>(scale);
            let size = monitor.size().to_logical::<f64>(scale);
            let area = monitor.work_area();
            let area_at = area.position.to_logical::<f64>(scale);
            let area_size = area.size.to_logical::<f64>(scale);
            Screen {
                bounds: Frame { x: at.x, y: at.y, width: size.width, height: size.height },
                area: Frame {
                    x: area_at.x,
                    y: area_at.y,
                    width: area_size.width,
                    height: area_size.height,
                },
                scale,
            }
        })
        .collect()
}

/// The image for one state, and whether the platform may tint it.
fn glyph(glyph: Glyph) -> (Image<'static>, bool) {
    match glyph {
        Glyph::Idle => (tauri::include_image!("./icons/tray-idle.png"), true),
        Glyph::Working => (tauri::include_image!("./icons/tray-working.png"), true),
        // The one that is not a template. See `menubar::Glyph`.
        Glyph::Attention => (tauri::include_image!("./icons/tray-attention.png"), false),
    }
}
