use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

// A menu-bar popover must appear over whatever Space is active, including a
// fullscreen app's own Space. Debugging established that neither
// set_visible_on_all_workspaces() nor setting the raw NSWindowCollectionBehavior
// directly on the plain NSWindow was sufficient: isOnActiveSpace stayed false
// over a fullscreen app regardless. macOS only honours those behaviours on an
// NSPanel; tauri-nspanel subclasses the window into one at runtime, which is
// what this panel type is for. tauri-nspanel is AppKit-only (it references
// objc2/objc2-app-kit unconditionally in its own source, gating only its
// *dependency* declaration to macOS), so both the dependency and every use of
// it here are macOS-gated, with a plain-window fallback for other targets.
#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(PopoverPanel {
        config: {
            can_become_key_window: true,
            can_become_main_window: false
        }
    })
}

#[cfg(target_os = "macos")]
fn toggle_popover(app: &tauri::AppHandle) {
    use tauri_nspanel::ManagerExt;
    use tauri_plugin_positioner::{Position, WindowExt};

    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    let Ok(panel) = app.get_webview_panel("popover") else {
        return;
    };
    if panel.is_visible() {
        panel.hide();
    } else {
        // Anchor under the menu-bar item. Must precede show(), or the window is
        // briefly painted centre-screen and then jumps. The positioner plugin
        // operates on the underlying window, which still applies once it has
        // been converted to an NSPanel.
        let _ = win.move_window(Position::TrayBottomCenter);
        // show() alone (orderFrontRegardless) never takes key window status.
        // show_and_make_key() also makes the content view first responder, which
        // is what let win.show() + win.set_focus() achieve on a plain window —
        // needed so the popover can take keyboard input for Task 8's
        // Escape-to-dismiss. The non-activating style mask set at conversion
        // time is what lets it become key without activating the app.
        panel.show_and_make_key();
    }
}

#[cfg(not(target_os = "macos"))]
fn toggle_popover(app: &tauri::AppHandle) {
    use tauri_plugin_positioner::{Position, WindowExt};

    let Some(win) = app.get_webview_window("popover") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
    } else {
        let _ = win.move_window(Position::TrayBottomCenter);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_positioner::init());

    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .setup(|app| {
            // Menu-bar app: no Dock icon, no app switcher entry.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Convert the popover window to an NSPanel so it can be shown over a
            // fullscreen app's Space. See the module-level comment above the
            // PopoverPanel definition for why this replaced a direct
            // NSWindowCollectionBehavior set on the plain NSWindow.
            #[cfg(target_os = "macos")]
            {
                use tauri_nspanel::{CollectionBehavior, PanelLevel, StyleMask, WebviewWindowExt};

                if let Some(win) = app.get_webview_window("popover") {
                    match win.to_panel::<PopoverPanel>() {
                        Ok(panel) => {
                            panel.set_level(PanelLevel::Floating.value());
                            // Non-activating: the documented requirement for a
                            // panel to be shown over a fullscreen app.
                            panel.set_style_mask(StyleMask::empty().nonactivating_panel().into());
                            panel.set_collection_behavior(
                                CollectionBehavior::new()
                                    .full_screen_auxiliary()
                                    .can_join_all_spaces()
                                    .into(),
                            );
                            panel.set_hides_on_deactivate(false);
                        }
                        Err(err) => {
                            // Must not panic at launch: fall through with default
                            // window behaviour rather than aborting setup over a
                            // fullscreen-visibility refinement.
                            eprintln!("perch: failed to convert popover window to panel: {err}");
                        }
                    }
                }
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
