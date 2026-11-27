//! BRP methods: `drive/send`, `drive/status`, and `drive/clear`.

use bevy::{
    ecs::system::SystemId,
    prelude::*,
    remote::{
        BrpError, BrpResult, RemoteMethodSystemId, RemoteMethods, RemotePlugin, error_codes,
        http::RemoteHttpPlugin,
    },
};
use serde_json::{Value, json};

use crate::{
    commands::{DriverCommand, parse_command_value},
    real_input::RealInputBlocking,
    status::DriverStatus,
    steps::DriverQueue,
};

/// BRP method that queues one command or an array of commands.
pub const SEND_METHOD: &str = "drive/send";
/// BRP method that returns the driver status.
pub const STATUS_METHOD: &str = "drive/status";
/// BRP method that empties the queue and releases everything held.
pub const CLEAR_METHOD: &str = "drive/clear";

/// One-shot system id of a game-provided BRP method.
pub type RemoteMethodSystem = SystemId<In<Option<Value>>, BrpResult>;

/// Game methods registered before `RemotePlugin` built `RemoteMethods`.
#[derive(Resource, Debug, Default)]
pub struct PendingRemoteMethods {
    methods: Vec<(String, RemoteMethodSystem)>,
}

/// Registers the handler as the BRP method `name`, now if `RemoteMethods` exists, else when
/// [`DriverRemotePlugin`] builds or, at the latest, in `Startup`.
///
/// Names starting with `drive/` belong to the driver: they are refused with an error log and
/// nothing is registered.
pub fn register_remote_method<Marker>(
    app: &mut App,
    name: String,
    handler: impl IntoSystem<In<Option<Value>>, BrpResult, Marker> + 'static,
) {
    if name.starts_with(RESERVED_PREFIX) {
        error!(
            "input driver: refused game remote method \"{name}\": names starting with \
             \"{RESERVED_PREFIX}\" are reserved for the driver"
        );
        return;
    }
    let system = app.register_system(handler);
    if let Some(mut methods) = app.world_mut().get_resource_mut::<RemoteMethods>() {
        insert_game_method(&mut methods, name, system);
        return;
    }
    if !app.world().contains_resource::<PendingRemoteMethods>() {
        app.init_resource::<PendingRemoteMethods>()
            .add_systems(Startup, drain_pending_methods_at_startup);
    }
    app.world_mut()
        .resource_mut::<PendingRemoteMethods>()
        .methods
        .push((name, system));
}

const RESERVED_PREFIX: &str = "drive/";

fn insert_game_method(methods: &mut RemoteMethods, name: String, system: RemoteMethodSystem) {
    let replaced = methods.insert(name.clone(), RemoteMethodSystemId::Instant(system));
    if replaced.is_some() {
        warn!("input driver: game remote method \"{name}\" replaced an existing method");
    }
}

fn drain_pending_remote_methods(world: &mut World) {
    if !world.contains_resource::<RemoteMethods>() {
        return;
    }
    let Some(pending) = world.remove_resource::<PendingRemoteMethods>() else {
        return;
    };
    let mut methods = world.resource_mut::<RemoteMethods>();
    for (name, system) in pending.methods {
        insert_game_method(&mut methods, name, system);
    }
}

// Startup runs after every plugin is built, so a game-added `RemotePlugin` exists by now.
fn drain_pending_methods_at_startup(world: &mut World) {
    drain_pending_remote_methods(world);
    let Some(pending) = world.get_resource::<PendingRemoteMethods>() else {
        return;
    };
    let names: Vec<&str> = pending
        .methods
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    warn!(
        "input driver: remote methods not served because no RemotePlugin exists (set \
         enable_remote to true or add your own RemotePlugin): {}",
        names.join(", ")
    );
}

/// Adds `RemotePlugin` with the driver methods, and the HTTP transport on `port`.
///
/// The game must not add `RemotePlugin` itself while this plugin is added.
pub struct DriverRemotePlugin {
    /// TCP port on 127.0.0.1.
    pub port: u16,
}

impl Plugin for DriverRemotePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RemotePlugin::default()
                .with_method_main(SEND_METHOD, send_commands_method)
                .with_method_main(STATUS_METHOD, status_method)
                .with_method_main(CLEAR_METHOD, clear_method),
            RemoteHttpPlugin::default().with_port(self.port),
        ));
        drain_pending_remote_methods(app.world_mut());
    }
}

fn invalid_parameters_error(message: String) -> BrpError {
    BrpError {
        code: error_codes::INVALID_PARAMS,
        message,
        data: None,
    }
}

/// Parses `drive/send` params: one command object, or an array of them. All or nothing.
///
/// # Errors
///
/// Returns a [`BrpError`] with the invalid-params code naming the failing array index.
pub fn parse_send_parameters(parameters: Option<Value>) -> Result<Vec<DriverCommand>, BrpError> {
    match parameters {
        None | Some(Value::Null) => Err(invalid_parameters_error(
            "drive/send needs params: a command object or an array of command objects".to_owned(),
        )),
        Some(Value::Array(items)) => items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                parse_command_value(item).map_err(|problem| {
                    invalid_parameters_error(format!("command {index}: {problem}"))
                })
            })
            .collect(),
        Some(single @ Value::Object(_)) => parse_command_value(single)
            .map(|command| vec![command])
            .map_err(|problem| invalid_parameters_error(format!("command: {problem}"))),
        Some(other) => Err(invalid_parameters_error(format!(
            "drive/send params must be an object or an array, got {other}"
        ))),
    }
}

fn status_json(status: &DriverStatus) -> BrpResult {
    serde_json::to_value(status).map_err(BrpError::internal)
}

fn send_commands_method(
    In(parameters): In<Option<Value>>,
    mut queue: ResMut<DriverQueue>,
    mut blocking: ResMut<RealInputBlocking>,
) -> BrpResult {
    let commands = parse_send_parameters(parameters)?;
    queue.enqueue_commands(&commands);
    if !commands.is_empty() {
        blocking.block_for_first_command();
    }
    Ok(json!({ "queued": commands.len(), "queue_length": queue.step_count() }))
}

fn status_method(
    In(_parameters): In<Option<Value>>,
    mut status: ResMut<DriverStatus>,
    queue: Res<DriverQueue>,
    blocking: Res<RealInputBlocking>,
) -> BrpResult {
    status.synchronize_with_queue(&queue);
    status.real_input_blocked = blocking.is_blocked();
    status_json(&status)
}

fn clear_method(
    In(_parameters): In<Option<Value>>,
    mut status: ResMut<DriverStatus>,
    mut queue: ResMut<DriverQueue>,
    blocking: Res<RealInputBlocking>,
) -> BrpResult {
    queue.clear_and_release_all();
    status.synchronize_with_queue(&queue);
    status.real_input_blocked = blocking.is_blocked();
    status_json(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_object_or_an_array() {
        let one =
            parse_send_parameters(Some(json!({"command": "quit"}))).expect("object should parse");
        assert_eq!(one, vec![DriverCommand::Quit {}]);
        let many = parse_send_parameters(Some(
            json!([{"command": "quit"}, {"command": "release_all"}]),
        ))
        .expect("array should parse");
        assert_eq!(many.len(), 2);
    }

    #[test]
    fn rejects_everything_when_one_command_is_bad() {
        let error = parse_send_parameters(Some(json!([{"command": "quit"}, {"command": "nope"}])))
            .expect_err("bad command should fail");
        assert_eq!(error.code, error_codes::INVALID_PARAMS);
        assert!(error.message.contains("command 1"), "{}", error.message);
    }

    fn echo_parameters(In(parameters): In<Option<Value>>) -> BrpResult {
        Ok(parameters.unwrap_or(Value::Null))
    }

    #[test]
    fn game_methods_registered_before_the_remote_plugin_are_added_when_it_builds() {
        let mut app = App::new();
        register_remote_method(&mut app, "game/echo".to_owned(), echo_parameters);
        app.add_plugins(RemotePlugin::default());
        drain_pending_remote_methods(app.world_mut());
        let methods = app.world().resource::<RemoteMethods>();
        assert!(methods.get("game/echo").is_some());
        assert!(app.world().get_resource::<PendingRemoteMethods>().is_none());
    }

    #[test]
    fn game_methods_registered_after_the_remote_plugin_are_added_at_once() {
        let mut app = App::new();
        app.add_plugins(RemotePlugin::default());
        register_remote_method(&mut app, "game/echo".to_owned(), echo_parameters);
        let methods = app.world().resource::<RemoteMethods>();
        assert!(methods.get("game/echo").is_some());
        assert!(app.world().get_resource::<PendingRemoteMethods>().is_none());
    }

    #[test]
    fn startup_serves_pending_methods_for_a_remote_plugin_added_afterward() {
        let mut app = App::new();
        register_remote_method(&mut app, "game/echo".to_owned(), echo_parameters);
        app.add_plugins(RemotePlugin::default());
        app.world_mut().run_schedule(Startup);
        let methods = app.world().resource::<RemoteMethods>();
        assert!(methods.get("game/echo").is_some());
        assert!(app.world().get_resource::<PendingRemoteMethods>().is_none());
    }

    #[test]
    fn startup_without_a_remote_plugin_keeps_the_pending_methods() {
        let mut app = App::new();
        register_remote_method(&mut app, "game/echo".to_owned(), echo_parameters);
        app.world_mut().run_schedule(Startup);
        assert!(app.world().get_resource::<PendingRemoteMethods>().is_some());
    }

    fn instant_id(methods: &RemoteMethods, name: &str) -> Option<RemoteMethodSystem> {
        match methods.get(name) {
            Some(RemoteMethodSystemId::Instant(id)) => Some(*id),
            _ => None,
        }
    }

    #[test]
    fn game_methods_cannot_take_driver_names() {
        let mut app = App::new();
        app.add_plugins(
            RemotePlugin::default().with_method_main(SEND_METHOD, send_commands_method),
        );
        let driver_id = instant_id(app.world().resource::<RemoteMethods>(), SEND_METHOD);
        assert!(driver_id.is_some());
        register_remote_method(&mut app, SEND_METHOD.to_owned(), echo_parameters);
        register_remote_method(&mut app, "drive/other".to_owned(), echo_parameters);
        let methods = app.world().resource::<RemoteMethods>();
        assert_eq!(instant_id(methods, SEND_METHOD), driver_id);
        assert!(methods.get("drive/other").is_none());
    }

    fn send_through_a_world(world: &mut World, parameters: Value) {
        world
            .run_system_cached_with(send_commands_method, Some(parameters))
            .expect("system should run")
            .expect("commands should be accepted");
    }

    #[test]
    fn the_first_live_command_turns_real_input_blocking_on() {
        let mut world = World::new();
        world.init_resource::<DriverQueue>();
        world.init_resource::<RealInputBlocking>();
        assert!(!world.resource::<RealInputBlocking>().is_blocked());

        send_through_a_world(&mut world, json!({"command": "release_all"}));
        assert!(world.resource::<RealInputBlocking>().is_blocked());

        world.resource_mut::<RealInputBlocking>().set_blocked(false);
        send_through_a_world(&mut world, json!([{"command": "release_all"}]));
        assert!(!world.resource::<RealInputBlocking>().is_blocked());
    }

    #[test]
    fn an_empty_batch_does_not_turn_blocking_on() {
        let mut world = World::new();
        world.init_resource::<DriverQueue>();
        world.init_resource::<RealInputBlocking>();
        send_through_a_world(&mut world, json!([]));
        assert!(!world.resource::<RealInputBlocking>().is_blocked());
    }

    #[test]
    fn rejects_missing_and_scalar_parameters() {
        assert!(parse_send_parameters(None).is_err());
        assert!(parse_send_parameters(Some(json!(5))).is_err());
    }
}
