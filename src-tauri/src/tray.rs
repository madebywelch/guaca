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

pub const TRAY_ID: &str = "guac.menubar";

pub const REVEAL: &str = "guac://reveal";

const PANEL: &str = "menubar";
const PAGE: &str = "menubar.html";

const OPEN: &str = "guac.open";

const COALESCE: Duration = Duration::from_millis(300);

const REOPEN: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Reveal {
    ForYou,
    Agent { id: AgentId },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub origin: String,
    pub token: String,
}

#[derive(Serialize)]
pub struct PanelAssignment {
    #[serde(flatten)]
    host: Host,
    generation: u64,
}

#[derive(Default)]
struct Spot {
    icon: Option<Frame>,
    wanted: Option<f64>,
    blurred: Option<Instant>,
}

pub struct Tray {
    app: AppHandle,
    icon: TrayIcon<Wry>,
    drawn: Mutex<Option<Look>>,
    wake: Arc<Notify>,
    fed: Mutex<Option<Presence>>,
    host: Mutex<menubar::PanelHost<Host>>,
    spot: Mutex<Spot>,
}

impl Tray {
    pub fn install(app: &AppHandle) -> tauri::Result<Arc<Self>> {
        let look = Presence::default().look();
        let (image, template) = glyph(look.glyph);

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
            .icon_as_template(template)
            .title(look.title.clone().unwrap_or_default())
            .tooltip(&look.tooltip)
            .menu(&menu)
            .show_menu_on_left_click(false)
            .build(app)?;

        let tray = Arc::new(Self {
            app: app.clone(),
            icon,
            drawn: Mutex::new(Some(look)),
            wake: Arc::new(Notify::new()),
            fed: Mutex::new(None),
            host: Mutex::new(menubar::PanelHost::default()),
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
                    tokio::time::sleep(COALESCE).await;

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

    pub fn feed(&self, presence: Option<Presence>) {
        *self.fed.lock() = presence;
        self.wake.notify_one();
    }

    fn redraw(&self) {
        let look = match self.fed.lock().as_ref() {
            Some(presence) => presence.look(),
            None => Presence::default().look(),
        };
        let mut drawn = self.drawn.lock();
        if !menubar::update_icon(&mut drawn, look, |look| self.wear(look)) {
            self.wake.notify_one();
        }
    }

    fn wear(&self, look: &Look) -> bool {
        let (image, template) = glyph(look.glyph);
        [
            self.icon.set_icon_with_as_template(Some(image), template),
            self.icon.set_title(look.title.as_deref().or(Some(""))),
            self.icon.set_tooltip(Some(&look.tooltip)),
        ]
        .into_iter()
        .fold(true, |all, result| match result {
            Ok(()) => all,
            Err(err) => {
                tracing::debug!(%err, "could not change the menu bar icon");
                false
            }
        })
    }

    pub fn attach(self: &Arc<Self>, host: Host) {
        if !self.host.lock().request(host) {
            return;
        }
        match self.app.get_webview_window(PANEL) {
            Some(panel) => {
                self.put_away(&panel, false);
                match panel.eval("if (window.dispatchEvent(new Event('guaca:before-reload', { cancelable: true }))) window.location.reload()") {
                    Ok(()) => tracing::info!("the menu bar panel is reloading"),
                    Err(err) => {
                        tracing::warn!(%err, "the menu bar panel is still showing the last workspace")
                    }
                }
            }
            None => match self.build_panel() {
                Ok(()) => tracing::info!("the menu bar panel was created"),
                Err(err) => {
                    tracing::warn!(%err, "no menu bar panel this session; the icon opens the window")
                }
            },
        }
    }

    pub fn host(&self) -> Option<PanelAssignment> {
        self.host.lock().desired().map(|(host, generation)| PanelAssignment { host, generation })
    }

    pub fn adopted(&self, generation: u64) -> bool {
        self.host.lock().acknowledge(generation)
    }

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
            .visible_on_all_workspaces(true)
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

    fn toggle(&self, rect: tauri::Rect) {
        if !self.host.lock().ready() {
            self.open_window(None);
            return;
        }
        let Some(panel) = self.app.get_webview_window(PANEL) else {
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
        #[cfg(target_os = "macos")]
        if let Err(err) = self.app.show() {
            tracing::debug!(%err, "could not unhide Guaca");
        }
        #[cfg(target_os = "macos")]
        if let Some(main) = objc2::MainThreadMarker::new() {
            #[allow(deprecated)]
            objc2_app_kit::NSApplication::sharedApplication(main).activateIgnoringOtherApps(true);
        }
        for (what, result) in [("show", panel.show()), ("focus", panel.set_focus())] {
            if let Err(err) = result {
                tracing::debug!(%err, "could not {what} the menu bar panel");
            }
        }
    }

    fn blurred(&self) {
        let Some(panel) = self.app.get_webview_window(PANEL) else { return };
        if !panel.is_visible().unwrap_or(false) {
            return;
        }
        self.spot.lock().blurred = Some(Instant::now());
        self.put_away(&panel, false);
    }

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

    pub fn close(&self) {
        if let Some(panel) = self.app.get_webview_window(PANEL) {
            self.put_away(&panel, true);
        }
    }

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

    pub fn open_window(&self, target: Option<Reveal>) {
        #[cfg(feature = "probe")]
        let trace_id = match &target {
            Some(Reveal::Agent { id }) => crate::probe::trace::opaque(&id.to_string()),
            _ => 0,
        };
        #[cfg(feature = "probe")]
        crate::probe::trace::event(crate::probe::trace::Phase::WindowOpen, trace_id, 0);
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
            if let Err(err) = self.app.emit_to(window.label(), REVEAL, target) {
                tracing::debug!(%err, "could not ask the window to go anywhere");
            } else {
                #[cfg(feature = "probe")]
                crate::probe::trace::event(crate::probe::trace::Phase::WindowReveal, trace_id, 0);
            }
        }
    }
}

#[cfg(feature = "probe")]
impl Tray {
    pub fn probe_view(&self) -> crate::probe::Menubar {
        let look = self.drawn.lock().clone().unwrap_or_else(|| Presence::default().look());
        let panel = self.app.get_webview_window(PANEL);
        crate::probe::Menubar {
            glyph: format!("{:?}", look.glyph).to_lowercase(),
            title: look.title,
            tooltip: look.tooltip,
            attached: self.host.lock().ready(),
            panel_visible: panel.as_ref().is_some_and(|panel| panel.is_visible().unwrap_or(false)),
            panel: panel.is_some(),
        }
    }

    pub fn probe_click(&self) -> Result<bool, String> {
        let rect = self
            .icon
            .rect()
            .map_err(|err| err.to_string())?
            .ok_or("the menu bar icon is not on screen")?;
        let closing = self.host.lock().ready()
            && self
                .app
                .get_webview_window(PANEL)
                .is_some_and(|panel| panel.is_visible().unwrap_or(false))
            && !main_window(&self.app).is_some_and(|window| window.is_visible().unwrap_or(false));
        crate::probe::trace::event(crate::probe::trace::Phase::ToggleEnter, 0, u64::from(closing));
        self.toggle(rect);
        crate::probe::trace::event(crate::probe::trace::Phase::ToggleReturn, 0, u64::from(closing));
        Ok(closing)
    }
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.webview_windows().into_iter().find(|(label, _)| label != PANEL).map(|(_, window)| window)
}

fn pixels(rect: tauri::Rect) -> Frame {
    let at = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    Frame { x: at.x, y: at.y, width: size.width, height: size.height }
}

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

fn glyph(glyph: Glyph) -> (Image<'static>, bool) {
    match glyph {
        Glyph::Idle => (tauri::include_image!("./icons/tray-idle.png"), true),
        Glyph::Working => (tauri::include_image!("./icons/tray-working.png"), true),
        Glyph::Attention => (tauri::include_image!("./icons/tray-attention.png"), false),
    }
}
