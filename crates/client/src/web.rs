//! Browser glue: tells the page when the game is ready to show, and notices when the browser
//! itself lets go of the pointer lock (Esc, alt-tab) without the game seeing it.

use crate::{
    chat::{ChatCancel, ChatOpen},
    vegetation::VegetationReady,
};
use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// Defined by index.html: hides the loading screen.
    #[wasm_bindgen(catch, js_namespace = window, js_name = deerReady)]
    fn deer_ready() -> Result<(), JsValue>;
}

pub fn plugin(app: &mut App) {
    console_error_panic_hook::set_once();
    app.add_systems(
        Update,
        (
            ready.run_if(resource_added::<VegetationReady>),
            lost_pointer_lock,
        ),
    );
}

fn ready() {
    let _ = deer_ready();
}

/// The browser grants the lock a moment after it is asked for, so only a lock that was seen
/// and then went away counts as lost.
fn lost_pointer_lock(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    window: Single<&Window, With<PrimaryWindow>>,
    chat: Res<ChatOpen>,
    mut cancel: ResMut<ChatCancel>,
    mut had_lock: Local<bool>,
) {
    if cursor.grab_mode == CursorGrabMode::None {
        *had_lock = false;
        return;
    }
    let locked = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.pointer_lock_element())
        .is_some();
    if locked {
        *had_lock = true;
    } else if *had_lock {
        *had_lock = false;
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        // Lost with the window still in front, that was Esc: it closes the chat box too.
        if chat.0 && window.focused {
            cancel.0 = true;
        }
    }
}
