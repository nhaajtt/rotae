use crate::settings::{self, ACCENTS, THEMES};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Listener, Wry,
};

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let s = settings::get(app);
    let autostart = crate::platform::autostart_enabled(app);
    let title = MenuItem::with_id(app, "title", if cfg!(windows) { "Rotae — Shift + drag a file" } else { "Rotae — drag a file to the screen edge" }, false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Rotae…", true, None::<&str>)?;
    let shelf = MenuItem::with_id(app, "shelf", "Shelf", true, None::<&str>)?;
    let help = MenuItem::with_id(app, "welcome", "How to use", true, None::<&str>)?;
    let activity = MenuItem::with_id(app, "activity", "Activity (results and errors)", true, None::<&str>)?;
    let enabled = CheckMenuItem::with_id(app, "enabled", "Enabled", true, s.enabled, None::<&str>)?;
    let explorer = CheckMenuItem::with_id(app, "explorer_only", if cfg!(windows) { "Only from File Explorer and desktop" } else { "Only from the file manager" }, true, s.explorer_only, None::<&str>)?;
    let startup = CheckMenuItem::with_id(app, "autostart", if cfg!(windows) { "Start with Windows" } else { "Start at login" }, true, autostart, None::<&str>)?;

    let accent = Submenu::with_id(app, "accent", "Accent", true)?;
    for a in ACCENTS {
        accent.append(&CheckMenuItem::with_id(app, format!("accent:{a}"), cap(a), true, s.accent == a, None::<&str>)?)?;
    }
    let theme = Submenu::with_id(app, "theme", "Theme", true)?;
    for t in THEMES {
        theme.append(&CheckMenuItem::with_id(app, format!("theme:{t}"), cap(t), true, s.theme == t, None::<&str>)?)?;
    }
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    // Linux can't tell which program a drag comes from, so "only from Explorer" has no meaning there
    let (s1, s2, s3) = (sep()?, sep()?, sep()?);
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = vec![&title, &open, &activity, &shelf, &help, &s1, &enabled];
    if cfg!(windows) {
        items.push(&explorer);
    }
    items.extend([&startup as &dyn tauri::menu::IsMenuItem<Wry>, &s2, &accent, &theme, &s3, &quit]);
    Menu::with_items(app, &items)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("icon"))
        .tooltip("Rotae")
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| {
            let id = ev.id().as_ref();
            match id {
                "quit" => {
                    crate::log("quit from tray");
                    app.exit(0)
                }
                "open" => {
                    let app = app.clone();
                    // building a window inside a menu event can deadlock on Windows
                    std::thread::spawn(move || crate::windows::open_hub_now(app, vec![], "actions".into(), None));
                }
                "activity" => {
                    let app = app.clone();
                    // building a window inside a menu event can deadlock on Windows
                    std::thread::spawn(move || crate::windows::open_hub_now(app, vec![], "activity".into(), None));
                }
                "welcome" => {
                    let app = app.clone();
                    // building a window inside a menu event can deadlock on Windows
                    std::thread::spawn(move || crate::windows::open_hub_now(app, vec![], "welcome".into(), None));
                }
                "shelf" => {
                    let app = app.clone();
                    std::thread::spawn(move || crate::windows::shelf_add_now(app, vec![]));
                }
                "enabled" => settings::update(app, |s| s.enabled = !s.enabled),
                "explorer_only" => settings::update(app, |s| s.explorer_only = !s.explorer_only),
                "autostart" => {
                    let _ = crate::platform::set_autostart(app, !crate::platform::autostart_enabled(app));
                }
                _ => {
                    if let Some(a) = id.strip_prefix("accent:") {
                        settings::update(app, |s| s.accent = a.into());
                    } else if let Some(t) = id.strip_prefix("theme:") {
                        settings::update(app, |s| s.theme = t.into());
                    }
                }
            }
            // rebuild so the check marks match the stored state (radio-like groups)
            if let (Some(tray), Ok(menu)) = (app.tray_by_id("main"), build_menu(app)) {
                let _ = tray.set_menu(Some(menu));
            }
        })
        .build(app)?;
    // settings changed in the hub: keep the tray's check marks in step
    let handle = app.clone();
    app.listen_any("settings", move |_| {
        if let (Some(tray), Ok(menu)) = (handle.tray_by_id("main"), build_menu(&handle)) {
            let _ = tray.set_menu(Some(menu));
        }
    });
    Ok(())
}
