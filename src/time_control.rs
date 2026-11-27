//! Pausing, slowing, and single-stepping game time.

use std::{
    fmt,
    time::{Duration, TryFromFloatSecsError},
};

use bevy::{
    prelude::*,
    time::{Time, TimeUpdateStrategy, Virtual},
};

use crate::status::{DriverStatus, record_error_in_world};

// Bevy's own default, which `Time<Virtual>` does not expose publicly.
const DEFAULT_VIRTUAL_MAXIMUM_DELTA: Duration = Duration::from_millis(250);

/// One change to the game clock, applied by a world command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeControlAction {
    /// Stop `Time<Virtual>`.
    Pause,
    /// Let `Time<Virtual>` run again.
    Resume,
    /// Set the relative speed of `Time<Virtual>`.
    SetSpeed(f32),
    /// Make every frame advance real time by exactly this many milliseconds.
    FixedStep {
        /// Milliseconds per frame.
        milliseconds: f64,
    },
    /// Return to automatic real-time updates and speed 1.0, without changing the pause state.
    RealTime,
}

/// Why a [`TimeControlAction`] could not be applied.
#[derive(Debug)]
pub enum TimeControlError {
    /// A time resource is missing, so `TimePlugin` is not part of the app.
    MissingResource(&'static str),
    /// The speed is not a finite number above zero.
    InvalidSpeed(f32),
    /// The step cannot be represented as a duration.
    InvalidFixedStep(TryFromFloatSecsError),
}

impl fmt::Display for TimeControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingResource(name) => write!(formatter, "time control needs {name}"),
            Self::InvalidSpeed(factor) => {
                write!(
                    formatter,
                    "time speed {factor} is not a finite number above 0"
                )
            }
            Self::InvalidFixedStep(source) => write!(formatter, "invalid fixed step: {source}"),
        }
    }
}

impl std::error::Error for TimeControlError {}

fn virtual_time(world: &mut World) -> Result<Mut<'_, Time<Virtual>>, TimeControlError> {
    world
        .get_resource_mut::<Time<Virtual>>()
        .ok_or(TimeControlError::MissingResource("Time<Virtual>"))
}

fn update_strategy(world: &mut World) -> Result<Mut<'_, TimeUpdateStrategy>, TimeControlError> {
    world
        .get_resource_mut::<TimeUpdateStrategy>()
        .ok_or(TimeControlError::MissingResource("TimeUpdateStrategy"))
}

/// Applies one clock change directly to the world.
///
/// # Errors
///
/// Returns [`TimeControlError`] when a time resource is missing or the value is unusable.
pub fn apply_time_action(
    world: &mut World,
    action: TimeControlAction,
) -> Result<(), TimeControlError> {
    match action {
        TimeControlAction::Pause => virtual_time(world)?.pause(),
        TimeControlAction::Resume => virtual_time(world)?.unpause(),
        TimeControlAction::SetSpeed(factor) => {
            if !factor.is_finite() || factor <= 0.0 {
                return Err(TimeControlError::InvalidSpeed(factor));
            }
            virtual_time(world)?.set_relative_speed(factor);
        }
        TimeControlAction::FixedStep { milliseconds } => {
            let step = Duration::try_from_secs_f64(milliseconds / 1000.0)
                .map_err(TimeControlError::InvalidFixedStep)?;
            *update_strategy(world)? = TimeUpdateStrategy::ManualDuration(step);
            // Otherwise `Time<Virtual>` clamps the step and the game advances less than asked.
            let mut virtual_time = virtual_time(world)?;
            if step > virtual_time.max_delta() {
                virtual_time.set_max_delta(step);
            }
        }
        TimeControlAction::RealTime => {
            *update_strategy(world)? = TimeUpdateStrategy::Automatic;
            let mut virtual_time = virtual_time(world)?;
            virtual_time.set_relative_speed(1.0);
            virtual_time.set_max_delta(DEFAULT_VIRTUAL_MAXIMUM_DELTA);
        }
    }
    Ok(())
}

/// Queues a world command that applies `action`, recording any failure in [`DriverStatus`].
pub fn queue_time_action(commands: &mut Commands, action: TimeControlAction) {
    commands.queue(move |world: &mut World| {
        if let Err(problem) = apply_time_action(world, action) {
            record_error_in_world(world, problem.to_string());
        }
    });
}

/// Registers the system that reports the game clock in the status.
pub struct DriverTimeControlPlugin;

impl Plugin for DriverTimeControlPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Last, report_time_state);
    }
}

fn report_time_state(
    virtual_time: Res<Time<Virtual>>,
    update_strategy: Res<TimeUpdateStrategy>,
    mut status: ResMut<DriverStatus>,
) {
    status.paused = virtual_time.is_paused();
    status.speed = virtual_time.relative_speed();
    status.fixed_step_milliseconds = match *update_strategy {
        TimeUpdateStrategy::ManualDuration(step) => Some(step.as_secs_f64() * 1000.0),
        _ => None,
    };
    status.virtual_elapsed_seconds = virtual_time.elapsed_secs_f64();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_time() -> World {
        let mut world = World::new();
        world.init_resource::<Time<Virtual>>();
        world.init_resource::<TimeUpdateStrategy>();
        world
    }

    #[test]
    fn pause_speed_and_fixed_step_change_the_clock() {
        let mut world = world_with_time();
        apply_time_action(&mut world, TimeControlAction::Pause).expect("pause");
        apply_time_action(&mut world, TimeControlAction::SetSpeed(2.0)).expect("speed");
        let fixed_step = TimeControlAction::FixedStep { milliseconds: 20.0 };
        apply_time_action(&mut world, fixed_step).expect("fixed step");
        let virtual_time = world.resource::<Time<Virtual>>();
        assert!(virtual_time.is_paused());
        assert!((virtual_time.relative_speed() - 2.0).abs() < f32::EPSILON);
        assert!(matches!(
            world.resource::<TimeUpdateStrategy>(),
            TimeUpdateStrategy::ManualDuration(step) if *step == Duration::from_millis(20)
        ));
    }

    #[test]
    fn real_time_restores_speed_and_strategy_but_not_the_pause() {
        let mut world = world_with_time();
        apply_time_action(&mut world, TimeControlAction::Pause).expect("pause");
        apply_time_action(&mut world, TimeControlAction::SetSpeed(3.0)).expect("speed");
        let fixed_step = TimeControlAction::FixedStep { milliseconds: 5.0 };
        apply_time_action(&mut world, fixed_step).expect("fixed step");
        apply_time_action(&mut world, TimeControlAction::RealTime).expect("real time");
        let virtual_time = world.resource::<Time<Virtual>>();
        assert!(virtual_time.is_paused());
        assert!((virtual_time.relative_speed() - 1.0).abs() < f32::EPSILON);
        assert!(matches!(
            world.resource::<TimeUpdateStrategy>(),
            TimeUpdateStrategy::Automatic
        ));
    }

    #[test]
    fn large_fixed_step_raises_max_delta_and_real_time_restores_it() {
        let mut world = world_with_time();
        let fixed_step = TimeControlAction::FixedStep {
            milliseconds: 500.0,
        };
        apply_time_action(&mut world, fixed_step).expect("fixed step");
        let max_delta = world.resource::<Time<Virtual>>().max_delta();
        assert_eq!(max_delta, Duration::from_millis(500));
        apply_time_action(&mut world, TimeControlAction::RealTime).expect("real time");
        let max_delta = world.resource::<Time<Virtual>>().max_delta();
        assert_eq!(max_delta, DEFAULT_VIRTUAL_MAXIMUM_DELTA);
    }

    #[test]
    fn rejects_unusable_values_and_missing_resources() {
        let mut world = world_with_time();
        assert!(apply_time_action(&mut world, TimeControlAction::SetSpeed(0.0)).is_err());
        assert!(
            apply_time_action(
                &mut world,
                TimeControlAction::FixedStep {
                    milliseconds: f64::NAN
                }
            )
            .is_err()
        );
        let mut empty_world = World::new();
        assert!(apply_time_action(&mut empty_world, TimeControlAction::Pause).is_err());
    }
}
