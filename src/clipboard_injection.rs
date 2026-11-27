//! Putting text on the real operating system clipboard from a script, and giving the user's own
//! text back.
//!
//! The first `clipboard_set` of a run saves the user's text in [`SavedUserClipboard`].
//! `clipboard_restore` and exiting the app put it back. Only text can be saved: an image or any
//! other content on the clipboard is replaced for good.

use std::fmt;

use bevy::{
    app::AppExit,
    clipboard::{Clipboard, ClipboardError},
    prelude::*,
};

use crate::status::record_error_in_world;

/// One change to the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardAction {
    /// Place this text, saving the user's text first when nothing is saved yet.
    Set(String),
    /// Put the saved text back and forget it.
    Restore,
}

impl ClipboardAction {
    fn command_name(&self) -> &'static str {
        match self {
            Self::Set(_) => "clipboard_set",
            Self::Restore => "clipboard_restore",
        }
    }
}

/// What the clipboard held before the driver first wrote to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardSnapshot {
    /// The user's text.
    Text(String),
    /// The clipboard was empty or held something other than text.
    NoText,
}

/// What reading the clipboard as text produced.
#[derive(Debug, Clone)]
pub enum ClipboardReadOutcome {
    /// The clipboard's text.
    Text(String),
    /// The clipboard is empty, holds non-text content, or the read was not immediately ready.
    NoText,
    /// The read failed, so what the clipboard holds is unknown.
    Failed(ClipboardError),
}

/// Sorts the raw result of `poll_result` into an outcome. Only errors that mean "no text here"
/// count as [`ClipboardReadOutcome::NoText`]; every other error is a failed read.
#[must_use]
pub fn classify_read(result: Option<Result<String, ClipboardError>>) -> ClipboardReadOutcome {
    match result {
        Some(Ok(text)) => ClipboardReadOutcome::Text(text),
        None
        | Some(Err(ClipboardError::ContentNotAvailable | ClipboardError::ConversionFailure)) => {
            ClipboardReadOutcome::NoText
        }
        Some(Err(error)) => ClipboardReadOutcome::Failed(error),
    }
}

/// What a restore has to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestorePlan {
    /// Write this text to the clipboard.
    WriteText(String),
    /// The clipboard held no text, so there is nothing to write back.
    NoTextWasSaved,
    /// No `clipboard_set` ran, so nothing was saved.
    NothingSaved,
    /// The clipboard no longer holds the driver's last text, so the user copied something newer
    /// and the saved text is dropped instead of written over it.
    UserChangedClipboard,
}

/// Decides what restoring a saved snapshot means. `last_written` is the last text the driver
/// wrote and `current` is what the clipboard holds now, or none when it could not be read.
#[must_use]
pub fn restore_plan(
    snapshot: Option<ClipboardSnapshot>,
    last_written: Option<&str>,
    current: Option<&str>,
) -> RestorePlan {
    match snapshot {
        Some(ClipboardSnapshot::Text(text)) if current.is_some() && current == last_written => {
            RestorePlan::WriteText(text)
        }
        Some(ClipboardSnapshot::Text(_)) => RestorePlan::UserChangedClipboard,
        Some(ClipboardSnapshot::NoText) => RestorePlan::NoTextWasSaved,
        None => RestorePlan::NothingSaved,
    }
}

/// The user's clipboard text, saved before the driver first overwrote it.
#[derive(Resource, Debug, Default)]
pub struct SavedUserClipboard {
    snapshot: Option<ClipboardSnapshot>,
    last_written: Option<String>,
}

impl SavedUserClipboard {
    /// Reads and saves the clipboard unless a snapshot is already saved, so later writes never
    /// replace the user's original. Returns the new snapshot, or none when one already existed.
    ///
    /// # Errors
    ///
    /// Returns the read error, and saves nothing, when the read failed.
    pub fn remember_first(
        &mut self,
        read: impl FnOnce() -> ClipboardReadOutcome,
    ) -> Result<Option<&ClipboardSnapshot>, ClipboardError> {
        if self.snapshot.is_some() {
            return Ok(None);
        }
        let snapshot = match read() {
            ClipboardReadOutcome::Text(text) => ClipboardSnapshot::Text(text),
            ClipboardReadOutcome::NoText => ClipboardSnapshot::NoText,
            ClipboardReadOutcome::Failed(error) => return Err(error),
        };
        Ok(Some(self.snapshot.insert(snapshot)))
    }

    /// Remembers the text the driver just wrote.
    pub fn note_written(&mut self, text: String) {
        self.last_written = Some(text);
    }

    /// Removes and returns the saved snapshot and the driver's last written text.
    pub fn take(&mut self) -> (Option<ClipboardSnapshot>, Option<String>) {
        (self.snapshot.take(), self.last_written.take())
    }
}

/// Why a clipboard command failed.
#[derive(Debug, Clone)]
pub enum ClipboardCommandError {
    /// The game has no `Clipboard` resource.
    NoClipboard {
        /// What the command could not do, as it completes "the clipboard cannot be ...".
        verb: &'static str,
    },
    /// The clipboard refused the text.
    Write(ClipboardError),
    /// The user's clipboard could not be read, so it was not changed.
    Read(ClipboardError),
}

impl fmt::Display for ClipboardCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoClipboard { verb } => write!(
                formatter,
                "the game has no ClipboardPlugin, so the clipboard cannot be {verb}"
            ),
            Self::Write(source) => write!(formatter, "{source}"),
            Self::Read(source) => write!(
                formatter,
                "could not read the clipboard to save it first, so it was not changed: {source}"
            ),
        }
    }
}

impl std::error::Error for ClipboardCommandError {}

fn read_clipboard(clipboard: &mut Clipboard) -> ClipboardReadOutcome {
    classify_read(clipboard.fetch_text().poll_result())
}

fn set_clipboard_text(world: &mut World, text: String) -> Result<(), ClipboardCommandError> {
    if !world.contains_resource::<Clipboard>() {
        return Err(ClipboardCommandError::NoClipboard { verb: "set" });
    }
    world.init_resource::<SavedUserClipboard>();
    world.resource_scope(|world, mut clipboard: Mut<Clipboard>| {
        let mut saved = world.resource_mut::<SavedUserClipboard>();
        let first_snapshot = saved
            .remember_first(|| read_clipboard(&mut clipboard))
            .map_err(ClipboardCommandError::Read)?;
        if matches!(first_snapshot, Some(ClipboardSnapshot::NoText)) {
            warn!(
                "input driver: the clipboard held no text, so its content cannot be restored \
                 after clipboard_set"
            );
        }
        clipboard
            .set_text(text.as_str())
            .map_err(ClipboardCommandError::Write)?;
        saved.note_written(text);
        Ok(())
    })
}

fn restore_saved_clipboard(world: &mut World) -> Result<RestorePlan, ClipboardCommandError> {
    world.init_resource::<SavedUserClipboard>();
    let (snapshot, last_written) = world.resource_mut::<SavedUserClipboard>().take();
    if !matches!(snapshot, Some(ClipboardSnapshot::Text(_))) {
        return Ok(restore_plan(snapshot, None, None));
    }
    let mut clipboard = world
        .get_resource_mut::<Clipboard>()
        .ok_or(ClipboardCommandError::NoClipboard { verb: "restored" })?;
    let current = match read_clipboard(&mut clipboard) {
        ClipboardReadOutcome::Text(text) => Some(text),
        ClipboardReadOutcome::NoText | ClipboardReadOutcome::Failed(_) => None,
    };
    let plan = restore_plan(snapshot, last_written.as_deref(), current.as_deref());
    if let RestorePlan::WriteText(text) = &plan {
        clipboard
            .set_text(text.as_str())
            .map_err(ClipboardCommandError::Write)?;
    }
    Ok(plan)
}

fn log_restore(plan: &RestorePlan, context: &str) {
    match plan {
        RestorePlan::WriteText(_) => {
            info!("input driver: {context}: put the user's clipboard text back");
        }
        RestorePlan::NoTextWasSaved => info!(
            "input driver: {context}: the clipboard held no text before, and non-text content \
             cannot be restored, so it was left as it is"
        ),
        RestorePlan::NothingSaved => {
            info!("input driver: {context}: nothing was saved, so the clipboard was left alone");
        }
        RestorePlan::UserChangedClipboard => info!(
            "input driver: {context}: the user changed the clipboard since the driver's last \
             write, so the saved text was dropped and the clipboard left as it is"
        ),
    }
}

fn apply_clipboard_action(world: &mut World, action: ClipboardAction) {
    let command = action.command_name();
    let outcome = match action {
        ClipboardAction::Set(text) => set_clipboard_text(world, text),
        ClipboardAction::Restore => restore_saved_clipboard(world).map(|plan| {
            log_restore(&plan, command);
        }),
    };
    if let Err(problem) = outcome {
        record_error_in_world(world, format!("{command}: {problem}"));
    }
}

/// Queues a world command that runs one clipboard change.
pub fn queue_clipboard_action(commands: &mut Commands, action: ClipboardAction) {
    commands.queue(move |world: &mut World| apply_clipboard_action(world, action));
}

fn restore_on_exit(world: &mut World) {
    match restore_saved_clipboard(world) {
        Ok(RestorePlan::NothingSaved) => {}
        Ok(plan) => log_restore(&plan, "restoring the clipboard on exit"),
        Err(problem) => warn!("input driver: restoring the clipboard on exit failed: {problem}"),
    }
}

// `quit` writes `AppExit` from a command queued in `PreUpdate`, and no driver system writes it in
// `Last`, so this system sees every driver exit without an ordering constraint.
fn restore_clipboard_when_exiting(mut exits: MessageReader<AppExit>, mut commands: Commands) {
    if exits.read().next().is_some() {
        commands.queue(restore_on_exit);
    }
}

/// Keeps the saved user clipboard and restores it when the app exits.
pub struct DriverClipboardPlugin;

impl Plugin for DriverClipboardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SavedUserClipboard>()
            .add_systems(Last, restore_clipboard_when_exiting);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::DriverStatus;

    fn text(value: &str) -> ClipboardReadOutcome {
        ClipboardReadOutcome::Text(value.to_owned())
    }

    fn remembered(
        saved: &mut SavedUserClipboard,
        outcome: ClipboardReadOutcome,
    ) -> Option<ClipboardSnapshot> {
        saved
            .remember_first(|| outcome)
            .expect("the read should not fail")
            .cloned()
    }

    #[test]
    fn only_no_text_errors_count_as_no_text() {
        assert!(matches!(
            classify_read(Some(Ok("mine".to_owned()))),
            ClipboardReadOutcome::Text(value) if value == "mine"
        ));
        for result in [
            None,
            Some(Err(ClipboardError::ContentNotAvailable)),
            Some(Err(ClipboardError::ConversionFailure)),
        ] {
            assert!(matches!(
                classify_read(result),
                ClipboardReadOutcome::NoText
            ));
        }
        for error in [
            ClipboardError::ClipboardNotSupported,
            ClipboardError::ClipboardOccupied,
            ClipboardError::Unknown {
                description: "boom".to_owned(),
            },
        ] {
            assert!(matches!(
                classify_read(Some(Err(error))),
                ClipboardReadOutcome::Failed(_)
            ));
        }
    }

    #[test]
    fn only_the_first_set_saves_the_original() {
        let mut saved = SavedUserClipboard::default();
        assert_eq!(
            remembered(&mut saved, text("original")),
            Some(ClipboardSnapshot::Text("original".to_owned()))
        );
        let second = saved
            .remember_first(|| panic!("a saved original must not be read again"))
            .expect("nothing is read");
        assert_eq!(second, None);
        assert_eq!(
            saved.take().0,
            Some(ClipboardSnapshot::Text("original".to_owned()))
        );
    }

    #[test]
    fn a_failed_read_saves_nothing() {
        let mut saved = SavedUserClipboard::default();
        let outcome = saved
            .remember_first(|| ClipboardReadOutcome::Failed(ClipboardError::ClipboardOccupied));
        assert!(matches!(outcome, Err(ClipboardError::ClipboardOccupied)));
        assert_eq!(saved.take(), (None, None));
        assert_eq!(
            remembered(&mut saved, ClipboardReadOutcome::NoText),
            Some(ClipboardSnapshot::NoText)
        );
    }

    #[test]
    fn taking_forgets_the_snapshot_and_the_last_written_text() {
        let mut saved = SavedUserClipboard::default();
        remembered(&mut saved, text("one"));
        saved.note_written("driver".to_owned());
        assert_eq!(
            saved.take(),
            (
                Some(ClipboardSnapshot::Text("one".to_owned())),
                Some("driver".to_owned())
            )
        );
        assert_eq!(saved.take(), (None, None));
    }

    #[test]
    fn restore_writes_back_only_while_the_clipboard_still_holds_the_driver_text() {
        let saved = || Some(ClipboardSnapshot::Text("mine".to_owned()));
        assert_eq!(
            restore_plan(saved(), Some("driver"), Some("driver")),
            RestorePlan::WriteText("mine".to_owned())
        );
        assert_eq!(
            restore_plan(saved(), Some("driver"), Some("newer copy")),
            RestorePlan::UserChangedClipboard
        );
        assert_eq!(
            restore_plan(saved(), Some("driver"), None),
            RestorePlan::UserChangedClipboard
        );
        assert_eq!(
            restore_plan(saved(), None, None),
            RestorePlan::UserChangedClipboard
        );
    }

    #[test]
    fn restore_plans_without_saved_text_ignore_the_clipboard() {
        assert_eq!(
            restore_plan(Some(ClipboardSnapshot::NoText), None, None),
            RestorePlan::NoTextWasSaved
        );
        assert_eq!(restore_plan(None, None, None), RestorePlan::NothingSaved);
    }

    #[test]
    fn a_failed_read_error_names_the_command_and_the_cause() {
        let message = ClipboardCommandError::Read(ClipboardError::ClipboardOccupied).to_string();
        let prefix = "could not read the clipboard to save it first, so it was not changed: ";
        assert!(message.starts_with(prefix), "{message}");
    }

    #[test]
    fn setting_without_a_clipboard_records_the_error() {
        let mut world = World::new();
        world.init_resource::<DriverStatus>();
        apply_clipboard_action(&mut world, ClipboardAction::Set("hello".to_owned()));
        assert_eq!(
            world.resource::<DriverStatus>().errors,
            vec![
                "clipboard_set: the game has no ClipboardPlugin, so the clipboard cannot be set"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn restoring_with_nothing_saved_is_not_an_error() {
        let mut world = World::new();
        world.init_resource::<DriverStatus>();
        apply_clipboard_action(&mut world, ClipboardAction::Restore);
        assert!(world.resource::<DriverStatus>().errors.is_empty());
    }
}
