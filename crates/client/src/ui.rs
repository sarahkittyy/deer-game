//! The title screen ("deer game", a username box, the doe/stag switch and play) and the small
//! dot in the middle of the screen while playing.

use crate::{
    AppState, GameWorld,
    audio::Sfx,
    deer::DeerAssets,
    env::MainCamera,
    net::{Net, NetEvent, Profile},
    player::{self, Look},
    wrap::Focus,
};
use bevy::{
    input_focus::{AutoFocus, FocusCause, InputFocus},
    picking::events::{PointerClick, PointerOut, PointerOver},
    prelude::*,
    text::{EditableText, TextCursorStyle},
    ui_widgets::TextInput,
};
use deer_shared::protocol::{MAX_NAME_LEN, clean_name};

/// How long play waits for the server before going on alone.
const CONNECT_WAIT: f32 = 3.0;

#[derive(Resource)]
pub struct UiFont(pub Handle<Font>);

impl FromWorld for UiFont {
    fn from_world(world: &mut World) -> Self {
        UiFont(world.resource::<AssetServer>().load("fonts/ui.ttf"))
    }
}

#[derive(Resource)]
struct TitleFont(Handle<Font>);

impl FromWorld for TitleFont {
    fn from_world(world: &mut World) -> Self {
        TitleFont(world.resource::<AssetServer>().load("fonts/title.ttf"))
    }
}

#[derive(Component)]
pub struct TitleRoot;

#[derive(Component)]
struct NameInput;

#[derive(Component)]
struct Placeholder;

#[derive(Component)]
struct SwitchKnob;

#[derive(Component)]
struct PlayButton;

#[derive(Component)]
pub struct Crosshair;

/// What the crosshair turns into when pointing at something to eat or drink.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorIcon {
    Berry,
    Water,
}

/// An empty full-window UI node: its computed size is the UI's view of the screen.
#[derive(Component)]
pub struct UiRoot;

/// Whether the switch is on stag.
#[derive(Resource, Default)]
struct Choice {
    stag: bool,
}

/// Set while play has been pressed and the game waits for the server's welcome.
#[derive(Resource)]
struct Joining {
    waited: f32,
}

// Deer fur, for the title's letters: rich brown, tan, dark brown, cream.
const FUR: [Color; 4] = [
    Color::srgb(0.62, 0.33, 0.17),
    Color::srgb(0.85, 0.62, 0.40),
    Color::srgb(0.42, 0.24, 0.14),
    Color::srgb(0.93, 0.87, 0.76),
];
const CREAM: Color = Color::srgb(0.93, 0.87, 0.76);

pub fn plugin(app: &mut App) {
    // Fonts are needed by the title, which spawns on the first state transition before Startup.
    app.add_systems(Startup, |mut commands: Commands| {
        commands.spawn((
            UiRoot,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            Pickable::IGNORE,
        ));
    });
    app.init_resource::<Choice>()
        .init_resource::<UiFont>()
        .init_resource::<TitleFont>()
        .add_systems(OnEnter(AppState::Title), spawn_title)
        .add_systems(OnExit(AppState::Title), despawn_title)
        .add_systems(OnEnter(AppState::Playing), spawn_crosshair)
        .add_systems(
            Update,
            (
                orbit_camera,
                placeholder,
                keep_name_focus.run_if(not(resource_exists::<Joining>)),
                switch_visual,
                keyboard_play,
                join.run_if(resource_exists::<Joining>),
            )
                .run_if(in_state(AppState::Title)),
        );
}

fn spawn_title(
    mut commands: Commands,
    title_font: Res<TitleFont>,
    ui_font: Res<UiFont>,
    assets: Res<AssetServer>,
    mut choice: ResMut<Choice>,
) {
    let saved = load_profile();
    if let Some(p) = &saved {
        choice.stag = p.stag;
    }
    let antlers: Handle<Image> = assets.load("ui/antlers.png");

    let root = commands
        .spawn((
            TitleRoot,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(22),
                ..default()
            },
            // A soft darkening behind the middle so the words read over the forest.
            BackgroundGradient::from(RadialGradient {
                stops: vec![
                    ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.45), percent(0)),
                    ColorStop::new(Color::srgba(0.0, 0.0, 0.0, 0.0), percent(70)),
                ],
                ..default()
            }),
        ))
        .id();

    // "deer game", one fur colour per letter.
    let title = commands
        .spawn((
            Text::default(),
            TextFont {
                font: title_font.0.clone().into(),
                font_size: FontSize::Px(110.0),
                ..default()
            },
            TextShadow {
                offset: Vec2::new(5.0, 5.0),
                color: Color::srgba(0.1, 0.05, 0.02, 0.85),
            },
            Node {
                margin: UiRect::bottom(px(18)),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    for (i, ch) in "deer game".chars().enumerate() {
        commands.spawn((
            TextSpan::new(ch.to_string()),
            TextFont {
                font: title_font.0.clone().into(),
                font_size: FontSize::Px(110.0),
                ..default()
            },
            TextColor(FUR[i % FUR.len()]),
            ChildOf(title),
        ));
    }

    // Username box, with a placeholder shown while it is empty.
    let field = commands
        .spawn((
            Node {
                width: px(300),
                height: px(46),
                border: px(3).all(),
                padding: UiRect::axes(px(12), px(8)),
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(CREAM.with_alpha(0.85)),
            BackgroundColor(Color::srgba(0.08, 0.05, 0.03, 0.6)),
            ChildOf(root),
        ))
        .id();
    commands.spawn((
        Placeholder,
        Text::new("username"),
        TextFont {
            font: ui_font.0.clone().into(),
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(CREAM.with_alpha(0.4)),
        Node {
            position_type: PositionType::Absolute,
            left: px(12),
            ..default()
        },
        ChildOf(field),
    ));
    commands.spawn((
        NameInput,
        TextInput,
        EditableText {
            max_characters: Some(MAX_NAME_LEN),
            allow_newlines: false,
            ..EditableText::new(saved.as_ref().map(|p| p.name.as_str()).unwrap_or(""))
        },
        TextLayout::no_wrap(),
        TextFont {
            font: ui_font.0.clone().into(),
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(CREAM),
        TextCursorStyle::default(),
        AutoFocus,
        Node {
            width: percent(100),
            ..default()
        },
        ChildOf(field),
    ));

    // Doe / stag switch: crossed-out antlers, a sliding knob, antlers.
    let switch_row = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(14),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let doe_icon = commands
        .spawn((
            ImageNode::new(antlers.clone()).with_color(CREAM),
            Node {
                width: px(40),
                height: px(40),
                ..default()
            },
            ChildOf(switch_row),
        ))
        .observe(|_: On<PointerClick>, mut c: ResMut<Choice>| c.stag = false)
        .id();
    // The line through the antlers.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: px(52),
            height: px(4),
            left: px(-6),
            top: px(18),
            ..default()
        },
        BackgroundColor(Color::srgb(0.75, 0.25, 0.18)),
        UiTransform::from_rotation(Rot2::degrees(-40.0)),
        ChildOf(doe_icon),
    ));
    let track = commands
        .spawn((
            Node {
                width: px(64),
                height: px(32),
                border: px(3).all(),
                border_radius: BorderRadius::all(px(16)),
                padding: px(3).all(),
                ..default()
            },
            BorderColor::all(CREAM.with_alpha(0.85)),
            BackgroundColor(Color::srgba(0.08, 0.05, 0.03, 0.6)),
            ChildOf(switch_row),
        ))
        .observe(
            |_: On<PointerClick>, mut c: ResMut<Choice>, mut commands: Commands| {
                c.stag = !c.stag;
                commands.trigger(Sfx::Click);
            },
        )
        .id();
    commands.spawn((
        SwitchKnob,
        Node {
            width: px(20),
            height: px(20),
            border_radius: BorderRadius::all(px(10)),
            ..default()
        },
        BackgroundColor(FUR[1]),
        ChildOf(track),
    ));
    commands
        .spawn((
            ImageNode::new(antlers).with_color(CREAM),
            Node {
                width: px(40),
                height: px(40),
                ..default()
            },
            ChildOf(switch_row),
        ))
        .observe(|_: On<PointerClick>, mut c: ResMut<Choice>| c.stag = true);

    // [play]
    commands
        .spawn((
            PlayButton,
            Node {
                padding: UiRect::axes(px(20), px(8)),
                margin: UiRect::top(px(8)),
                ..default()
            },
            ChildOf(root),
            children![(
                Text::new("[play]"),
                TextFont {
                    font: ui_font.0.clone().into(),
                    font_size: FontSize::Px(34.0),
                    ..default()
                },
                TextColor(CREAM),
                TextShadow {
                    offset: Vec2::new(3.0, 3.0),
                    color: Color::srgba(0.1, 0.05, 0.02, 0.85),
                },
            )],
        ))
        .observe(|_: On<PointerClick>, mut commands: Commands| {
            commands.run_system_cached(start_join)
        })
        .observe(
            |over: On<PointerOver>,
             mut colors: Query<&mut TextColor>,
             children: Query<&Children>| {
                for c in children.iter_descendants(over.entity) {
                    if let Ok(mut t) = colors.get_mut(c) {
                        t.0 = FUR[1];
                    }
                }
            },
        )
        .observe(
            |out: On<PointerOut>, mut colors: Query<&mut TextColor>, children: Query<&Children>| {
                for c in children.iter_descendants(out.entity) {
                    if let Ok(mut t) = colors.get_mut(c) {
                        t.0 = CREAM;
                    }
                }
            },
        );
}

fn despawn_title(mut commands: Commands, roots: Query<Entity, With<TitleRoot>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<Joining>();
}

fn placeholder(
    input: Query<&EditableText, With<NameInput>>,
    mut ph: Query<&mut Visibility, With<Placeholder>>,
) {
    let (Ok(input), Ok(mut vis)) = (input.single(), ph.single_mut()) else {
        return;
    };
    *vis = if input.value().to_string().is_empty() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
}

/// The name box is the only thing to type into, so it keeps the keyboard. (In a browser the
/// focus it gets on spawn is lost while the page is still taking focus.)
fn keep_name_focus(mut focus: ResMut<InputFocus>, input: Query<Entity, With<NameInput>>) {
    if let Ok(e) = input.single()
        && focus.get() != Some(e)
    {
        focus.set(e, FocusCause::Navigated);
    }
}

fn switch_visual(choice: Res<Choice>, mut knob: Query<&mut Node, With<SwitchKnob>>) {
    if let Ok(mut n) = knob.single_mut() {
        n.margin.left = if choice.stag { px(32) } else { px(0) };
    }
}

/// Presses play, as the button does.
pub fn press_play(commands: &mut Commands) {
    commands.run_system_cached(start_join);
}

fn keyboard_play(keys: Res<ButtonInput<KeyCode>>, mut commands: Commands) {
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        commands.run_system_cached(start_join);
    }
}

fn start_join(
    mut commands: Commands,
    joining: Option<Res<Joining>>,
    input: Query<&EditableText, With<NameInput>>,
    choice: Res<Choice>,
    mut net: ResMut<Net>,
    mut focus: ResMut<InputFocus>,
) {
    if joining.is_some() {
        return;
    }
    let name = input
        .single()
        .map(|t| clean_name(&t.value().to_string()))
        .unwrap_or_else(|_| "deer".into());
    let name = crate::debug::name_override().unwrap_or(name);
    let profile = Profile {
        name,
        stag: choice.stag || std::env::var("DEER_STAG").is_ok(),
    };
    save_profile(&profile);
    commands.insert_resource(profile);
    commands.insert_resource(Joining { waited: 0.0 });
    commands.trigger(Sfx::Click);
    focus.clear();
    net.connect();
}

/// Waits for the server's welcome (or gives up and plays alone), then puts the deer down.
#[allow(clippy::too_many_arguments)]
fn join(
    mut commands: Commands,
    time: Res<Time>,
    mut joining: ResMut<Joining>,
    mut events: MessageReader<NetEvent>,
    profile: Res<Profile>,
    assets: Res<DeerAssets>,
    world: Res<GameWorld>,
    mut look: ResMut<Look>,
    mut next: ResMut<NextState<AppState>>,
) {
    joining.waited += time.delta_secs();
    let mut spawn = None;
    for ev in events.read() {
        if let NetEvent::Welcome { spawn: s, .. } = ev {
            spawn = Some(*s);
        }
    }
    if spawn.is_none() && joining.waited > CONNECT_WAIT {
        let n = (time.elapsed_secs_f64() * 1000.0) as u64;
        spawn = Some(world.0.spawn_point(n));
    }
    if let Some(at) = spawn {
        let at = crate::debug::spawn_override().unwrap_or(at);
        player::spawn_local(
            &mut commands,
            &assets,
            &world.0,
            profile.stag,
            at,
            &mut look,
        );
        next.set(AppState::Playing);
    }
}

/// A slow circle over a pond while the title is up.
fn orbit_camera(
    time: Res<Time>,
    world: Res<GameWorld>,
    mut focus: ResMut<Focus>,
    mut cam: Single<&mut Transform, With<MainCamera>>,
) {
    if std::env::var("DEER_CAM").is_ok() {
        return;
    }
    // Drift around inside the pond's shore, looking across the water at the trees.
    let center = Vec3::new(86.5, 0.0, 141.0);
    let a = time.elapsed_secs() * 0.02 + 2.2;
    let mut eye = center + Vec3::new(a.cos() * 13.0, 0.0, a.sin() * 13.0);
    eye.y = world.0.height(eye.x, eye.z).max(0.0) + 3.2;
    let b = a + 2.3;
    let target = center + Vec3::new(b.cos() * 30.0, 3.5, b.sin() * 30.0);
    **cam = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    focus.0 = eye;
}

fn spawn_crosshair(mut commands: Commands, assets: Res<AssetServer>) {
    for (icon, path, tint) in [
        (
            CursorIcon::Berry,
            "ui/berry.png",
            Color::srgb(0.95, 0.45, 0.55),
        ),
        (
            CursorIcon::Water,
            "ui/water.png",
            Color::srgb(0.7, 0.88, 1.0),
        ),
    ] {
        commands.spawn((
            icon,
            crate::GameEntity,
            ImageNode::new(assets.load(path)).with_color(tint),
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: percent(50),
                width: px(24),
                height: px(24),
                margin: UiRect {
                    left: px(-12),
                    top: px(-12),
                    ..default()
                },
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ));
    }
    commands.spawn((
        Crosshair,
        crate::GameEntity,
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            width: px(4),
            height: px(4),
            margin: UiRect {
                left: px(-2),
                top: px(-2),
                ..default()
            },
            border_radius: BorderRadius::all(px(2)),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 0.97, 0.9, 0.35)),
    ));
}

// The last name and choice, so the title remembers them.

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

fn load_profile() -> Option<Profile> {
    #[cfg(target_arch = "wasm32")]
    let raw = storage()?.get_item("deer-profile").ok()??;
    #[cfg(not(target_arch = "wasm32"))]
    let raw = std::fs::read_to_string(profile_path()?).ok()?;
    let (stag, name) = raw.split_once('\n')?;
    Some(Profile {
        name: clean_name(name),
        stag: stag == "stag",
    })
}

fn save_profile(p: &Profile) {
    let raw = format!("{}\n{}", if p.stag { "stag" } else { "doe" }, p.name);
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = storage() {
        let _ = s.set_item("deer-profile", &raw);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = profile_path() {
        let _ = std::fs::write(path, raw);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn profile_path() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("HOME")?).join(".deer-game-profile"))
}
