//! Deterministic test scene for checking the input driver by eye and by screenshot.

use std::{fs, path::PathBuf};

use bevy::{
    input::{
        ButtonState,
        gestures::{DoubleTapGesture, PanGesture, PinchGesture, RotationGesture},
        keyboard::KeyboardInput,
        mouse::MouseWheel,
        touch::Touches,
    },
    prelude::*,
    remote::BrpResult,
    ui::UiGlobalTransform,
    window::{PrimaryWindow, WindowResolution},
};
use bevy_input_driver::{DriverSeed, InputDriverAppExt, InputDriverPlugin};
use serde_json::{Value, json};

const DARK_BLUE: Color = Color::srgb(0.05, 0.1, 0.45);
const BRIGHT_GREEN: Color = Color::srgb(0.1, 0.95, 0.2);
const ORANGE: Color = Color::srgb(1.0, 0.5, 0.0);
const TEXT_SIZE: f32 = 18.0;

#[derive(Component)]
struct ToggleButton {
    is_green: bool,
}

#[derive(Component)]
struct DraggableSquare {
    top_left: Vec2,
}

#[derive(Component)]
struct TypedTextLine;

#[derive(Component)]
struct StatusTextLine;

#[derive(Resource)]
struct TypedText(String);

impl Default for TypedText {
    fn default() -> Self {
        Self("typed: ".to_owned())
    }
}

#[derive(Resource, Default)]
struct InputTotals {
    scroll_y: f32,
    pinch: f32,
    rotation: f32,
    pan: Vec2,
    double_taps: u32,
}

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "driver demo".to_owned(),
                resolution: WindowResolution::new(800, 600),
                resizable: false,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(InputDriverPlugin::default())
        .set_driver_state_handlers(save_demo_state, load_demo_state)
        .add_driver_check("button_is_green", button_is_green)
        .add_driver_check("square_top_left", square_top_left)
        .add_driver_target("node_center", node_center)
        .add_driver_remote_method("demo/typed_text", typed_text_method)
        .insert_resource(ClearColor(Color::srgb(0.15, 0.15, 0.15)))
        .init_resource::<TypedText>()
        .init_resource::<InputTotals>()
        .add_systems(
            Startup,
            (
                spawn_camera,
                spawn_toggle_button,
                spawn_draggable_square,
                spawn_typed_text_line,
                spawn_status_text_line,
            ),
        )
        .add_systems(
            Update,
            (
                collect_typed_text,
                accumulate_scroll,
                accumulate_pinch,
                accumulate_rotation,
                accumulate_pan,
                count_double_taps,
                show_typed_text,
                show_status,
            ),
        )
        .run()
}

fn absolute_node(left: f32, top: f32, width: f32, height: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(left),
        top: Val::Px(top),
        width: Val::Px(width),
        height: Val::Px(height),
        ..default()
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn spawn_toggle_button(mut commands: Commands) {
    commands
        .spawn((
            absolute_node(40.0, 40.0, 200.0, 80.0),
            BackgroundColor(DARK_BLUE),
            ToggleButton { is_green: false },
            Name::new("button"),
        ))
        .observe(toggle_button_color);
}

fn toggle_button_color(
    click: On<PointerClick>,
    mut buttons: Query<(&mut ToggleButton, &mut BackgroundColor)>,
) -> Result {
    let (mut button, mut background) = buttons.get_mut(click.event_target())?;
    button.is_green = !button.is_green;
    background.0 = if button.is_green {
        BRIGHT_GREEN
    } else {
        DARK_BLUE
    };
    Ok(())
}

fn spawn_draggable_square(mut commands: Commands) {
    let top_left = Vec2::new(400.0, 250.0);
    commands
        .spawn((
            absolute_node(top_left.x, top_left.y, 100.0, 100.0),
            BackgroundColor(ORANGE),
            DraggableSquare { top_left },
            Name::new("square"),
        ))
        .observe(move_square_with_drag);
}

fn move_square_with_drag(
    drag: On<PointerDrag>,
    mut squares: Query<(&mut DraggableSquare, &mut Node)>,
) -> Result {
    let (mut square, mut node) = squares.get_mut(drag.event_target())?;
    square.top_left += drag.delta;
    node.left = Val::Px(square.top_left.x);
    node.top = Val::Px(square.top_left.y);
    Ok(())
}

fn spawn_text_line(commands: &mut Commands, top: f32, marker: impl Component) {
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(TEXT_SIZE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(40.0),
            top: Val::Px(top),
            ..default()
        },
        marker,
    ));
}

fn spawn_typed_text_line(mut commands: Commands) {
    spawn_text_line(&mut commands, 500.0, TypedTextLine);
}

fn spawn_status_text_line(mut commands: Commands) {
    spawn_text_line(&mut commands, 540.0, StatusTextLine);
}

fn collect_typed_text(mut keyboard: MessageReader<KeyboardInput>, mut typed: ResMut<TypedText>) {
    for input in keyboard.read() {
        if input.state == ButtonState::Pressed
            && let Some(text) = &input.text
        {
            typed.0.push_str(text.as_str());
        }
    }
}

fn accumulate_scroll(mut wheel: MessageReader<MouseWheel>, mut totals: ResMut<InputTotals>) {
    totals.scroll_y += wheel.read().map(|event| event.y).sum::<f32>();
}

fn accumulate_pinch(mut pinch: MessageReader<PinchGesture>, mut totals: ResMut<InputTotals>) {
    totals.pinch += pinch.read().map(|event| event.0).sum::<f32>();
}

fn accumulate_rotation(
    mut rotation: MessageReader<RotationGesture>,
    mut totals: ResMut<InputTotals>,
) {
    totals.rotation += rotation.read().map(|event| event.0).sum::<f32>();
}

fn accumulate_pan(mut pan: MessageReader<PanGesture>, mut totals: ResMut<InputTotals>) {
    totals.pan += pan.read().map(|event| event.0).sum::<Vec2>();
}

fn count_double_taps(mut taps: MessageReader<DoubleTapGesture>, mut totals: ResMut<InputTotals>) {
    totals.double_taps += u32::try_from(taps.read().count()).unwrap_or(u32::MAX);
}

fn show_typed_text(typed: Res<TypedText>, mut line: Single<&mut Text, With<TypedTextLine>>) {
    line.0.clone_from(&typed.0);
}

fn show_status(
    keys: Res<ButtonInput<KeyCode>>,
    totals: Res<InputTotals>,
    touches: Res<Touches>,
    window: Single<&Window, With<PrimaryWindow>>,
    seed: Res<DriverSeed>,
    virtual_time: Res<Time<Virtual>>,
    mut line: Single<&mut Text, With<StatusTextLine>>,
) {
    let held = keys
        .get_pressed()
        .map(|key| format!("{key:?}"))
        .collect::<Vec<_>>()
        .join(",");
    line.0 = format!(
        "held: [{held}] scroll_y: {} pinch: {} rotation: {} pan: {} double_taps: {} touches: {} cursor: {:?} seed: {:?} paused: {} virtual_elapsed: {:.2}",
        totals.scroll_y,
        totals.pinch,
        totals.rotation,
        totals.pan,
        totals.double_taps,
        touches.iter().count(),
        window.cursor_position(),
        seed.0,
        virtual_time.is_paused(),
        virtual_time.elapsed_secs(),
    );
}

fn button_is_green(
    In(_arguments): In<Value>,
    button: Single<&ToggleButton>,
) -> Result<Value, String> {
    Ok(Value::Bool(button.is_green))
}

fn square_top_left(
    In(_arguments): In<Value>,
    square: Single<&DraggableSquare>,
) -> Result<Value, String> {
    Ok(json!([square.top_left.x, square.top_left.y]))
}

/// Target `{"target":"node_center","name":"square"}`: the logical center of the named UI node,
/// read from the layout as it is now, so it follows the node after it moves.
fn node_center(
    In(arguments): In<Value>,
    nodes: Query<(&Name, &UiGlobalTransform, &ComputedNode)>,
) -> Result<Vec2, String> {
    let name = arguments
        .get("name")
        .and_then(Value::as_str)
        .ok_or("node_center needs a \"name\" string")?;
    let (_, transform, computed) = nodes
        .iter()
        .find(|(node_name, _, _)| node_name.as_str() == name)
        .ok_or_else(|| format!("no UI node named {name:?}"))?;
    Ok(transform.translation * computed.inverse_scale_factor())
}

fn typed_text_method(In(_parameters): In<Option<Value>>, typed: Res<TypedText>) -> BrpResult {
    Ok(Value::String(typed.0.clone()))
}

fn save_demo_state(
    In(path): In<PathBuf>,
    square: Single<&DraggableSquare>,
    button: Single<&ToggleButton>,
) -> Result<(), String> {
    let text = format!(
        "{} {} {}\n",
        square.top_left.x, square.top_left.y, button.is_green
    );
    fs::write(&path, text).map_err(|problem| format!("cannot write {}: {problem}", path.display()))
}

fn parse_demo_state(text: &str) -> Result<(Vec2, bool), String> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    let [x, y, is_green] = fields.as_slice() else {
        return Err(format!("expected 3 fields, found {}", fields.len()));
    };
    let parse_coordinate = |field: &str| {
        field
            .parse::<f32>()
            .map_err(|problem| format!("bad coordinate {field:?}: {problem}"))
    };
    let is_green = is_green
        .parse::<bool>()
        .map_err(|problem| format!("bad toggle {is_green:?}: {problem}"))?;
    Ok((
        Vec2::new(parse_coordinate(x)?, parse_coordinate(y)?),
        is_green,
    ))
}

fn load_demo_state(
    In(path): In<PathBuf>,
    mut square: Single<(&mut DraggableSquare, &mut Node)>,
    mut button: Single<(&mut ToggleButton, &mut BackgroundColor)>,
) -> Result<(), String> {
    let text = fs::read_to_string(&path)
        .map_err(|problem| format!("cannot read {}: {problem}", path.display()))?;
    let (top_left, is_green) = parse_demo_state(&text)?;
    let (square_state, square_node) = &mut *square;
    square_state.top_left = top_left;
    square_node.left = Val::Px(top_left.x);
    square_node.top = Val::Px(top_left.y);
    let (button_state, button_background) = &mut *button;
    button_state.is_green = is_green;
    button_background.0 = if is_green { BRIGHT_GREEN } else { DARK_BLUE };
    Ok(())
}
