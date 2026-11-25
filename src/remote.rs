//! BRP methods: `drive/send`, `drive/status`, and `drive/clear`.

use bevy::{
    prelude::*,
    remote::{BrpError, BrpResult, RemotePlugin, error_codes, http::RemoteHttpPlugin},
};
use serde_json::{Value, json};

use crate::{
    commands::{DriverCommand, parse_command_value},
    status::DriverStatus,
    steps::DriverQueue,
};

/// BRP method that queues one command or an array of commands.
pub const SEND_METHOD: &str = "drive/send";
/// BRP method that returns the driver status.
pub const STATUS_METHOD: &str = "drive/status";
/// BRP method that empties the queue and releases everything held.
pub const CLEAR_METHOD: &str = "drive/clear";

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
    }
}

fn invalid_params(message: String) -> BrpError {
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
pub fn parse_send_params(params: Option<Value>) -> Result<Vec<DriverCommand>, BrpError> {
    match params {
        None | Some(Value::Null) => Err(invalid_params(
            "drive/send needs params: a command object or an array of command objects".to_owned(),
        )),
        Some(Value::Array(items)) => items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                parse_command_value(item)
                    .map_err(|problem| invalid_params(format!("command {index}: {problem}")))
            })
            .collect(),
        Some(single @ Value::Object(_)) => parse_command_value(single)
            .map(|command| vec![command])
            .map_err(|problem| invalid_params(format!("command: {problem}"))),
        Some(other) => Err(invalid_params(format!(
            "drive/send params must be an object or an array, got {other}"
        ))),
    }
}

fn status_json(status: &DriverStatus) -> BrpResult {
    serde_json::to_value(status).map_err(BrpError::internal)
}

fn send_commands_method(
    In(params): In<Option<Value>>,
    mut queue: ResMut<DriverQueue>,
) -> BrpResult {
    let commands = parse_send_params(params)?;
    queue.enqueue_commands(&commands);
    Ok(json!({ "queued": commands.len(), "queue_length": queue.step_count() }))
}

fn status_method(
    In(_params): In<Option<Value>>,
    mut status: ResMut<DriverStatus>,
    queue: Res<DriverQueue>,
) -> BrpResult {
    status.synchronize_with_queue(&queue);
    status_json(&status)
}

fn clear_method(
    In(_params): In<Option<Value>>,
    mut status: ResMut<DriverStatus>,
    mut queue: ResMut<DriverQueue>,
) -> BrpResult {
    queue.clear_and_release_all();
    status.synchronize_with_queue(&queue);
    status_json(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_object_or_an_array() {
        let one = parse_send_params(Some(json!({"cmd": "quit"}))).expect("object should parse");
        assert_eq!(one, vec![DriverCommand::Quit {}]);
        let many = parse_send_params(Some(json!([{"cmd": "quit"}, {"cmd": "release_all"}])))
            .expect("array should parse");
        assert_eq!(many.len(), 2);
    }

    #[test]
    fn rejects_everything_when_one_command_is_bad() {
        let error = parse_send_params(Some(json!([{"cmd": "quit"}, {"cmd": "nope"}])))
            .expect_err("bad command should fail");
        assert_eq!(error.code, error_codes::INVALID_PARAMS);
        assert!(error.message.contains("command 1"), "{}", error.message);
    }

    #[test]
    fn rejects_missing_and_scalar_params() {
        assert!(parse_send_params(None).is_err());
        assert!(parse_send_params(Some(json!(5))).is_err());
    }
}
