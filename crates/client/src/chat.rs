//! Chat: T opens a small box under the chat icon; Enter says the line (Esc drops it), and it
//! floats over the speaker's head for a few seconds, for everyone nearby to read. Lines said in
//! a row stack up, each in its own bubble, newest at the bottom with the tail.
//!
//! `DEER_SAY=t,line` says `line` at `t` seconds (or `t,a|b` says a, then b a second later), for
//! testing without a keyboard.

use crate::{
    AppState, GameEntity,
    deer::Mouth,
    ghosts::Ghost,
    net::{Net, NetEvent},
    player::LocalDeer,
    ui::UiFont,
    wrap::{Focus, Wrapped},
};
use bevy::{
    input_focus::{FocusCause, InputFocus},
    prelude::*,
    text::{EditableText, TextCursorStyle},
    ui_widgets::TextInput,
};
use deer_shared::protocol::{ClientMsg, MAX_CHAT_LEN, clean_chat};

/// Lines stay up this long, more for longer ones.
const SHOW_MIN: f32 = 5.0;
const SHOW_MAX: f32 = 10.0;
const SHOW_PER_CHAR: f32 = 0.08;
const FADE: f32 = 0.6;
/// At most this many lines over one deer; the oldest goes first.
const MAX_LINES: usize = 4;
/// Lines fade out between these distances (a little further than name tags).
const NEAR: f32 = 30.0;
const FAR: f32 = 50.0;
const CREAM: Color = Color::srgb(0.93, 0.87, 0.76);
/// Height of the bubble's tail, and the clear gap between stacked bubbles (pixels).
const TAIL: f32 = 8.0;
const GAP: f32 = 3.0;
/// How dark the bubbles are, at most: just enough to read the text over a bright sky.
const BUBBLE_ALPHA: f32 = 0.32;
/// How quickly the bubbles follow the head: slow enough to ride out a gallop's bobbing.
const FOLLOW: f32 = 4.0;

/// While the chat box is open the keys are typing, not walking.
#[derive(Resource, Default)]
pub struct ChatOpen(pub bool);

#[derive(Component)]
struct ChatBox;

#[derive(Component)]
struct ChatInput;

/// The bubbles over one deer: a column of lines with the tail under the newest.
#[derive(Component)]
struct Stack {
    deer: Entity,
    lines: Entity,
    tail: Entity,
    name: Option<Entity>,
    /// Where the head is in the deer's own space, smoothed.
    head: Option<Vec3>,
}

/// One line's bubble.
#[derive(Component)]
struct Line {
    until: f32,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<ChatOpen>()
        .add_systems(
            Update,
            (type_line, debug_say, heard, place_bubbles)
                .chain()
                // After the player's systems: they see the box open on the frame Esc or Enter
                // closes it (Esc must not also let the mouse go), and the deer and camera have
                // moved. After the name tags, which a bubble hides.
                .after(crate::player::PlayerSet)
                .after(crate::ghosts::place_tags)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(OnExit(AppState::Playing), |mut open: ResMut<ChatOpen>| {
            open.0 = false
        });
}

#[allow(clippy::too_many_arguments)]
fn type_line(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    font: Res<UiFont>,
    mut open: ResMut<ChatOpen>,
    mut focus: ResMut<InputFocus>,
    mut net: ResMut<Net>,
    boxes: Query<Entity, With<ChatBox>>,
    input: Query<&EditableText, With<ChatInput>>,
    me: Query<Entity, With<LocalDeer>>,
    stacks: Query<(Entity, &Stack)>,
    children: Query<&Children>,
) {
    if !open.0 {
        if keys.just_pressed(KeyCode::KeyT) {
            let input = spawn_box(&mut commands, &font);
            focus.set(input, FocusCause::Navigated);
            open.0 = true;
        }
        return;
    }
    let send = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    if !send && !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if send
        && let Ok(text) = input.single()
        && let Some(line) = clean_chat(&text.value().to_string())
        && let Ok(me) = me.single()
    {
        net.send(&ClientMsg::Chat(line.clone()));
        let said = Said {
            deer: me,
            name: None,
            line,
            now: time.elapsed_secs(),
        };
        say(&mut commands, &font, &stacks, &children, said);
    }
    for e in &boxes {
        commands.entity(e).despawn();
    }
    focus.clear();
    open.0 = false;
}

fn spawn_box(commands: &mut Commands, font: &UiFont) -> Entity {
    let frame = commands
        .spawn((
            ChatBox,
            GameEntity,
            Node {
                position_type: PositionType::Absolute,
                left: px(18),
                top: px(92),
                width: px(320),
                height: px(36),
                border: px(2).all(),
                padding: UiRect::axes(px(9), px(5)),
                align_items: AlignItems::Center,
                ..default()
            },
            BorderColor::all(CREAM.with_alpha(0.85)),
            BackgroundColor(Color::srgba(0.08, 0.05, 0.03, 0.6)),
        ))
        .id();
    commands
        .spawn((
            ChatInput,
            TextInput,
            EditableText {
                max_characters: Some(MAX_CHAT_LEN),
                allow_newlines: false,
                ..EditableText::new("")
            },
            TextLayout::no_wrap(),
            TextFont {
                font: font.0.clone().into(),
                font_size: FontSize::Px(16.0),
                ..default()
            },
            TextColor(CREAM),
            TextCursorStyle::default(),
            Node {
                width: percent(100),
                ..default()
            },
            ChildOf(frame),
        ))
        .id()
}

/// A line someone said. `name` heads the bubbles over other deer, whose name tag they hide.
struct Said {
    deer: Entity,
    name: Option<String>,
    line: String,
    now: f32,
}

/// Adds a line to the bottom of `deer`'s stack of bubbles (making the stack if there is none).
fn say(
    commands: &mut Commands,
    font: &UiFont,
    stacks: &Query<(Entity, &Stack)>,
    children: &Query<&Children>,
    said: Said,
) {
    let lines = match stacks.iter().find(|(_, s)| s.deer == said.deer) {
        Some((_, stack)) => {
            // Room for the new line: the oldest ones go.
            if let Ok(old) = children.get(stack.lines) {
                let extra = (old.len() + 1).saturating_sub(MAX_LINES);
                for e in old.iter().take(extra) {
                    commands.entity(e).despawn();
                }
            }
            stack.lines
        }
        None => spawn_stack(commands, font, said.deer, said.name),
    };
    let show = (SHOW_MIN + said.line.chars().count() as f32 * SHOW_PER_CHAR).min(SHOW_MAX);
    let bubble = commands
        .spawn((
            Line {
                until: said.now + show,
            },
            Node {
                max_width: px(250),
                padding: UiRect::axes(px(9), px(5)),
                border_radius: BorderRadius::all(px(7)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            ChildOf(lines),
        ))
        .id();
    commands.spawn((
        Text::new(said.line),
        TextFont {
            font: font.0.clone().into(),
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::NONE),
        TextLayout::justify(Justify::Center),
        ChildOf(bubble),
    ));
}

/// An empty stack over `deer`; returns the column the lines go in.
fn spawn_stack(
    commands: &mut Commands,
    font: &UiFont,
    deer: Entity,
    name: Option<String>,
) -> Entity {
    let root = commands
        .spawn((
            GameEntity,
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();
    let name = name.map(|name| {
        commands
            .spawn((
                Text::new(name),
                TextFont {
                    font: font.0.clone().into(),
                    font_size: FontSize::Px(12.0),
                    ..default()
                },
                TextColor(Color::NONE),
                Node {
                    margin: UiRect::bottom(px(GAP)),
                    ..default()
                },
                ChildOf(root),
            ))
            .id()
    });
    let lines = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(GAP),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    // The tail: the lower half of a square turned on its corner, clipped so the half that
    // would overlap the bubble (and darken it) is not drawn.
    let clip = commands
        .spawn((
            Node {
                width: px(TAIL * 2.0),
                height: px(TAIL),
                overflow: Overflow::clip(),
                justify_content: JustifyContent::Center,
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    let side = TAIL * std::f32::consts::SQRT_2;
    let tail = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: px(side),
                height: px(side),
                top: px(-side * 0.5),
                ..default()
            },
            UiTransform::from_rotation(Rot2::degrees(45.0)),
            BackgroundColor(Color::NONE),
            ChildOf(clip),
        ))
        .id();
    commands.entity(root).insert(Stack {
        deer,
        lines,
        tail,
        name,
        head: None,
    });
    lines
}

/// Lines from everyone else.
fn heard(
    mut commands: Commands,
    mut events: MessageReader<NetEvent>,
    time: Res<Time>,
    font: Res<UiFont>,
    roster: Res<crate::ghosts::Roster>,
    ghosts: Query<(Entity, &Ghost)>,
    stacks: Query<(Entity, &Stack)>,
    children: Query<&Children>,
) {
    for ev in events.read() {
        if let NetEvent::Chat { id, text } = ev
            && let Some((deer, _)) = ghosts.iter().find(|(_, g)| g.id == *id)
        {
            let said = Said {
                deer,
                name: roster.name(*id),
                line: text.clone(),
                now: time.elapsed_secs(),
            };
            say(&mut commands, &font, &stacks, &children, said);
        }
    }
}

fn debug_say(
    mut commands: Commands,
    time: Res<Time>,
    font: Res<UiFont>,
    mut net: ResMut<Net>,
    me: Query<Entity, With<LocalDeer>>,
    stacks: Query<(Entity, &Stack)>,
    children: Query<&Children>,
    mut said: Local<usize>,
) {
    let Some((t, lines)) = std::env::var("DEER_SAY")
        .ok()
        .and_then(|v| {
            v.split_once(',')
                .map(|(t, l)| (t.parse::<f32>().ok(), l.to_string()))
        })
        .and_then(|(t, l)| Some((t?, l)))
    else {
        return;
    };
    let now = time.elapsed_secs();
    let Some(line) = lines.split('|').nth(*said) else {
        return;
    };
    if now < t + *said as f32 {
        return;
    }
    *said += 1;
    if let (Some(line), Ok(me)) = (clean_chat(line), me.single()) {
        net.send(&ClientMsg::Chat(line.clone()));
        let said = Said {
            deer: me,
            name: None,
            line,
            now,
        };
        say(&mut commands, &font, &stacks, &children, said);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn place_bubbles(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<(&Camera, &Transform), With<crate::env::MainCamera>>,
    ui_size: Single<&ComputedNode, With<crate::ui::UiRoot>>,
    focus: Res<Focus>,
    deer: Query<(&Transform, Option<&Wrapped>, &Mouth, Option<&Ghost>)>,
    mut stacks: Query<(
        Entity,
        &mut Stack,
        &mut Node,
        &mut Visibility,
        &ComputedNode,
    )>,
    lines: Query<(&Line, &Children)>,
    children: Query<&Children>,
    mut backgrounds: Query<&mut BackgroundColor>,
    mut texts: Query<&mut TextColor>,
    mut tags: Query<&mut Visibility, Without<Stack>>,
) {
    let now = time.elapsed_secs();
    let follow = 1.0 - (-time.delta_secs() * FOLLOW).exp();
    let (cam, cam_t) = *camera;
    let cam_t = GlobalTransform::from(*cam_t);
    let screen_size = ui_size.size() * ui_size.inverse_scale_factor();
    for (e, mut stack, mut node, mut vis, computed) in &mut stacks {
        // The lines still up, oldest first; a stack with none left goes.
        let live: Vec<(Entity, f32)> = children
            .get(stack.lines)
            .map(|c| c.iter().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|l| lines.get(l).ok().map(|(line, _)| (l, line.until)))
            .collect();
        for &(l, until) in &live {
            if now >= until {
                commands.entity(l).despawn();
            }
        }
        let live: Vec<(Entity, f32)> = live.into_iter().filter(|(_, u)| now < *u).collect();
        let Ok((t, wrapped, mouth, ghost)) = deer.get(stack.deer) else {
            commands.entity(e).despawn();
            continue;
        };
        if live.is_empty() {
            commands.entity(e).despawn();
            continue;
        }
        // Over the top of the head: where the head sits on the body, smoothed so a gallop's
        // bobbing does not shake the text, carried by where the deer is this frame.
        let head = stack.head.get_or_insert(mouth.head_local);
        *head += (mouth.head_local - *head) * follow;
        let at = wrapped.map_or(t.translation, |w| crate::wrap::nearest(w.0, focus.0));
        let anchor = at + t.rotation * *head + Vec3::Y * 0.22;
        let dist = anchor.distance(cam_t.translation());
        let near = 1.0 - ((dist - NEAR) / (FAR - NEAR)).clamp(0.0, 1.0);
        let on_screen = cam
            .world_to_ndc(&cam_t, anchor)
            .filter(|n| n.z > 0.0 && n.z < 1.0)
            .map(|n| Vec2::new((n.x + 1.0) * 0.5, (1.0 - n.y) * 0.5) * screen_size);
        let Some(screen) = on_screen.filter(|_| near > 0.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let size = computed.size() * computed.inverse_scale_factor();
        node.left = Val::Px((screen.x - size.x * 0.5).round());
        node.top = Val::Px((screen.y - size.y).round());
        *vis = Visibility::Inherited;

        // Each line fades on its own; the tail goes with the newest, the name with the last.
        let fade = |until: f32| near * ((until - now) / FADE).clamp(0.0, 1.0);
        for &(l, until) in &live {
            let a = fade(until);
            if let Ok(mut bg) = backgrounds.get_mut(l) {
                bg.0 = Color::srgba(0.0, 0.0, 0.0, BUBBLE_ALPHA * a);
            }
            if let Ok((_, kids)) = lines.get(l) {
                for k in kids {
                    if let Ok(mut text) = texts.get_mut(*k) {
                        text.0 = Color::srgba(1.0, 1.0, 1.0, a);
                    }
                }
            }
        }
        let newest = live.last().map_or(0.0, |&(_, u)| fade(u));
        let longest = live.iter().map(|&(_, u)| fade(u)).fold(0.0, f32::max);
        if let Ok(mut bg) = backgrounds.get_mut(stack.tail) {
            bg.0 = Color::srgba(0.0, 0.0, 0.0, BUBBLE_ALPHA * newest);
        }
        if let Some(name) = stack.name
            && let Ok(mut text) = texts.get_mut(name)
        {
            text.0 = Color::srgba(1.0, 1.0, 1.0, 0.8 * longest);
        }
        // The bubbles say who is talking; their tag would sit right behind them.
        if let Some(g) = ghost
            && let Ok(mut tag) = tags.get_mut(g.tag)
        {
            *tag = Visibility::Hidden;
        }
    }
}
