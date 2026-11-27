//! The built-in `hovered_entities` check: which entities a picking pointer hovers.

use bevy::{
    ecs::entity::EntityHashMap,
    picking::{backend::HitData, hover::HoverMap, pointer::PointerId},
    prelude::*,
};
use serde_json::{Map, Value, json};

use crate::checks::{DriverChecks, register_driver_check};

const CHECK_NAME: &str = "hovered_entities";

const ACCEPTED_ARGUMENTS: &str = "accepted arguments: none, {\"pointer\":\"mouse\"}, or \
     {\"pointer\":\"touch\",\"id\":<unsigned integer>}";

const NO_PICKING_MESSAGE: &str = "hovered_entities needs bevy_picking: no HoverMap resource (add \
     DefaultPlugins or PickingPlugin)";

/// Registers the `hovered_entities` check. A game check with the same name wins, whether the
/// game registers it before or after this plugin is added.
pub struct DriverHoverCheckPlugin;

impl Plugin for DriverHoverCheckPlugin {
    fn build(&self, app: &mut App) {
        let game_check_registered = app
            .world()
            .get_resource::<DriverChecks>()
            .is_some_and(|checks| checks.get(CHECK_NAME).is_some());
        if !game_check_registered {
            register_driver_check(app, CHECK_NAME.to_owned(), hovered_entities);
        }
    }
}

fn bad_arguments(problem: &str) -> String {
    format!("bad hovered_entities arguments: {problem}; {ACCEPTED_ARGUMENTS}")
}

fn parse_pointer(arguments: &Value) -> Result<PointerId, String> {
    match arguments {
        Value::Null => Ok(PointerId::Mouse),
        Value::Object(fields) => parse_pointer_fields(fields),
        _ => Err(bad_arguments("arguments must be an object")),
    }
}

fn parse_pointer_fields(fields: &Map<String, Value>) -> Result<PointerId, String> {
    if let Some(key) = fields
        .keys()
        .find(|key| !matches!(key.as_str(), "pointer" | "id"))
    {
        return Err(bad_arguments(&format!("unknown key {key:?}")));
    }
    let kind = fields.get("pointer").and_then(Value::as_str);
    match (kind, fields.get("id")) {
        (Some("mouse"), None) => Ok(PointerId::Mouse),
        (Some("mouse"), Some(_)) => Err(bad_arguments("the mouse takes no id")),
        (Some("touch"), Some(id)) => id
            .as_u64()
            .map(PointerId::Touch)
            .ok_or_else(|| bad_arguments("a touch id must be an unsigned integer")),
        (Some("touch"), None) => Err(bad_arguments("a touch pointer needs an id")),
        (Some(other), _) => Err(bad_arguments(&format!("unknown pointer kind {other:?}"))),
        (None, _) => Err(bad_arguments("\"pointer\" must be a string")),
    }
}

fn describe_pointer(pointer: PointerId) -> String {
    match pointer {
        PointerId::Mouse => "mouse".to_owned(),
        PointerId::Touch(id) => format!("touch {id}"),
        PointerId::Custom(uuid) => format!("custom {uuid}"),
    }
}

fn unknown_pointer_message(requested: PointerId, existing: &Query<&PointerId>) -> String {
    let mut descriptions: Vec<String> = existing.iter().copied().map(describe_pointer).collect();
    descriptions.sort();
    let listed = if descriptions.is_empty() {
        "none".to_owned()
    } else {
        descriptions.join(", ")
    };
    format!(
        "no pointer {}; pointers: {listed}",
        describe_pointer(requested)
    )
}

/// What labels an entity: its `Name`, and whether it is a window.
type LabelQuery<'world, 'state> = Query<'world, 'state, (Option<&'static Name>, Has<Window>)>;

const WINDOW_LABEL: &str = "window";

// bevy_picking counts the window as hovered wherever nothing else is, and windows rarely have a
// `Name`, so without this label empty space would show an entity ID that changes between runs.
fn entity_label(entity: Entity, labels: &LabelQuery) -> String {
    match labels.get(entity) {
        Ok((Some(name), _)) => name.as_str().to_owned(),
        Ok((None, true)) => WINDOW_LABEL.to_owned(),
        _ => entity.to_string(),
    }
}

fn nearest_first_labels(hits: &EntityHashMap<HitData>, labels: &LabelQuery) -> Vec<String> {
    let mut labeled: Vec<(f32, String)> = hits
        .iter()
        .map(|(entity, hit)| (hit.depth, entity_label(*entity, labels)))
        .collect();
    labeled.sort_by(|(left_depth, left_label), (right_depth, right_label)| {
        left_depth
            .total_cmp(right_depth)
            .then_with(|| left_label.cmp(right_label))
    });
    labeled.into_iter().map(|(_, label)| label).collect()
}

fn hovered_entities(
    In(arguments): In<Value>,
    hover_map: Option<Res<HoverMap>>,
    pointers: Query<&PointerId>,
    labels: LabelQuery,
) -> Result<Value, String> {
    let requested = parse_pointer(&arguments)?;
    let hover_map = hover_map.ok_or(NO_PICKING_MESSAGE)?;
    if !pointers.iter().any(|pointer| *pointer == requested) {
        return Err(unknown_pointer_message(requested, &pointers));
    }
    let hovered = hover_map
        .get(&requested)
        .map(|hits| nearest_first_labels(hits, &labels))
        .unwrap_or_default();
    Ok(json!(hovered))
}

#[cfg(test)]
mod tests {
    use bevy::picking::pointer::PointerMap;

    use super::*;
    use crate::{
        InputDriverAppExt,
        checks::{DriverChecks, run_check},
    };

    fn world_with_check() -> World {
        let mut world = World::new();
        world.init_resource::<PointerMap>();
        world.insert_resource(HoverMap::default());
        let system = world.register_system(hovered_entities);
        let mut checks = DriverChecks::default();
        checks.insert(CHECK_NAME.to_owned(), system);
        world.insert_resource(checks);
        world.spawn(PointerId::Mouse);
        world
    }

    fn hover(world: &mut World, pointer: PointerId, hits: &[(Entity, f32)]) {
        let camera = world.spawn_empty().id();
        let mut map = EntityHashMap::default();
        for (entity, depth) in hits {
            map.insert(*entity, HitData::new(camera, *depth, None, None));
        }
        world.resource_mut::<HoverMap>().0.insert(pointer, map);
    }

    fn run(world: &mut World, arguments: Value) -> Result<Value, String> {
        run_check(world, CHECK_NAME, arguments).map_err(|problem| problem.to_string())
    }

    #[test]
    fn null_arguments_mean_the_mouse_and_sort_nearest_first() {
        let mut world = world_with_check();
        let far = world.spawn(Name::new("far")).id();
        let near = world.spawn(Name::new("near")).id();
        hover(&mut world, PointerId::Mouse, &[(far, 2.0), (near, 0.5)]);
        assert_eq!(run(&mut world, Value::Null), Ok(json!(["near", "far"])));
        assert_eq!(
            run(&mut world, json!({"pointer": "mouse"})),
            Ok(json!(["near", "far"]))
        );
    }

    #[test]
    fn touch_id_selects_its_own_pointer() {
        let mut world = world_with_check();
        world.spawn(PointerId::Touch(2));
        let grid = world.spawn(Name::new("Grid layer")).id();
        let other = world.spawn(Name::new("other")).id();
        hover(&mut world, PointerId::Touch(2), &[(grid, 0.0)]);
        hover(&mut world, PointerId::Mouse, &[(other, 0.0)]);
        assert_eq!(
            run(&mut world, json!({"pointer": "touch", "id": 2})),
            Ok(json!(["Grid layer"]))
        );
    }

    #[test]
    fn equal_depths_sort_by_name() {
        let mut world = world_with_check();
        let second = world.spawn(Name::new("b")).id();
        let first = world.spawn(Name::new("a")).id();
        hover(&mut world, PointerId::Mouse, &[(second, 1.0), (first, 1.0)]);
        assert_eq!(run(&mut world, Value::Null), Ok(json!(["a", "b"])));
    }

    #[test]
    fn an_unnamed_window_shows_as_window_and_a_named_one_keeps_its_name() {
        let mut world = world_with_check();
        let unnamed_window = world.spawn(Window::default()).id();
        let named_window = world.spawn((Window::default(), Name::new("main"))).id();
        hover(
            &mut world,
            PointerId::Mouse,
            &[(unnamed_window, 1.0), (named_window, 2.0)],
        );
        assert_eq!(run(&mut world, Value::Null), Ok(json!(["window", "main"])));
    }

    #[test]
    fn an_unnamed_entity_shows_its_display_text() {
        let mut world = world_with_check();
        let unnamed = world.spawn_empty().id();
        hover(&mut world, PointerId::Mouse, &[(unnamed, 0.0)]);
        assert_eq!(
            run(&mut world, Value::Null),
            Ok(json!([unnamed.to_string()]))
        );
    }

    #[test]
    fn a_pointer_that_hovers_nothing_gives_an_empty_list() {
        let mut world = world_with_check();
        assert_eq!(run(&mut world, Value::Null), Ok(json!([])));
        hover(&mut world, PointerId::Mouse, &[]);
        assert_eq!(run(&mut world, Value::Null), Ok(json!([])));
    }

    #[test]
    fn an_unknown_pointer_lists_the_existing_ones() {
        let mut world = world_with_check();
        world.spawn(PointerId::Touch(2));
        assert_eq!(
            run(&mut world, json!({"pointer": "touch", "id": 5})),
            Err("no pointer touch 5; pointers: mouse, touch 2".to_owned())
        );
    }

    #[test]
    fn an_unknown_pointer_with_no_pointers_says_none() {
        let mut world = World::new();
        world.init_resource::<PointerMap>();
        world.insert_resource(HoverMap::default());
        let system = world.register_system(hovered_entities);
        let mut checks = DriverChecks::default();
        checks.insert(CHECK_NAME.to_owned(), system);
        world.insert_resource(checks);
        assert_eq!(
            run(&mut world, Value::Null),
            Err("no pointer mouse; pointers: none".to_owned())
        );
    }

    #[test]
    fn a_missing_hover_map_is_an_error() {
        let mut world = world_with_check();
        world.remove_resource::<HoverMap>();
        assert_eq!(
            run(&mut world, Value::Null),
            Err(NO_PICKING_MESSAGE.to_owned())
        );
    }

    #[test]
    fn bad_argument_shapes_name_the_accepted_ones() {
        let mut world = world_with_check();
        let bad_shapes = [
            json!("mouse"),
            json!([1]),
            json!({}),
            json!({"pointer": "pen"}),
            json!({"pointer": 3}),
            json!({"pointer": "touch"}),
            json!({"pointer": "touch", "id": "2"}),
            json!({"pointer": "touch", "id": -1}),
            json!({"pointer": "touch", "id": 1.5}),
            json!({"pointer": "mouse", "id": 0}),
            json!({"pointer": "mouse", "extra": true}),
        ];
        for arguments in bad_shapes {
            let message = run(&mut world, arguments.clone()).unwrap_err();
            assert!(
                message.contains(ACCEPTED_ARGUMENTS),
                "{arguments}: {message}"
            );
        }
    }

    fn game_hover_check(In(_arguments): In<Value>) -> Result<Value, String> {
        Ok(json!("from the game"))
    }

    #[test]
    fn a_game_check_registered_before_the_plugin_wins() {
        let mut app = App::new();
        app.add_driver_check(CHECK_NAME, game_hover_check);
        app.add_plugins(DriverHoverCheckPlugin);
        assert_eq!(
            run(app.world_mut(), Value::Null),
            Ok(json!("from the game"))
        );
    }

    #[test]
    fn a_game_check_registered_after_the_plugin_wins() {
        let mut app = App::new();
        app.add_plugins(DriverHoverCheckPlugin);
        app.add_driver_check(CHECK_NAME, game_hover_check);
        assert_eq!(
            run(app.world_mut(), Value::Null),
            Ok(json!("from the game"))
        );
    }
}
