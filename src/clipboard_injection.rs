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
    /// The clipboard is empty or holds non-text content.
    NoText,
    /// The read was not ready immediately, so what the clipboard holds is unknown.
    NotReady,
    /// The read failed, so what the clipboard holds is unknown.
    Failed(ClipboardError),
}

impl ClipboardReadOutcome {
    /// The reason the clipboard's content is unknown, or none when the read told what it holds.
    #[must_use]
    pub fn into_problem(self) -> Option<ClipboardReadProblem> {
        match self {
            Self::Text(_) | Self::NoText => None,
            Self::NotReady => Some(ClipboardReadProblem::NotReady),
            Self::Failed(error) => Some(ClipboardReadProblem::Failed(error)),
        }
    }
}

/// Why a clipboard read could not tell what the clipboard holds.
#[derive(Debug, Clone)]
pub enum ClipboardReadProblem {
    /// The read was not ready immediately.
    NotReady,
    /// The read failed.
    Failed(ClipboardError),
}

impl fmt::Display for ClipboardReadProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady => write!(formatter, "the clipboard was not ready to be read"),
            Self::Failed(source) => write!(formatter, "{source}"),
        }
    }
}

/// Sorts the raw result of `poll_result` into an outcome. Only errors that mean "no text here"
/// count as [`ClipboardReadOutcome::NoText`]; a read that is not ready or fails leaves the
/// clipboard's content unknown.
#[must_use]
pub fn classify_read(result: Option<Result<String, ClipboardError>>) -> ClipboardReadOutcome {
    match result {
        Some(Ok(text)) => ClipboardReadOutcome::Text(text),
        Some(Err(ClipboardError::ContentNotAvailable | ClipboardError::ConversionFailure)) => {
            ClipboardReadOutcome::NoText
        }
        None => ClipboardReadOutcome::NotReady,
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
    /// The clipboard could not be read, so the saved text is kept for a later restore.
    CouldNotCheck,
}

/// Decides what restoring a saved snapshot means. `last_written` is the last text the driver
/// wrote and `current` is the result of reading the clipboard now. Only a read that succeeded can
/// show that the user changed the clipboard.
#[must_use]
pub fn restore_plan(
    snapshot: Option<&ClipboardSnapshot>,
    last_written: Option<&str>,
    current: &ClipboardReadOutcome,
) -> RestorePlan {
    match (snapshot, current) {
        (None, _) => RestorePlan::NothingSaved,
        (Some(ClipboardSnapshot::NoText), _) => RestorePlan::NoTextWasSaved,
        (
            Some(ClipboardSnapshot::Text(_)),
            ClipboardReadOutcome::NotReady | ClipboardReadOutcome::Failed(_),
        ) => RestorePlan::CouldNotCheck,
        (Some(ClipboardSnapshot::Text(saved)), ClipboardReadOutcome::Text(text))
            if Some(text.as_str()) == last_written =>
        {
            RestorePlan::WriteText(saved.clone())
        }
        (Some(ClipboardSnapshot::Text(_)), _) => RestorePlan::UserChangedClipboard,
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
    /// Returns the problem, and saves nothing, when the read failed or was not ready.
    pub fn remember_first(
        &mut self,
        read: impl FnOnce() -> ClipboardReadOutcome,
    ) -> Result<Option<&ClipboardSnapshot>, ClipboardReadProblem> {
        if self.snapshot.is_some() {
            return Ok(None);
        }
        let snapshot = match read() {
            ClipboardReadOutcome::Text(text) => ClipboardSnapshot::Text(text),
            ClipboardReadOutcome::NoText => ClipboardSnapshot::NoText,
            ClipboardReadOutcome::NotReady => return Err(ClipboardReadProblem::NotReady),
            ClipboardReadOutcome::Failed(error) => return Err(ClipboardReadProblem::Failed(error)),
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

    /// Forgets the saved snapshot and the driver's last written text.
    pub fn forget(&mut self) {
        self.take();
    }

    /// The saved snapshot and the driver's last written text, left in place.
    #[must_use]
    pub fn peek(&self) -> (Option<&ClipboardSnapshot>, Option<&str>) {
        (self.snapshot.as_ref(), self.last_written.as_deref())
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
    Read(ClipboardReadProblem),
    /// The clipboard could not be read before restoring, so the saved text was kept.
    Check(ClipboardReadProblem),
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
            Self::Check(source) => write!(
                formatter,
                "could not read the clipboard to check it, so the saved text was kept for a later \
                 clipboard_restore: {source}"
            ),
        }
    }
}

impl std::error::Error for ClipboardCommandError {}

/// The clipboard operations the driver needs, so tests can swap in a clipboard that fails.
trait ClipboardAccess {
    fn read_text(&mut self) -> ClipboardReadOutcome;
    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError>;
}

impl ClipboardAccess for Clipboard {
    fn read_text(&mut self) -> ClipboardReadOutcome {
        classify_read(self.fetch_text().poll_result())
    }

    fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
        self.set_text(text)
    }
}

fn set_clipboard_text(world: &mut World, text: String) -> Result<(), ClipboardCommandError> {
    if !world.contains_resource::<Clipboard>() {
        return Err(ClipboardCommandError::NoClipboard { verb: "set" });
    }
    world.init_resource::<SavedUserClipboard>();
    world.resource_scope(|world, mut clipboard: Mut<Clipboard>| {
        let mut saved = world.resource_mut::<SavedUserClipboard>();
        save_original_and_write(&mut *clipboard, &mut saved, text)
    })
}

fn save_original_and_write(
    clipboard: &mut impl ClipboardAccess,
    saved: &mut SavedUserClipboard,
    text: String,
) -> Result<(), ClipboardCommandError> {
    let first_snapshot = saved
        .remember_first(|| clipboard.read_text())
        .map_err(ClipboardCommandError::Read)?;
    if matches!(first_snapshot, Some(ClipboardSnapshot::NoText)) {
        warn!(
            "input driver: the clipboard held no text, so its content cannot be restored \
             after clipboard_set"
        );
    }
    clipboard
        .write_text(&text)
        .map_err(ClipboardCommandError::Write)?;
    saved.note_written(text);
    Ok(())
}

fn restore_saved_clipboard(world: &mut World) -> Result<RestorePlan, ClipboardCommandError> {
    world.init_resource::<SavedUserClipboard>();
    world.resource_scope(|world, mut saved: Mut<SavedUserClipboard>| {
        let mut clipboard = world.get_resource_mut::<Clipboard>();
        restore_saved_text(clipboard.as_deref_mut(), &mut saved)
    })
}

// Only a saved text needs the clipboard, so a missing one is an error only then.
fn restore_saved_text(
    clipboard: Option<&mut impl ClipboardAccess>,
    saved: &mut SavedUserClipboard,
) -> Result<RestorePlan, ClipboardCommandError> {
    let (snapshot, last_written) = {
        let (snapshot, last_written) = saved.peek();
        (snapshot.cloned(), last_written.map(str::to_owned))
    };
    if !matches!(snapshot, Some(ClipboardSnapshot::Text(_))) {
        saved.forget();
        return Ok(restore_plan(
            snapshot.as_ref(),
            None,
            &ClipboardReadOutcome::NoText,
        ));
    }
    let Some(clipboard) = clipboard else {
        return Err(ClipboardCommandError::NoClipboard { verb: "restored" });
    };
    let plan = check_and_restore(clipboard, snapshot.as_ref(), last_written.as_deref())?;
    saved.forget();
    Ok(plan)
}

// The saved state stays until this succeeds, so a failed read or write can be retried.
fn check_and_restore(
    clipboard: &mut impl ClipboardAccess,
    snapshot: Option<&ClipboardSnapshot>,
    last_written: Option<&str>,
) -> Result<RestorePlan, ClipboardCommandError> {
    let outcome = clipboard.read_text();
    let plan = restore_plan(snapshot, last_written, &outcome);
    match (plan, outcome.into_problem()) {
        (RestorePlan::CouldNotCheck, Some(problem)) => Err(ClipboardCommandError::Check(problem)),
        (RestorePlan::WriteText(text), _) => {
            clipboard
                .write_text(&text)
                .map_err(ClipboardCommandError::Write)?;
            Ok(RestorePlan::WriteText(text))
        }
        (plan, _) => Ok(plan),
    }
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
        RestorePlan::CouldNotCheck => warn!(
            "input driver: {context}: the clipboard could not be checked, so the saved text was \
             kept"
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
        Err(problem) => warn!(
            "input driver: restoring the clipboard on exit failed, so the user's clipboard text \
             may be lost: {problem}"
        ),
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

    #[derive(Default)]
    struct FakeClipboard {
        current_text: Option<String>,
        next_read: Option<ClipboardReadOutcome>,
        next_write_failure: Option<ClipboardError>,
        read_count: usize,
        writes: Vec<String>,
    }

    impl FakeClipboard {
        fn holding(value: &str) -> Self {
            Self {
                current_text: Some(value.to_owned()),
                ..Self::default()
            }
        }
    }

    impl ClipboardAccess for FakeClipboard {
        fn read_text(&mut self) -> ClipboardReadOutcome {
            self.read_count += 1;
            if let Some(outcome) = self.next_read.take() {
                return outcome;
            }
            self.current_text
                .clone()
                .map_or(ClipboardReadOutcome::NoText, ClipboardReadOutcome::Text)
        }

        fn write_text(&mut self, text: &str) -> Result<(), ClipboardError> {
            if let Some(error) = self.next_write_failure.take() {
                return Err(error);
            }
            self.current_text = Some(text.to_owned());
            self.writes.push(text.to_owned());
            Ok(())
        }
    }

    fn saved_with_driver_text(original: &str, driver: &str) -> SavedUserClipboard {
        let mut saved = SavedUserClipboard::default();
        remembered(&mut saved, text(original));
        saved.note_written(driver.to_owned());
        saved
    }

    fn assert_still_saved(saved: &SavedUserClipboard) {
        assert_eq!(
            saved.peek(),
            (
                Some(&ClipboardSnapshot::Text("mine".to_owned())),
                Some("driver")
            )
        );
    }

    #[test]
    fn setting_fails_and_saves_nothing_when_the_first_read_fails() {
        let mut clipboard = FakeClipboard::holding("mine");
        clipboard.next_read = Some(ClipboardReadOutcome::Failed(
            ClipboardError::ClipboardOccupied,
        ));
        let mut saved = SavedUserClipboard::default();
        let result = save_original_and_write(&mut clipboard, &mut saved, "driver".to_owned());
        assert!(matches!(
            result,
            Err(ClipboardCommandError::Read(ClipboardReadProblem::Failed(
                ClipboardError::ClipboardOccupied
            )))
        ));
        assert_eq!(clipboard.current_text.as_deref(), Some("mine"));
        assert!(clipboard.writes.is_empty());
        assert_eq!(saved.peek(), (None, None));
    }

    #[test]
    fn setting_fails_and_saves_nothing_when_the_first_read_is_not_ready() {
        let mut clipboard = FakeClipboard::holding("mine");
        clipboard.next_read = Some(ClipboardReadOutcome::NotReady);
        let mut saved = SavedUserClipboard::default();
        let result = save_original_and_write(&mut clipboard, &mut saved, "driver".to_owned());
        assert!(matches!(
            result,
            Err(ClipboardCommandError::Read(ClipboardReadProblem::NotReady))
        ));
        assert_eq!(clipboard.current_text.as_deref(), Some("mine"));
        assert!(clipboard.writes.is_empty());
        assert_eq!(saved.peek(), (None, None));
    }

    #[test]
    fn a_failed_write_while_setting_keeps_the_snapshot_but_not_the_last_written_text() {
        let mut clipboard = FakeClipboard::holding("mine");
        clipboard.next_write_failure = Some(ClipboardError::ClipboardOccupied);
        let mut saved = SavedUserClipboard::default();
        let result = save_original_and_write(&mut clipboard, &mut saved, "driver".to_owned());
        assert!(matches!(
            result,
            Err(ClipboardCommandError::Write(
                ClipboardError::ClipboardOccupied
            ))
        ));
        assert_eq!(clipboard.current_text.as_deref(), Some("mine"));
        assert_eq!(
            saved.peek(),
            (Some(&ClipboardSnapshot::Text("mine".to_owned())), None)
        );
    }

    #[test]
    fn setting_saves_the_original_once_and_notes_the_driver_text() {
        let mut clipboard = FakeClipboard::holding("mine");
        let mut saved = SavedUserClipboard::default();
        save_original_and_write(&mut clipboard, &mut saved, "driver".to_owned())
            .expect("the first set should succeed");
        assert_eq!(clipboard.current_text.as_deref(), Some("driver"));
        assert_still_saved(&saved);

        save_original_and_write(&mut clipboard, &mut saved, "second".to_owned())
            .expect("the second set should succeed");
        assert_eq!(clipboard.read_count, 1);
        assert_eq!(clipboard.writes, ["driver", "second"]);
        assert_eq!(
            saved.peek(),
            (
                Some(&ClipboardSnapshot::Text("mine".to_owned())),
                Some("second")
            )
        );
    }

    #[test]
    fn restoring_writes_the_user_text_back_and_forgets_the_saved_state() {
        let mut clipboard = FakeClipboard::holding("driver");
        let mut saved = saved_with_driver_text("mine", "driver");
        let plan = restore_saved_text(Some(&mut clipboard), &mut saved)
            .expect("the restore should succeed");
        assert_eq!(plan, RestorePlan::WriteText("mine".to_owned()));
        assert_eq!(clipboard.current_text.as_deref(), Some("mine"));
        assert_eq!(saved.peek(), (None, None));
    }

    #[test]
    fn restoring_keeps_the_saved_state_when_the_check_is_not_ready() {
        let mut clipboard = FakeClipboard::holding("driver");
        clipboard.next_read = Some(ClipboardReadOutcome::NotReady);
        let mut saved = saved_with_driver_text("mine", "driver");
        let result = restore_saved_text(Some(&mut clipboard), &mut saved);
        assert!(matches!(
            result,
            Err(ClipboardCommandError::Check(ClipboardReadProblem::NotReady))
        ));
        assert!(clipboard.writes.is_empty());
        assert_eq!(clipboard.current_text.as_deref(), Some("driver"));
        assert_still_saved(&saved);
    }

    #[test]
    fn restoring_keeps_the_saved_state_when_the_check_fails() {
        let mut clipboard = FakeClipboard::holding("driver");
        clipboard.next_read = Some(ClipboardReadOutcome::Failed(
            ClipboardError::ClipboardOccupied,
        ));
        let mut saved = saved_with_driver_text("mine", "driver");
        let result = restore_saved_text(Some(&mut clipboard), &mut saved);
        assert!(matches!(
            result,
            Err(ClipboardCommandError::Check(ClipboardReadProblem::Failed(
                ClipboardError::ClipboardOccupied
            )))
        ));
        assert_still_saved(&saved);
    }

    #[test]
    fn a_failed_restore_write_keeps_the_saved_state_until_a_retry_succeeds() {
        let mut clipboard = FakeClipboard::holding("driver");
        clipboard.next_write_failure = Some(ClipboardError::ClipboardOccupied);
        let mut saved = saved_with_driver_text("mine", "driver");
        let result = restore_saved_text(Some(&mut clipboard), &mut saved);
        assert!(matches!(
            result,
            Err(ClipboardCommandError::Write(
                ClipboardError::ClipboardOccupied
            ))
        ));
        assert_still_saved(&saved);

        let plan =
            restore_saved_text(Some(&mut clipboard), &mut saved).expect("the retry should succeed");
        assert_eq!(plan, RestorePlan::WriteText("mine".to_owned()));
        assert_eq!(clipboard.current_text.as_deref(), Some("mine"));
        assert_eq!(saved.peek(), (None, None));
    }

    #[test]
    fn restoring_leaves_a_newer_user_copy_alone_and_forgets_the_saved_state() {
        let mut clipboard = FakeClipboard::holding("newer copy");
        let mut saved = saved_with_driver_text("mine", "driver");
        let plan = restore_saved_text(Some(&mut clipboard), &mut saved)
            .expect("the restore should succeed");
        assert_eq!(plan, RestorePlan::UserChangedClipboard);
        assert!(clipboard.writes.is_empty());
        assert_eq!(clipboard.current_text.as_deref(), Some("newer copy"));
        assert_eq!(saved.peek(), (None, None));
    }

    #[test]
    fn only_no_text_errors_count_as_no_text() {
        assert!(matches!(
            classify_read(Some(Ok("mine".to_owned()))),
            ClipboardReadOutcome::Text(value) if value == "mine"
        ));
        assert!(matches!(
            classify_read(None),
            ClipboardReadOutcome::NotReady
        ));
        for result in [
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
        assert!(matches!(
            outcome,
            Err(ClipboardReadProblem::Failed(
                ClipboardError::ClipboardOccupied
            ))
        ));
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
    fn not_ready_reads_cannot_save_the_original() {
        let mut saved = SavedUserClipboard::default();
        let outcome = saved.remember_first(|| ClipboardReadOutcome::NotReady);
        assert!(matches!(outcome, Err(ClipboardReadProblem::NotReady)));
        assert_eq!(saved.take(), (None, None));
    }

    #[test]
    fn restore_writes_back_only_while_the_clipboard_still_holds_the_driver_text() {
        let saved = ClipboardSnapshot::Text("mine".to_owned());
        assert_eq!(
            restore_plan(Some(&saved), Some("driver"), &text("driver")),
            RestorePlan::WriteText("mine".to_owned())
        );
        assert_eq!(
            restore_plan(Some(&saved), Some("driver"), &text("newer copy")),
            RestorePlan::UserChangedClipboard
        );
        assert_eq!(
            restore_plan(Some(&saved), Some("driver"), &ClipboardReadOutcome::NoText),
            RestorePlan::UserChangedClipboard
        );
        assert_eq!(
            restore_plan(Some(&saved), None, &text("driver")),
            RestorePlan::UserChangedClipboard
        );
    }

    #[test]
    fn restore_could_not_check_when_the_read_failed_or_was_not_ready() {
        let saved = ClipboardSnapshot::Text("mine".to_owned());
        for outcome in [
            ClipboardReadOutcome::NotReady,
            ClipboardReadOutcome::Failed(ClipboardError::ClipboardOccupied),
        ] {
            assert_eq!(
                restore_plan(Some(&saved), Some("driver"), &outcome),
                RestorePlan::CouldNotCheck
            );
        }
    }

    #[test]
    fn restore_plans_without_saved_text_ignore_the_clipboard() {
        let unreadable = ClipboardReadOutcome::NotReady;
        assert_eq!(
            restore_plan(Some(&ClipboardSnapshot::NoText), None, &unreadable),
            RestorePlan::NoTextWasSaved
        );
        assert_eq!(
            restore_plan(None, None, &unreadable),
            RestorePlan::NothingSaved
        );
    }

    #[test]
    fn peeking_and_forgetting_the_saved_state() {
        let mut saved = SavedUserClipboard::default();
        remembered(&mut saved, text("one"));
        saved.note_written("driver".to_owned());
        assert_eq!(
            saved.peek(),
            (
                Some(&ClipboardSnapshot::Text("one".to_owned())),
                Some("driver")
            )
        );
        saved.forget();
        assert_eq!(saved.peek(), (None, None));
    }

    #[test]
    fn read_problems_name_the_command_and_the_cause() {
        let message = ClipboardCommandError::Read(ClipboardReadProblem::Failed(
            ClipboardError::ClipboardOccupied,
        ))
        .to_string();
        let prefix = "could not read the clipboard to save it first, so it was not changed: ";
        assert!(message.starts_with(prefix), "{message}");

        let message = ClipboardCommandError::Check(ClipboardReadProblem::NotReady).to_string();
        assert_eq!(
            message,
            "could not read the clipboard to check it, so the saved text was kept for a later \
             clipboard_restore: the clipboard was not ready to be read"
        );
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
