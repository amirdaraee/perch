use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

fn toggle_popover(app: &tauri::AppHandle) {
    use tauri_plugin_positioner::{Position, WindowExt};

    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        // Anchor under the menu-bar item. Must precede show(), or the window is
        // briefly painted centre-screen and then jumps.
        let _ = win.move_window(Position::TrayBottomCenter);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .setup(|app| {
            // Menu-bar app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // A menu-bar popover must appear over whatever Space is active, including a
            // fullscreen app's own Space. Without this it silently fails to show there.
            if let Some(win) = app.get_webview_window("popover") {
                let _ = win.set_visible_on_all_workspaces(true);
            }

            let quit = MenuItem::with_id(app, "quit", "Quit Perch", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            TrayIconBuilder::with_id("perch-tray")
                // `?`, not `unwrap()`: a missing icon must surface as a setup
                // error, not a panic on launch.
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .ok_or("no default window icon configured")?,
                )
                .icon_as_template(true)
                .title("Perch")
                .menu(&menu)
                // Left click toggles the popover; the menu is right-click only.
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    if event.id().as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // Must run for every event, not just left-clicks — the
                    // plugin needs the position updates to anchor correctly.
                    tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the popover must not quit the app — Perch lives in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            // A popover that outlives its focus is just a window. Clicking
            // away dismisses it.
            if let WindowEvent::Focused(false) = event {
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Perch");
}
