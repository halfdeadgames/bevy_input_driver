//! Game-provided checks, run by the `expect` command, and the exit code they decide.

use std::{cmp::Ordering, collections::BTreeMap, fmt};

use bevy::{app::AppExit, ecs::system::SystemId, prelude::*};
use serde::Serialize;
use serde_json::{Number, Value};

use crate::status::DriverStatus;

/// One-shot system id of a check that receives the script's `args` and returns what it read.
pub type CheckSystemId = SystemId<In<Value>, Result<Value, String>>;

/// The registered checks by name. Fill it with [`InputDriverAppExt::add_driver_check`].
///
/// [`InputDriverAppExt::add_driver_check`]: crate::InputDriverAppExt::add_driver_check
#[derive(Resource, Debug, Default)]
pub struct DriverChecks {
    systems: BTreeMap<String, CheckSystemId>,
}

impl DriverChecks {
    /// Registers `system` under `name`, replacing any check with that name, and returns the
    /// replaced system so the caller can unregister it.
    pub fn insert(&mut self, name: String, system: CheckSystemId) -> Option<CheckSystemId> {
        self.systems.insert(name, system)
    }

    /// The system registered under `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<CheckSystemId> {
        self.systems.get(name).copied()
    }

    /// Every registered name, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.systems.keys().cloned().collect()
    }
}

/// The outcome of one `expect` command.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckResult {
    /// Name of the check.
    pub check: String,
    /// True when the check ran, returned a value, and matched `equals` if one was given.
    pub passed: bool,
    /// The value the check returned, when it returned one.
    pub value: Option<Value>,
    /// Why it failed; none when it passed.
    pub message: Option<String>,
}

impl CheckResult {
    fn passed(check: &str, value: Value) -> Self {
        Self {
            check: check.to_owned(),
            passed: true,
            value: Some(value),
            message: None,
        }
    }

    fn failed(check: &str, value: Option<Value>, message: &str) -> Self {
        Self {
            check: check.to_owned(),
            passed: false,
            value,
            message: Some(truncate_message(message)),
        }
    }
}

/// The most characters a failure message keeps.
const MAX_MESSAGE_CHARS: usize = 1000;

// Counts chars, not bytes, so the cut never lands inside a multi-byte character.
fn truncate_message(message: &str) -> String {
    match message.char_indices().nth(MAX_MESSAGE_CHARS) {
        None => message.to_owned(),
        Some((cut, _)) => format!("{}...", &message[..cut]),
    }
}

/// Why a check did not return a value.
#[derive(Debug, PartialEq, Eq)]
pub enum CheckError {
    /// No check has this name.
    Unknown {
        /// The requested name.
        name: String,
        /// Every registered name, sorted.
        registered: Vec<String>,
    },
    /// Bevy could not run the registered system.
    CannotRun(String),
    /// The check ran and returned an error.
    Failed(String),
}

impl fmt::Display for CheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { name, registered } if registered.is_empty() => write!(
                formatter,
                "no check named \"{name}\"; registered checks: none (register one with \
                 add_driver_check)"
            ),
            Self::Unknown { name, registered } => write!(
                formatter,
                "no check named \"{name}\"; registered checks: {}",
                registered.join(", ")
            ),
            Self::CannotRun(problem) => write!(formatter, "cannot run check: {problem}"),
            Self::Failed(problem) => write!(formatter, "{problem}"),
        }
    }
}

impl std::error::Error for CheckError {}

/// Runs the check registered as `name` with `args`.
///
/// # Errors
///
/// Returns [`CheckError`] when no check has that name, Bevy cannot run it, or it returns an error.
pub fn run_check(world: &mut World, name: &str, args: Value) -> Result<Value, CheckError> {
    let system = world
        .get_resource::<DriverChecks>()
        .and_then(|checks| checks.get(name))
        .ok_or_else(|| CheckError::Unknown {
            name: name.to_owned(),
            registered: world
                .get_resource::<DriverChecks>()
                .map(DriverChecks::names)
                .unwrap_or_default(),
        })?;
    world
        .run_system_with::<In<Value>, Result<Value, String>>(system, args)
        .map_err(|problem| CheckError::CannotRun(problem.to_string()))?
        .map_err(CheckError::Failed)
}

/// True when two JSON values are equal, comparing numbers by numeric value (so 5 equals 5.0)
/// at any depth and everything else exactly.
#[must_use]
pub fn json_values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => numbers_equal(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_values_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| json_values_equal(left, right))
                })
        }
        (left, right) => left == right,
    }
}

// Integers compare exactly so large ones are not blurred by the float conversion.
fn numbers_equal(left: &Number, right: &Number) -> bool {
    match (integer_value(left), integer_value(right)) {
        (Some(left), Some(right)) => left == right,
        (Some(integer), None) => integer_equals_float(integer, right.as_f64()),
        (None, Some(integer)) => integer_equals_float(integer, left.as_f64()),
        (None, None) => floats_equal(left.as_f64(), right.as_f64()),
    }
}

fn integer_value(number: &Number) -> Option<i128> {
    number
        .as_i64()
        .map(i128::from)
        .or_else(|| number.as_u64().map(i128::from))
}

// Above 2^53 several integers share one f64, so the float must also convert back to this exact
// integer.
fn integer_equals_float(integer: i128, float: Option<f64>) -> bool {
    let integer_as_float = integer as f64;
    floats_equal(Some(integer_as_float), float) && integer_as_float as i128 == integer
}

fn floats_equal(left: Option<f64>, right: Option<f64>) -> bool {
    matches!(
        (left, right),
        (Some(left), Some(right)) if left.partial_cmp(&right) == Some(Ordering::Equal)
    )
}

/// Runs the check and judges it against `equals`.
pub fn evaluate_check(
    world: &mut World,
    name: &str,
    args: Value,
    equals: Option<&Value>,
) -> CheckResult {
    match run_check(world, name, args) {
        Err(problem) => CheckResult::failed(name, None, &problem.to_string()),
        Ok(value) => match equals {
            Some(expected) if !json_values_equal(expected, &value) => {
                let message = format!("expected {expected}, got {value}");
                CheckResult::failed(name, Some(value), &message)
            }
            _ => CheckResult::passed(name, value),
        },
    }
}

/// Queues a world command that runs the check and records the result in [`DriverStatus`].
pub fn queue_check(commands: &mut Commands, name: String, args: Value, equals: Option<Value>) {
    commands.queue(move |world: &mut World| {
        let result = evaluate_check(world, &name, args, equals.as_ref());
        world
            .resource_mut::<DriverStatus>()
            .note_check_result(result);
    });
}

/// The exit a `quit` produces: an error exit when any check has failed, else success.
#[must_use]
pub fn exit_for_failed_checks(checks_failed: u64) -> AppExit {
    if checks_failed > 0 {
        AppExit::error()
    } else {
        AppExit::Success
    }
}

/// Queues a world command that exits the app. It reads the failure count only when it runs, so
/// an `expect` queued earlier in the same frame is already counted.
pub fn queue_quit(commands: &mut Commands) {
    commands.queue(|world: &mut World| {
        let checks_failed = world.resource::<DriverStatus>().checks_failed;
        world
            .resource_mut::<Messages<AppExit>>()
            .write(exit_for_failed_checks(checks_failed));
    });
}

/// Stores the check under `name`, creating [`DriverChecks`] if it does not exist yet.
pub fn register_driver_check<Marker>(
    app: &mut App,
    name: String,
    check_system: impl IntoSystem<In<Value>, Result<Value, String>, Marker> + 'static,
) {
    let system = app.register_system(check_system);
    let replaced_system = app
        .world_mut()
        .get_resource_or_insert_with(DriverChecks::default)
        .insert(name, system);
    if let Some(replaced_system) = replaced_system {
        // Already gone only if the game unregistered it itself, which leaves nothing to clean up.
        let _already_unregistered = app.world_mut().unregister_system(replaced_system);
    }
}

/// Creates [`DriverChecks`] when no check was registered before the plugin was added.
pub struct DriverChecksPlugin;

impl Plugin for DriverChecksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DriverChecks>();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn returns_five(In(_args): In<Value>) -> Result<Value, String> {
        Ok(json!(5))
    }

    fn echoes_args(In(args): In<Value>) -> Result<Value, String> {
        Ok(args)
    }

    fn always_fails(In(_args): In<Value>) -> Result<Value, String> {
        Err("health is wrong".to_owned())
    }

    fn world_with_checks() -> World {
        let mut world = World::new();
        world.init_resource::<DriverStatus>();
        let mut checks = DriverChecks::default();
        checks.insert("five".to_owned(), world.register_system(returns_five));
        checks.insert("echo".to_owned(), world.register_system(echoes_args));
        checks.insert("fail".to_owned(), world.register_system(always_fails));
        world.insert_resource(checks);
        world
    }

    fn expect(world: &mut World, name: &str, args: Value, equals: Option<Value>) {
        queue_check(&mut world.commands(), name.to_owned(), args, equals);
        world.flush();
    }

    #[test]
    fn numbers_compare_by_value_at_any_depth() {
        assert!(json_values_equal(&json!(5), &json!(5.0)));
        assert!(json_values_equal(
            &json!([1, {"a": 2}]),
            &json!([1.0, {"a": 2.0}])
        ));
        assert!(!json_values_equal(&json!(5), &json!(5.5)));
        assert!(json_values_equal(&json!(-3), &json!(-3.0)));
        assert!(json_values_equal(&json!(u64::MAX), &json!(u64::MAX)));
        assert!(json_values_equal(&json!(7_u64), &json!(7_i64)));
        assert!(!json_values_equal(
            &json!(9_007_199_254_740_993_u64),
            &json!(9_007_199_254_740_992.0)
        ));
        assert!(!json_values_equal(
            &json!(i64::MAX),
            &json!(9_223_372_036_854_775_808.0)
        ));
        assert!(!json_values_equal(&json!("5"), &json!(5)));
        assert!(!json_values_equal(&json!([1, 2]), &json!([1])));
        assert!(!json_values_equal(&json!({"a": 1}), &json!({"b": 1})));
        assert!(!json_values_equal(
            &json!({"a": 1}),
            &json!({"a": 1, "b": 1})
        ));
        assert!(!json_values_equal(&json!(true), &json!(1)));
        assert!(json_values_equal(&json!(u64::MAX), &json!(u64::MAX)));
        assert!(!json_values_equal(&json!(u64::MAX), &json!(u64::MAX - 1)));
    }

    #[test]
    fn a_passing_check_records_its_value() {
        let mut world = world_with_checks();
        expect(&mut world, "five", Value::Null, Some(json!(5.0)));
        let status = world.resource::<DriverStatus>();
        assert_eq!(status.checks_failed, 0);
        assert!(status.errors.is_empty());
        assert_eq!(
            status.checks,
            vec![CheckResult {
                check: "five".to_owned(),
                passed: true,
                value: Some(json!(5)),
                message: None,
            }]
        );
    }

    #[test]
    fn the_check_receives_the_script_args() {
        let mut world = world_with_checks();
        expect(&mut world, "echo", json!({"who": 2}), None);
        assert_eq!(
            world.resource::<DriverStatus>().checks[0].value,
            Some(json!({"who": 2}))
        );
    }

    #[test]
    fn a_check_that_returns_an_error_fails() {
        let mut world = world_with_checks();
        expect(&mut world, "fail", Value::Null, None);
        let status = world.resource::<DriverStatus>();
        assert_eq!(status.checks_failed, 1);
        assert_eq!(status.checks[0].value, None);
        assert_eq!(status.checks[0].message.as_deref(), Some("health is wrong"));
        assert_eq!(
            status.errors,
            vec!["expect fail: health is wrong".to_owned()]
        );
    }

    #[test]
    fn an_equals_mismatch_fails_with_both_values() {
        let mut world = world_with_checks();
        expect(&mut world, "five", Value::Null, Some(json!(6)));
        let status = world.resource::<DriverStatus>();
        assert_eq!(status.checks_failed, 1);
        assert_eq!(status.checks[0].value, Some(json!(5)));
        assert_eq!(
            status.checks[0].message.as_deref(),
            Some("expected 6, got 5")
        );
        assert_eq!(
            status.errors,
            vec!["expect five: expected 6, got 5".to_owned()]
        );
    }

    fn returns_null(In(_args): In<Value>) -> Result<Value, String> {
        Ok(Value::Null)
    }

    #[test]
    fn equals_null_fails_for_a_value_and_passes_for_null() {
        let mut world = world_with_checks();
        let null_check = world.register_system(returns_null);
        world
            .resource_mut::<DriverChecks>()
            .insert("null".to_owned(), null_check);
        expect(&mut world, "five", Value::Null, Some(Value::Null));
        expect(&mut world, "null", Value::Null, Some(Value::Null));
        let status = world.resource::<DriverStatus>();
        assert!(!status.checks[0].passed);
        assert_eq!(
            status.checks[0].message.as_deref(),
            Some("expected null, got 5")
        );
        assert!(status.checks[1].passed);
    }

    #[test]
    fn long_messages_are_cut_at_a_char_boundary() {
        let short = "a".repeat(MAX_MESSAGE_CHARS);
        assert_eq!(truncate_message(&short), short);

        let long = format!("{}é{}", "a".repeat(MAX_MESSAGE_CHARS - 1), "b".repeat(50));
        let cut = truncate_message(&long);
        assert_eq!(cut.chars().count(), MAX_MESSAGE_CHARS + 3);
        assert!(cut.ends_with("é..."), "{cut}");

        let straddling = format!("{}éé", "a".repeat(MAX_MESSAGE_CHARS - 1));
        assert!(truncate_message(&straddling).ends_with("é..."));
    }

    #[test]
    fn an_unknown_name_lists_the_registered_checks() {
        let mut world = world_with_checks();
        expect(&mut world, "nope", Value::Null, None);
        let status = world.resource::<DriverStatus>();
        assert_eq!(status.checks_failed, 1);
        assert_eq!(
            status.checks[0].message.as_deref(),
            Some("no check named \"nope\"; registered checks: echo, fail, five")
        );

        let mut empty = World::new();
        empty.init_resource::<DriverStatus>();
        expect(&mut empty, "nope", Value::Null, None);
        let message = empty.resource::<DriverStatus>().checks[0]
            .message
            .clone()
            .unwrap_or_default();
        assert!(message.contains("registered checks: none"), "{message}");
        assert!(message.contains("add_driver_check"), "{message}");
    }

    #[test]
    fn the_cap_drops_old_results_but_keeps_counting_failures() {
        let mut world = world_with_checks();
        for _ in 0..60 {
            expect(&mut world, "fail", Value::Null, None);
        }
        let status = world.resource::<DriverStatus>();
        assert_eq!(status.checks.len(), 50);
        assert_eq!(status.checks_failed, 60);
    }

    #[test]
    fn failed_checks_choose_the_error_exit() {
        assert_eq!(exit_for_failed_checks(0), AppExit::Success);
        assert_eq!(exit_for_failed_checks(1), AppExit::error());
        assert_eq!(exit_for_failed_checks(u64::MAX), AppExit::error());
    }

    #[test]
    fn quit_sees_a_check_queued_before_it_in_the_same_frame() {
        let mut world = world_with_checks();
        world.init_resource::<Messages<AppExit>>();
        queue_check(
            &mut world.commands(),
            "five".to_owned(),
            Value::Null,
            Some(json!(6)),
        );
        queue_quit(&mut world.commands());
        world.flush();
        let mut cursor = world.resource_mut::<Messages<AppExit>>().get_cursor();
        let exits: Vec<AppExit> = cursor
            .read(world.resource::<Messages<AppExit>>())
            .cloned()
            .collect();
        assert_eq!(exits, vec![AppExit::error()]);
    }
}
