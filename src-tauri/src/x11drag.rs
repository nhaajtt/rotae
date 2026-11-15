//! Linux X11: holding Shift while dragging opens the wheel right under the pointer, as on Windows.
//! The drag then enters the wheel window by itself, which hands over the dragged files, so there
//! is no need to ask the file manager what is selected. The pointer and modifier state are read
//! from the X server about 60 times a second (no grabs, nothing is intercepted). Wayland doesn't let
//! apps watch the pointer outside their windows; there the drop strip is the way in.

use std::{cell::Cell, rc::Rc, time::Duration};
use tauri::AppHandle;

/// Pixels the pointer must travel with the button down before it counts as a drag.
const THRESHOLD: i32 = 10;

pub fn start(app: AppHandle) {
    let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"));
    if wayland || std::env::var_os("DISPLAY").is_none() {
        crate::log("shift-drag: not an X11 session, using the drop strip only");
        return;
    }
    let a = app.clone();
    if app.run_on_main_thread(move || watch(a)).is_err() {
        crate::log("shift-drag: couldn't start");
    }
}

fn watch(app: AppHandle) {
    use gtk::{gdk, glib, prelude::*};
    let Some(display) = gdk::Display::default() else { return };
    let Some(pointer) = display.default_seat().and_then(|s| s.pointer()) else { return };
    let Some(root) = gdk::Screen::default().and_then(|s| s.root_window()) else { return };
    crate::log("shift-drag: watching the pointer (X11)");
    let down: Rc<Cell<Option<(i32, i32)>>> = Rc::new(Cell::new(None));
    let fired = Rc::new(Cell::new(false));
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let (_, x, y, mask) = root.device_position(&pointer);
        let button = mask.contains(gdk::ModifierType::BUTTON1_MASK);
        let shift = mask.contains(gdk::ModifierType::SHIFT_MASK);
        match (button, down.get()) {
            (true, None) => {
                // a drag that starts on one of Rotae's own windows (shelf, toast) isn't for the wheel
                let own = pointer.window_at_position().0.is_some();
                down.set(Some((x, y)));
                fired.set(own);
            }
            (true, Some((ox, oy))) if !fired.get() && shift && ((x - ox).abs() > THRESHOLD || (y - oy).abs() > THRESHOLD) => {
                fired.set(true);
                let (on, open) = (crate::settings::get(&app).enabled, crate::windows::wheel_open());
                crate::log(&format!("shift-drag at {x},{y} (menu on: {on}, wheel already open: {open})"));
                if on && !open {
                    let k = root.scale_factor().max(1);
                    let (app, px, py) = (app.clone(), x * k, y * k);
                    // window calls from the GTK loop's own callback are handed to a thread
                    std::thread::spawn(move || crate::windows::zone::open_for_drag(&app, px, py));
                }
            }
            (false, Some(_)) => {
                down.set(None);
                if fired.replace(false) {
                    // released without the drag ever reaching the wheel (a text selection, a
                    // drag of something that isn't files): close it again
                    let app = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(500));
                        crate::windows::zone::give_up(&app);
                    });
                }
            }
            _ => {}
        }
        glib::ControlFlow::Continue
    });
}
