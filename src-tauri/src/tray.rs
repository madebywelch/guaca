//! The menu bar presence.
//!
//! The second file that knows Tauri exists, and it earns that the same way
//! `app.rs` does: everything above it is a plain library. What the strip *says*
//! is decided in `menubar.rs`, which has no idea a platform menu exists and is
//! tested without one. This file draws that decision, and turns a click back
//! into something the runtime already knows how to do.
//!
//! Two things here are less obvious than they look.
//!
//! **The presence is read, not accumulated.** Every number on the strip but the
//! session total is a fresh read of whatever already holds the truth: the
//! roster, the activity map, the pending requests, the usage table. A presence
//! assembled by adding up events drifts the moment one is missed, and the thing
//! that would drift is the number the operator is using to decide whether to go
//! and look. The reads are local and happen at most every
//! [`COALESCE`], and only while something is happening.
//!
//! **A menu is edited in place when it can be.** Replacing the menu closes one
//! the operator is reading, and the spend on it moves every few seconds while a
//! crew works, so a strip that rebuilt on every change would be unreadable
//! exactly when it was worth reading. `menubar::plan` decides which of the two
//! a change is.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tokio::sync::Notify;

use crate::domain::approval::Decision;
use crate::domain::ids::{AgentId, GroupId};
use crate::domain::usage::Tokens;
use crate::menubar::{self, Command, Crew, Glyph, Look, Member, Presence, Row, Update};
use crate::runtime::events::UiEvent;
use crate::runtime::Runtime;

/// The strip's own id, so `app.rs` can ask whether it is there.
///
/// That question is load-bearing: closing the window hides it instead of
/// quitting, and an app with no window and no menu bar icon is an app the
/// operator cannot see, cannot reach and cannot stop. Hiding is conditional on
/// this existing.
pub const TRAY_ID: &str = "guac.menubar";

/// The channel the strip asks the window to go somewhere on.
///
/// Its own channel rather than a variant of `UiEvent`. That one is the runtime
/// telling the UI what happened; this is one surface asking another to open a
/// channel, and folding the two together would put a case in the transcript's
/// event handling for something the runtime never emits.
pub const REVEAL: &str = "guac://reveal";

/// The channel a click goes down when the strip is showing a box.
///
/// The row was drawn from a presence the window handed over, so the act it
/// stands for belongs to the box too, and the window is what holds a
/// connection to the box. Local rows never come through here.
pub const MENUBAR: &str = "guac://menubar";

/// What a click on a fed strip asks the window to do.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Ask {
    StopAll,
    Decide { approval: crate::domain::ids::ApprovalId, decision: crate::domain::approval::Decision },
}

/// Where the strip is asking the window to go.
///
/// One channel and two destinations, because it is one gesture: the operator
/// clicked a row and expects to be looking at what it named. Two, because the
/// window answers them with different calls and neither is the other's fallback.
/// `select` follows an agent into whatever crew it is in; `focusGroup` opens a
/// crew and picks nobody in it, because choosing somebody would put an agent's
/// history on screen as a side effect of a click that was about the crew.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Reveal {
    Agent { id: AgentId },
    Crew { id: GroupId },
}

/// How long a burst of events becomes one redraw.
///
/// A cascade emits an activity change and a token count per agent per call, and
/// each of those is a real change to what the strip says. Coalescing is what
/// keeps that from being a menu rebuilt on the main thread ten times a second.
const COALESCE: std::time::Duration = std::time::Duration::from_millis(300);

/// The most requests read at once. Well past what the menu shows, so the count
/// of what did not fit is a real count.
const APPROVAL_WINDOW: u32 = 50;

/// A drawn menu, and the handles that can edit it without replacing it.
struct Painted {
    menu: Menu<Wry>,
    /// One entry per row, in the same order. `None` where the row has no text
    /// that can change, so an index into this is an index into the rows it was
    /// built from.
    items: Vec<Option<MenuItem<Wry>>>,
}

/// What is on screen right now.
struct Drawn {
    rows: Vec<Row>,
    look: Look,
    items: Vec<Option<MenuItem<Wry>>>,
}

pub struct Tray {
    app: AppHandle,
    runtime: Runtime,
    icon: TrayIcon<Wry>,
    /// Spent since the window opened.
    ///
    /// The one number with nowhere to be read from: "since this window opened"
    /// is not a question the usage table is asked anywhere else, and adding a
    /// timestamp filter to it would answer a different question after a
    /// restart.
    session: Mutex<Tokens>,
    drawn: Mutex<Drawn>,
    wake: Arc<Notify>,
    /// A presence the window handed over, while it is showing a workspace
    /// that is not this machine's. The strip follows the window: while this
    /// is set the local runtime is not read, and every click on a row that
    /// came from it is sent back to the window to act on.
    fed: Mutex<Option<Presence>>,
}

impl Tray {
    /// Puts the icon in the menu bar and starts keeping it current.
    ///
    /// Called before the agents are started, so nothing that happens on the way
    /// up is missed.
    pub fn install(app: &AppHandle, runtime: Runtime) -> tauri::Result<Arc<Self>> {
        let presence = read(&runtime, Tokens::default());
        let look = presence.look();
        let rows = presence.rows();

        let painted = build(app, &rows)?;
        let (image, template) = glyph(look.glyph);

        let icon = TrayIconBuilder::with_id(TRAY_ID)
            .icon(image)
            // macOS tints a template image to match the menu bar, in either
            // appearance and while the bar is highlighted. Giving that up is
            // the price of the one glyph that has a color, and `Look` is what
            // decides which of the two this is.
            .icon_as_template(template)
            .title(look.title.clone().unwrap_or_default())
            .tooltip(&look.tooltip)
            .menu(&painted.menu)
            // Left click opens the menu rather than the window. The whole point
            // of the strip is the glance that does not interrupt anything, and
            // the window is one item away.
            .show_menu_on_left_click(true)
            .build(app)?;

        let tray = Arc::new(Self {
            app: app.clone(),
            runtime,
            icon,
            session: Mutex::new(Tokens::default()),
            drawn: Mutex::new(Drawn { rows, look, items: painted.items }),
            wake: Arc::new(Notify::new()),
            fed: Mutex::new(None),
        });

        {
            let clicks = tray.clone();
            tray.icon.on_menu_event(move |_app, event| clicks.clicked(event.id().as_ref()));
        }

        {
            let tray = tray.clone();
            let wake = tray.wake.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    wake.notified().await;
                    // The burst, not the first event in it. Everything that
                    // arrives while this sleeps is already counted, and
                    // `Notify` holds one permit, so the next pass runs
                    // immediately and coalesces the next burst.
                    tokio::time::sleep(COALESCE).await;

                    // Off the async pool. Every menu call hops to the main
                    // thread and blocks on the answer, so a redraw that ran
                    // here would hold an executor thread for as long as the
                    // main thread was busy, and the threads it would hold are
                    // the ones the agents run on.
                    let redraw = tray.clone();
                    if let Err(err) =
                        tauri::async_runtime::spawn_blocking(move || redraw.redraw()).await
                    {
                        tracing::warn!(%err, "the menu bar stopped keeping itself current");
                        return;
                    }
                }
            });
        }

        Ok(tray)
    }

    /// Notices something the strip might have to say.
    ///
    /// Called from the event sink, which is on an agent's own task, so this does
    /// no I/O and takes no menu lock: it counts the call and rings the bell.
    pub fn observe(&self, event: &UiEvent) {
        if let UiEvent::TokensUsed { prompt, completion, cost, .. } = event {
            menubar::add_call(&mut self.session.lock(), *prompt, *completion, *cost);
        }
        if menubar::touches(event) {
            self.wake.notify_one();
        }
    }

    /// Takes a presence from the window, or gives the strip back to this
    /// machine's runtime. Redraws either way.
    pub fn feed(&self, presence: Option<Presence>) {
        *self.fed.lock() = presence;
        self.wake.notify_one();
    }

    /// Whether the strip is currently drawn from what the window handed over.
    fn is_fed(&self) -> bool {
        self.fed.lock().is_some()
    }

    /// Reads the world and makes the strip agree with it.
    fn redraw(&self) {
        let presence = match self.fed.lock().clone() {
            Some(fed) => fed,
            None => read(&self.runtime, *self.session.lock()),
        };
        let look = presence.look();
        let rows = presence.rows();

        let mut drawn = self.drawn.lock();

        // Committed only when it all landed, for the same reason the text edits
        // below are: recording a change that failed leaves the strip stale for
        // good, because the next pass compares against what it thinks it wrote.
        if look != drawn.look && self.wear(&look) {
            drawn.look = look;
        }

        match menubar::plan(&drawn.rows, &rows) {
            Update::Nothing => {}
            Update::Text(edits) => {
                let mut all = true;
                for (index, text) in edits {
                    // A row with a label has an item, so a miss here is this
                    // file and `menubar` having drifted apart rather than
                    // anything about the workspace.
                    match drawn.items.get(index).and_then(Option::as_ref) {
                        Some(item) => {
                            if let Err(err) = item.set_text(menubar::escape_mnemonic(&text)) {
                                tracing::debug!(%err, "could not update a menu bar row");
                                all = false;
                            }
                        }
                        None => {
                            tracing::warn!(index, "no menu item to put {text:?} in");
                            all = false;
                        }
                    }
                }
                // Only what was written is remembered as written. Recording an
                // edit that failed would leave the row stale for good: the next
                // pass would compare against text that is not on screen and
                // find nothing to do.
                if all {
                    drawn.rows = rows;
                }
            }
            Update::Rebuild => match build(&self.app, &rows) {
                Ok(painted) => {
                    if let Err(err) = self.icon.set_menu(Some(painted.menu)) {
                        tracing::debug!(%err, "could not replace the menu bar menu");
                        return;
                    }
                    drawn.rows = rows;
                    drawn.items = painted.items;
                }
                // The menu on screen is the last one that built, which is stale
                // rather than wrong, and every row on it still does what it
                // says. Keeping it beats an empty menu bar.
                Err(err) => tracing::warn!(%err, "could not build the menu bar menu"),
            },
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
                tracing::debug!(%err, "could not change the menu bar");
                false
            }
        })
    }

    /// One click, from a menu item id back to something the runtime does.
    ///
    /// An id this build does not recognize is ignored. That is not defensive
    /// padding: the disabled rows carry ids too, and a guess at an unparseable
    /// one is a permission request answered by nobody.
    fn clicked(&self, id: &str) {
        let Some(command) = Command::parse(id) else {
            tracing::debug!(id, "a menu bar row with nothing behind it was clicked");
            return;
        };

        match command {
            Command::Open => self.reveal(None),
            Command::Reveal(agent) => self.reveal(Some(Reveal::Agent { id: agent })),
            Command::Enter(crew) => self.reveal(Some(Reveal::Crew { id: crew })),
            // Both of these touch SQLite, and this runs on the main thread
            // inside the event loop. Off it: the operator's click should not be
            // the frame the window drops.
            // A row drawn from a box's presence belongs to the box, and the
            // window is what can reach it. Local rows act on the local runtime
            // as they always did.
            Command::StopAll if self.is_fed() => self.ask(Ask::StopAll),
            Command::Decide(approval, decision) if self.is_fed() => {
                self.ask(Ask::Decide { approval, decision })
            }
            Command::StopAll => {
                let runtime = self.runtime.clone();
                tauri::async_runtime::spawn(async move {
                    let stopped = runtime.stop_everything();
                    tracing::info!(stopped, "stopped every conversation from the menu bar");
                });
            }
            Command::Decide(approval, decision) => {
                let runtime = self.runtime.clone();
                let wake = self.wake.clone();
                tauri::async_runtime::spawn(async move {
                    match runtime.decide_approval(approval, decision) {
                        Ok(settled) => tracing::info!(
                            %approval,
                            state = settled.state.as_str(),
                            "answered a permission request from the menu bar"
                        ),
                        // Answered in the window, or lapsed while the menu was
                        // open. The row is stale rather than broken, and the
                        // redraw is what takes it away.
                        Err(err) => {
                            tracing::debug!(%err, %approval, "that request was already settled");
                            wake.notify_one();
                        }
                    }
                });
            }
        }
    }

    /// Sends a click on a fed row to the window, which holds the connection
    /// the act has to go over.
    fn ask(&self, ask: Ask) {
        if let Err(err) = self.app.emit(MENUBAR, ask) {
            tracing::debug!(%err, "could not hand a menu bar click to the window");
        }
    }

    /// Brings the window back, optionally somewhere in particular.
    ///
    /// Shown *and* unminimized *and* focused, because the window can be in any
    /// of the three states and only one of the three calls fixes each.
    fn reveal(&self, target: Option<Reveal>) {
        let Some(window) = window(&self.app) else {
            tracing::warn!("no window to open from the menu bar");
            return;
        };
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
            // one that is being drawn.
            if let Err(err) = self.app.emit(REVEAL, target) {
                tracing::debug!(%err, "could not ask the window to go anywhere");
            }
        }
    }
}

/// The window, whatever it is called.
fn window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    app.get_webview_window("main").or_else(|| app.webview_windows().into_values().next())
}

/// Everything the strip shows, read fresh.
///
/// A failed read is treated as nothing rather than propagated. The strip is the
/// redundant copy of state the window already holds; a menu bar that says "no
/// requests" because the disk hiccuped is wrong for 300 milliseconds, and one
/// that took a turn down with it is wrong permanently.
fn read(runtime: &Runtime, session: Tokens) -> Presence {
    let store = runtime.store();

    let roster = match store.list_agents() {
        Ok(agents) => agents
            .into_iter()
            .map(|card| (card.id, Member { name: card.name, crew: card.group_id }))
            .collect(),
        Err(err) => {
            tracing::debug!(%err, "could not read the roster for the menu bar");
            Default::default()
        }
    };

    // A read that failed is no crews, which is a menu that names none: the same
    // strip this was before crews were on it, rather than one that guesses at
    // where an agent is.
    let crews = match store.list_groups() {
        Ok(groups) => {
            groups.into_iter().map(|group| Crew { id: group.id, name: group.name }).collect()
        }
        Err(err) => {
            tracing::debug!(%err, "could not read the crews for the menu bar");
            Vec::new()
        }
    };

    let waiting = match store.pending_approvals(APPROVAL_WINDOW) {
        Ok(waiting) => waiting,
        Err(err) => {
            tracing::debug!(%err, "could not read pending requests for the menu bar");
            Vec::new()
        }
    };

    let stuck = match store.open_escalations(APPROVAL_WINDOW) {
        Ok(stuck) => stuck,
        Err(err) => {
            tracing::debug!(%err, "could not read open escalations for the menu bar");
            Vec::new()
        }
    };

    let all_time = match store.usage_by_group() {
        Ok(groups) => menubar::sum(groups.into_values()),
        Err(err) => {
            tracing::debug!(%err, "could not read spend for the menu bar");
            Tokens::default()
        }
    };

    Presence {
        roster,
        crews,
        activity: runtime.activity_snapshot(),
        waiting,
        stuck,
        session,
        all_time,
        running: runtime.live_runs(),
    }
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

/// Draws a list of rows as a platform menu.
///
/// Returns the item handles alongside it, in row order, so the next change can
/// edit a row rather than replace the menu it is in.
fn build(app: &AppHandle, rows: &[Row]) -> tauri::Result<Painted> {
    let menu = Menu::new(app)?;
    let mut items = Vec::with_capacity(rows.len());

    for (index, row) in rows.iter().enumerate() {
        match row {
            Row::Note(text) => {
                let item = note(app, &index.to_string(), text)?;
                menu.append(&item)?;
                items.push(Some(item));
            }
            Row::Separator => {
                menu.append(&PredefinedMenuItem::separator(app)?)?;
                items.push(None);
            }
            Row::Waiting { id, agent, label, detail, always } => {
                let sub = Submenu::new(app, menubar::escape_mnemonic(label), true)?;
                // The request's own fields first, because a decision made
                // without them is a decision made blind, and they are inert:
                // the values are what a model asked for.
                for (field, line) in detail.iter().enumerate() {
                    sub.append(&note(app, &format!("{index}.{field}"), line)?)?;
                }
                if !detail.is_empty() {
                    sub.append(&PredefinedMenuItem::separator(app)?)?;
                }
                sub.append(&answer(app, Command::Decide(*id, Decision::Allow), "Allow")?)?;
                if *always {
                    sub.append(&answer(
                        app,
                        Command::Decide(*id, Decision::AlwaysAllow),
                        "Always allow",
                    )?)?;
                }
                sub.append(&answer(app, Command::Decide(*id, Decision::Deny), "Deny")?)?;
                sub.append(&PredefinedMenuItem::separator(app)?)?;
                sub.append(&answer(app, Command::Reveal(*agent), "Open in Guaca")?)?;
                menu.append(&sub)?;
                // The label never moves: it is composed from a request that
                // does not change. See `Row::shape`.
                items.push(None);
            }
            Row::Agent { id, label } => {
                let item = answer(app, Command::Reveal(*id), label)?;
                menu.append(&item)?;
                items.push(Some(item));
            }
            Row::Crew { id, label } => {
                let item = answer(app, Command::Enter(*id), label)?;
                menu.append(&item)?;
                items.push(Some(item));
            }
            Row::Open => {
                let item = answer(app, Command::Open, "Open Guaca")?;
                menu.append(&item)?;
                items.push(None);
            }
            Row::StopAll(label) => {
                let item = answer(app, Command::StopAll, label)?;
                menu.append(&item)?;
                items.push(Some(item));
            }
            Row::Quit => {
                // The platform's own quit, so it behaves like every other app's
                // and picks up the accelerator the operator already knows.
                menu.append(&PredefinedMenuItem::quit(app, Some("Quit Guaca"))?)?;
                items.push(None);
            }
        }
    }

    Ok(Painted { menu, items })
}

/// A row that says something and does nothing.
///
/// It still carries an id, because two menu items with the same id is not
/// something to find out about on a platform that minds. `Command::parse`
/// refuses these, which is what keeps a heading from being clickable if a
/// platform ever decides a disabled item can be.
fn note(app: &AppHandle, key: &str, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(
        app,
        format!("guac.note.{key}"),
        menubar::escape_mnemonic(text),
        false,
        None::<&str>,
    )
}

/// A row that does something.
fn answer(app: &AppHandle, command: Command, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, command.id(), menubar::escape_mnemonic(text), true, None::<&str>)
}
