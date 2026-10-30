//! The little row in the top-left corner while playing: the chat icon (press T) and the heart,
//! which berries and water fill and time slowly empties.

use crate::{AppState, GameEntity};
use bevy::{
    image::{ImageLoaderSettings, ImageSampler},
    prelude::*,
};

/// How much of the heart one berry fills.
pub const BERRY_HEART: f32 = 0.15;
/// How much of the heart one full drink of water fills: two and a half berries.
pub const DRINK_HEART: f32 = BERRY_HEART * 2.5;
/// How fast the heart empties, per second: a berry a minute keeps it level.
const DRAIN: f32 = BERRY_HEART / 60.0;
/// The heart at the start of each visit.
const START: f32 = 0.5;
/// From here up the heart looks completely full: a berry's worth, a minute's grace before it
/// visibly starts to empty.
const LOOKS_FULL: f32 = 1.0 - BERRY_HEART;
/// The chat icon and the heart are both 38 x 34 pixel art (outline included), shown at one and a
/// half times that
/// (three screen pixels per art pixel on a high-density screen), pixels kept sharp.
const ICON_SIZE: Vec2 = Vec2::new(57.0, 51.0);

/// How full the heart is, 0 to 1.
#[derive(Resource)]
pub struct Heart(pub f32);

impl Default for Heart {
    fn default() -> Self {
        Heart(START)
    }
}

impl Heart {
    pub fn eat_berry(&mut self) {
        self.0 = (self.0 + BERRY_HEART).min(1.0);
    }

    pub fn drink(&mut self, amount: f32) {
        self.0 = (self.0 + amount).min(1.0);
    }
}

/// The clipping box that shows the red part of the heart, as tall as the heart is full.
#[derive(Component)]
struct HeartFill;

pub fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Playing), spawn)
        .add_systems(Update, drain_and_show.run_if(in_state(AppState::Playing)));
}

/// Pixel art: sampled without blurring.
fn load_pixels(assets: &AssetServer, path: &'static str) -> Handle<Image> {
    assets
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| s.sampler = ImageSampler::nearest())
        .load(path)
}

fn spawn(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Heart::default());
    let row = commands
        .spawn((
            GameEntity,
            Node {
                position_type: PositionType::Absolute,
                left: px(18),
                top: px(16),
                column_gap: px(14),
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();

    // A speech bubble with T, the key that opens the chat, on it.
    commands.spawn((
        Node {
            width: px(ICON_SIZE.x),
            height: px(ICON_SIZE.y),
            ..default()
        },
        ImageNode::new(load_pixels(&assets, "ui/chat.png")),
        ChildOf(row),
    ));

    // The heart: the grey one underneath, the red one over it cut off at how full it is.
    let heart = commands
        .spawn((
            Node {
                width: px(ICON_SIZE.x),
                height: px(ICON_SIZE.y),
                ..default()
            },
            ChildOf(row),
        ))
        .id();
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ImageNode::new(load_pixels(&assets, "ui/heart_empty.png"))
            .with_color(Color::srgba(1.0, 1.0, 1.0, 0.75)),
        ChildOf(heart),
    ));
    let fill = commands
        .spawn((
            HeartFill,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(0),
                width: percent(100),
                height: percent(START * 100.0),
                overflow: Overflow::clip(),
                ..default()
            },
            ChildOf(heart),
        ))
        .id();
    // Pinned to the bottom of the clipping box, so shortening the box uncovers it from the top.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(0),
            width: px(ICON_SIZE.x),
            height: px(ICON_SIZE.y),
            ..default()
        },
        ImageNode::new(load_pixels(&assets, "ui/heart_full.png")),
        ChildOf(fill),
    ));
}

fn drain_and_show(
    time: Res<Time>,
    heart: Option<ResMut<Heart>>,
    mut fill: Query<&mut Node, With<HeartFill>>,
) {
    let Some(mut heart) = heart else { return };
    heart.0 = (heart.0 - DRAIN * time.delta_secs()).max(0.0);
    // The art's top and bottom rows are outline; map the fill onto the 32 red rows between, the
    // lowest always showing, and the top outline along with the last of them.
    let shown = (2.0 + (heart.0 / LOOKS_FULL).min(1.0) * 32.0) / 34.0;
    if let Ok(mut node) = fill.single_mut() {
        let h = percent(if heart.0 <= 0.0 { 0.0 } else { shown * 100.0 });
        if node.height != h {
            node.height = h;
        }
    }
}
