use serde::{Deserialize, Serialize};

use super::ids::ArtifactId;

pub const MAX_WIDGETS: usize = 6;

pub const DEFAULT_EVERY: u32 = 5;
pub const MIN_EVERY: u32 = 1;
pub const MAX_EVERY: u32 = 24 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Width {
    #[default]
    Narrow,
    Wide,
}

impl Width {
    pub fn parse(raw: Option<&str>) -> Result<Width, WidgetError> {
        match raw.map(|raw| raw.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("narrow") => Ok(Width::Narrow),
            Some("wide") => Ok(Width::Wide),
            Some(other) => Err(WidgetError::BadWidth(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Width::Narrow => "narrow",
            Width::Wide => "wide",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Widget {
    pub artifact_id: ArtifactId,
    pub width: Width,
    pub every_minutes: u32,
    pub added_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WidgetError {
    #[error(
        "the status bar already holds {MAX_WIDGETS} widgets. The operator can unpin one with \
         Edit on the bar"
    )]
    Full,
    #[error("that page is already on the status bar")]
    AlreadyPinned,
    #[error("that page is not on the status bar")]
    NotPinned,
    #[error("`width` is narrow or wide, not {0:?}")]
    BadWidth(String),
    #[error(
        "`every_minutes` is how often the page's reads run, from {MIN_EVERY} to {MAX_EVERY}, not {0}"
    )]
    BadEvery(u32),
}

impl Widget {
    pub fn new(
        artifact_id: ArtifactId,
        width: Width,
        every_minutes: Option<u32>,
        added_by: &str,
    ) -> Result<Self, WidgetError> {
        let every_minutes = every_minutes.unwrap_or(DEFAULT_EVERY);
        if !(MIN_EVERY..=MAX_EVERY).contains(&every_minutes) {
            return Err(WidgetError::BadEvery(every_minutes));
        }
        Ok(Self { artifact_id, width, every_minutes, added_by: added_by.trim().to_string() })
    }
}

pub fn pin(bar: &mut Vec<Widget>, widget: Widget) -> Result<(), WidgetError> {
    if bar.iter().any(|pinned| pinned.artifact_id == widget.artifact_id) {
        return Err(WidgetError::AlreadyPinned);
    }
    if bar.len() >= MAX_WIDGETS {
        return Err(WidgetError::Full);
    }
    bar.push(widget);
    Ok(())
}

pub fn resize(bar: &mut [Widget], id: ArtifactId, width: Width) -> Result<(), WidgetError> {
    let pinned =
        bar.iter_mut().find(|pinned| pinned.artifact_id == id).ok_or(WidgetError::NotPinned)?;
    pinned.width = width;
    Ok(())
}

pub fn unpin(bar: &mut Vec<Widget>, id: ArtifactId) -> Result<Widget, WidgetError> {
    let at =
        bar.iter().position(|pinned| pinned.artifact_id == id).ok_or(WidgetError::NotPinned)?;
    Ok(bar.remove(at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widget() -> Widget {
        Widget::new(ArtifactId::new(), Width::Narrow, None, "Rae").unwrap()
    }

    #[test]
    fn reads_run_between_once_a_minute_and_once_a_day() {
        let id = ArtifactId::new();
        assert_eq!(widget().every_minutes, DEFAULT_EVERY);
        assert_eq!(Widget::new(id, Width::Wide, Some(0), "Rae"), Err(WidgetError::BadEvery(0)));
        assert!(matches!(
            Widget::new(id, Width::Wide, Some(MAX_EVERY + 1), "Rae"),
            Err(WidgetError::BadEvery(_))
        ));
        assert_eq!(Widget::new(id, Width::Wide, Some(1), "Rae").unwrap().every_minutes, 1);
    }

    #[test]
    fn a_width_is_narrow_unless_asked_for_wide() {
        assert_eq!(Width::parse(None), Ok(Width::Narrow));
        assert_eq!(Width::parse(Some(" Wide ")), Ok(Width::Wide));
        assert_eq!(Width::parse(Some("tall")), Err(WidgetError::BadWidth("tall".into())));
    }

    #[test]
    fn the_bar_holds_each_page_once_and_a_few_in_all() {
        let mut bar = Vec::new();
        let first = widget();
        pin(&mut bar, first.clone()).unwrap();
        assert_eq!(pin(&mut bar, first.clone()), Err(WidgetError::AlreadyPinned));
        while bar.len() < MAX_WIDGETS {
            pin(&mut bar, widget()).unwrap();
        }
        assert_eq!(pin(&mut bar, widget()), Err(WidgetError::Full));

        resize(&mut bar, first.artifact_id, Width::Wide).unwrap();
        assert_eq!(bar[0].width, Width::Wide, "resized where it stands");
        assert_eq!(unpin(&mut bar, first.artifact_id).unwrap().artifact_id, first.artifact_id);
        assert_eq!(unpin(&mut bar, first.artifact_id), Err(WidgetError::NotPinned));
        assert_eq!(resize(&mut bar, first.artifact_id, Width::Narrow), Err(WidgetError::NotPinned));
    }

    #[test]
    fn what_crosses_ipc_is_spelled_the_way_the_frontend_spells_it() {
        let wire = serde_json::to_value(widget()).unwrap();
        assert!(wire.get("artifactId").is_some(), "{wire}");
        assert_eq!(wire["width"], "narrow");
        assert_eq!(wire["everyMinutes"], DEFAULT_EVERY);
        assert_eq!(wire["addedBy"], "Rae");
    }
}
