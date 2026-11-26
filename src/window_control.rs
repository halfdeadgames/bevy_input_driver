//! Asking Bevy to minimize or restore the primary window.

use bevy::{ecs::query::QuerySingleError, prelude::*, window::PrimaryWindow};

use crate::status::record_error_in_world;

/// What to ask of the primary window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    /// Ask for the window to be minimized.
    Minimize,
    /// Ask for the window to be restored from minimized.
    Restore,
}

impl WindowAction {
    fn command_name(self) -> &'static str {
        match self {
            Self::Minimize => "window_minimize",
            Self::Restore => "window_restore",
        }
    }

    fn wants_minimized(self) -> bool {
        self == Self::Minimize
    }
}

/// Queues a world command that sets the minimize request on the primary window. It only
/// requests the change; the operating system may apply it later or not at all.
pub fn queue_window_action(commands: &mut Commands, action: WindowAction) {
    commands.queue(move |world: &mut World| apply_window_action(world, action));
}

fn apply_window_action(world: &mut World, action: WindowAction) {
    let mut primary_windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    let problem = match primary_windows.single_mut(world) {
        Ok(mut window) => {
            window.set_minimized(action.wants_minimized());
            return;
        }
        Err(QuerySingleError::NoEntities(_)) => "no primary window",
        Err(QuerySingleError::MultipleEntities(_)) => "more than one primary window",
    };
    let message = format!("{}: {problem}", action.command_name());
    record_error_in_world(world, message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::DriverStatus;

    fn world_with_status() -> World {
        let mut world = World::new();
        world.init_resource::<DriverStatus>();
        world
    }

    #[test]
    fn minimize_then_restore_sets_the_request() {
        let mut world = world_with_status();
        let window = world.spawn((Window::default(), PrimaryWindow)).id();

        apply_window_action(&mut world, WindowAction::Minimize);
        let request = world
            .get_mut::<Window>(window)
            .expect("window exists")
            .internal
            .take_minimize_request();
        assert_eq!(request, Some(true));

        apply_window_action(&mut world, WindowAction::Restore);
        let request = world
            .get_mut::<Window>(window)
            .expect("window exists")
            .internal
            .take_minimize_request();
        assert_eq!(request, Some(false));
        assert!(world.resource::<DriverStatus>().errors.is_empty());
    }

    #[test]
    fn missing_primary_window_records_an_error() {
        let mut world = world_with_status();
        apply_window_action(&mut world, WindowAction::Minimize);
        apply_window_action(&mut world, WindowAction::Restore);
        let errors = &world.resource::<DriverStatus>().errors;
        assert_eq!(errors.len(), 2);
        assert!(errors[0].contains("window_minimize: no primary window"));
        assert!(errors[1].contains("window_restore: no primary window"));
    }

    #[test]
    fn two_primary_windows_are_named_in_the_error_and_left_alone() {
        let mut world = world_with_status();
        let first_window = world.spawn((Window::default(), PrimaryWindow)).id();
        world.spawn((Window::default(), PrimaryWindow));

        apply_window_action(&mut world, WindowAction::Minimize);

        let errors = &world.resource::<DriverStatus>().errors;
        assert_eq!(
            errors,
            &vec!["window_minimize: more than one primary window".to_owned()]
        );
        let request = world
            .get_mut::<Window>(first_window)
            .expect("window exists")
            .internal
            .take_minimize_request();
        assert_eq!(request, None);
    }
}
