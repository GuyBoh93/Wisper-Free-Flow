// Global keyboard listener via rdev::grab. Suppresses the hotkey's main key
// (and its release) while the hotkey is active, so holding e.g. shift+space
// does NOT leak a space character into the focused app.
//
// Modifier keys (shift/ctrl/alt/meta) are passed through unchanged — the
// occasional standalone "shift tap" is invisible to most apps and avoids
// surprising side effects.
//
// The grab callback runs on the OS hook thread and must return quickly. All
// it does is update a small Mutex-guarded state and push events to an mpsc.

use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use rdev::{Event, EventType, Key, grab};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}

#[derive(Debug, Clone)]
struct HotkeyDef {
    modifiers: HashSet<Key>,
    key: Key,
}

struct ListenerState {
    held: HashSet<Key>,
    active: bool,
}

pub fn spawn_listener(hotkey_str: &str) -> Result<Receiver<HotkeyEvent>> {
    let hotkey = parse(hotkey_str)?;
    let (tx, rx) = channel();

    std::thread::spawn(move || {
        let state = Arc::new(Mutex::new(ListenerState {
            held: HashSet::new(),
            active: false,
        }));
        let hotkey = hotkey;
        let tx = tx;

        let callback = move |event: Event| -> Option<Event> {
            handle_event(event, &hotkey, &state, &tx)
        };

        if let Err(e) = grab(callback) {
            tracing::error!("rdev grab error: {e:?}");
        }
    });

    Ok(rx)
}

fn handle_event(
    event: Event,
    hotkey: &HotkeyDef,
    state: &Arc<Mutex<ListenerState>>,
    tx: &Sender<HotkeyEvent>,
) -> Option<Event> {
    let mut suppress = false;

    match event.event_type {
        EventType::KeyPress(k) => {
            let mut s = state.lock();
            s.held.insert(k);

            if !s.active && combo_satisfied(hotkey, &s.held) {
                s.active = true;
                let _ = tx.send(HotkeyEvent::Pressed);
            }

            // Suppress only the main key (not modifiers) while the hotkey is active.
            if s.active && k == hotkey.key {
                suppress = true;
            }
        }
        EventType::KeyRelease(k) => {
            let mut s = state.lock();
            s.held.remove(&k);

            let was_active = s.active;
            let is_main = k == hotkey.key;
            let is_mod = hotkey.modifiers.contains(&k);

            if s.active && (is_main || is_mod) {
                s.active = false;
                let _ = tx.send(HotkeyEvent::Released);
            }

            // Suppress the release that pairs with a suppressed press.
            if was_active && is_main {
                suppress = true;
            }
        }
        _ => {}
    }

    if suppress { None } else { Some(event) }
}

fn combo_satisfied(hk: &HotkeyDef, held: &HashSet<Key>) -> bool {
    held.contains(&hk.key) && hk.modifiers.iter().all(|m| held.contains(m))
}

fn parse(s: &str) -> Result<HotkeyDef> {
    let mut modifiers = HashSet::new();
    let mut key: Option<Key> = None;

    for raw in s.split('+').map(|p| p.trim().to_lowercase()) {
        match raw.as_str() {
            "shift" | "lshift" | "left_shift" => {
                modifiers.insert(Key::ShiftLeft);
            }
            "rshift" | "right_shift" => {
                modifiers.insert(Key::ShiftRight);
            }
            "ctrl" | "control" | "lctrl" | "left_ctrl" => {
                modifiers.insert(Key::ControlLeft);
            }
            "rctrl" | "right_ctrl" => {
                modifiers.insert(Key::ControlRight);
            }
            "alt" => {
                modifiers.insert(Key::Alt);
            }
            "ralt" | "right_alt" | "altgr" => {
                modifiers.insert(Key::AltGr);
            }
            "cmd" | "win" | "super" | "meta" => {
                modifiers.insert(Key::MetaLeft);
            }
            other => {
                key = Some(named_key(other)?);
            }
        }
    }

    let key = key.ok_or_else(|| anyhow!("hotkey must include a non-modifier key: {s}"))?;
    Ok(HotkeyDef { modifiers, key })
}

fn named_key(name: &str) -> Result<Key> {
    use Key::*;
    let k = match name {
        "space" => Space,
        "tab" => Tab,
        "enter" | "return" => Return,
        "escape" | "esc" => Escape,
        "backspace" => Backspace,
        "capslock" | "caps_lock" => CapsLock,
        "scrolllock" | "scroll_lock" => ScrollLock,
        "pause" => Pause,
        "insert" => Insert,
        "delete" | "del" => Delete,
        "home" => Home,
        "end" => End,
        "pageup" | "page_up" => PageUp,
        "pagedown" | "page_down" => PageDown,
        "f1" => F1, "f2" => F2, "f3" => F3, "f4" => F4,
        "f5" => F5, "f6" => F6, "f7" => F7, "f8" => F8,
        "f9" => F9, "f10" => F10, "f11" => F11, "f12" => F12,
        other if other.len() == 1 => {
            let c = other.chars().next().unwrap().to_ascii_uppercase();
            match c {
                'A' => KeyA, 'B' => KeyB, 'C' => KeyC, 'D' => KeyD,
                'E' => KeyE, 'F' => KeyF, 'G' => KeyG, 'H' => KeyH,
                'I' => KeyI, 'J' => KeyJ, 'K' => KeyK, 'L' => KeyL,
                'M' => KeyM, 'N' => KeyN, 'O' => KeyO, 'P' => KeyP,
                'Q' => KeyQ, 'R' => KeyR, 'S' => KeyS, 'T' => KeyT,
                'U' => KeyU, 'V' => KeyV, 'W' => KeyW, 'X' => KeyX,
                'Y' => KeyY, 'Z' => KeyZ,
                '0' => Num0, '1' => Num1, '2' => Num2, '3' => Num3, '4' => Num4,
                '5' => Num5, '6' => Num6, '7' => Num7, '8' => Num8, '9' => Num9,
                _ => return Err(anyhow!("unsupported key: {other}")),
            }
        }
        _ => return Err(anyhow!("unknown key name: {name}")),
    };
    Ok(k)
}
