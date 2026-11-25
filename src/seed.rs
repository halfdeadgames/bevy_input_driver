//! The seed a game uses to make its randomness repeatable.

use std::fmt;

use bevy::prelude::*;

use crate::status::DriverStatus;

const SEED_ARGUMENT: &str = "--seed";

/// Seed for the game's random number generator, set by `--seed <u64>`, the plugin's `seed`
/// field, or the `set_seed` command.
///
/// Contract for games: build your single seeded random generator from this resource at
/// startup, and optionally reseed it when `Changed<DriverSeed>` fires. When it holds `None`,
/// pick your own seed. The driver does not depend on any random number crate.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriverSeed(pub Option<u64>);

/// Why a `--seed` argument could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedArgumentError {
    /// `--seed` was the last argument, with no value after it.
    MissingValue,
    /// The value is not an unsigned 64-bit integer.
    InvalidValue(String),
}

impl fmt::Display for SeedArgumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue => write!(formatter, "{SEED_ARGUMENT} needs a number after it"),
            Self::InvalidValue(value) => write!(
                formatter,
                "{SEED_ARGUMENT} value {value:?} is not an unsigned 64-bit integer"
            ),
        }
    }
}

impl std::error::Error for SeedArgumentError {}

fn parse_seed_value(value: &str) -> Result<u64, SeedArgumentError> {
    value
        .parse::<u64>()
        .map_err(|_| SeedArgumentError::InvalidValue(value.to_owned()))
}

/// Finds the seed in process arguments, as `--seed <u64>` or `--seed=<u64>`. The first
/// argument is the program name and is ignored by callers.
///
/// # Errors
///
/// Returns [`SeedArgumentError`] when the flag has no value or the value is not a `u64`.
pub fn seed_from_arguments(
    arguments: impl IntoIterator<Item = String>,
) -> Result<Option<u64>, SeedArgumentError> {
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        if argument == SEED_ARGUMENT {
            let value = arguments.next().ok_or(SeedArgumentError::MissingValue)?;
            return parse_seed_value(&value).map(Some);
        }
        if let Some(value) = argument.strip_prefix("--seed=") {
            return parse_seed_value(value).map(Some);
        }
    }
    Ok(None)
}

/// A `--seed` problem found while building the plugin, reported once the status exists.
#[derive(Resource, Debug)]
struct SeedArgumentProblem(Option<String>);

/// Inserts [`DriverSeed`] and reports it in the status.
pub struct DriverSeedPlugin {
    /// Seed used when no `--seed` argument is given.
    pub seed: Option<u64>,
}

impl Plugin for DriverSeedPlugin {
    fn build(&self, app: &mut App) {
        let (argument_seed, problem) = match seed_from_arguments(std::env::args().skip(1)) {
            Ok(seed) => (seed, None),
            Err(problem) => (None, Some(problem.to_string())),
        };
        app.insert_resource(DriverSeed(argument_seed.or(self.seed)))
            .insert_resource(SeedArgumentProblem(problem))
            .add_systems(Startup, report_seed_argument_problem)
            .add_systems(
                Last,
                report_seed_in_status.run_if(resource_changed::<DriverSeed>),
            );
    }
}

fn report_seed_argument_problem(
    problem: Res<SeedArgumentProblem>,
    mut status: ResMut<DriverStatus>,
) {
    if let Some(message) = &problem.0 {
        status.record_error(message.clone());
    }
}

fn report_seed_in_status(seed: Res<DriverSeed>, mut status: ResMut<DriverStatus>) {
    status.seed = seed.0;
}

/// Queues a world command that makes `seed` the current [`DriverSeed`].
pub fn queue_set_seed(commands: &mut Commands, seed: u64) {
    commands.queue(move |world: &mut World| {
        world.insert_resource(DriverSeed(Some(seed)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn finds_seed_argument_in_both_forms() {
        assert_eq!(
            seed_from_arguments(arguments(&["--seed", "42"])),
            Ok(Some(42))
        );
        assert_eq!(seed_from_arguments(arguments(&["--seed=7"])), Ok(Some(7)));
        assert_eq!(
            seed_from_arguments(arguments(&["--other", "--seed", "18446744073709551615"])),
            Ok(Some(u64::MAX))
        );
        assert_eq!(seed_from_arguments(arguments(&["--other"])), Ok(None));
    }

    #[test]
    fn rejects_missing_and_invalid_seed_values() {
        assert_eq!(
            seed_from_arguments(arguments(&["--seed"])),
            Err(SeedArgumentError::MissingValue)
        );
        assert_eq!(
            seed_from_arguments(arguments(&["--seed", "-3"])),
            Err(SeedArgumentError::InvalidValue("-3".to_owned()))
        );
        assert!(seed_from_arguments(arguments(&["--seed=abc"])).is_err());
    }
}
