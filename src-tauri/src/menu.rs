//! Native application menu builder for Narrator.
//! Uses `#[cfg]` for strict platform separation.

use tauri::{
    menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu},
    App, AppHandle, Wry,
};

/// Custom menu item IDs — forwarded to the frontend via events.
pub const NEW_PROJECT: &str = "new_project";
pub const OPEN_PROJECT: &str = "open_project";
pub const SAVE_PROJECT: &str = "save_project";
pub const OPEN_SETTINGS: &str = "open_settings";
pub const NARRATOR_HELP: &str = "narrator_help";
/// Prefix for dynamic "Open Recent" items — full ID is "recent:<project_id>"
pub const RECENT_PREFIX: &str = "recent:";

/// IDs of items that require an open project.
const PROJECT_ITEMS: &[&str] = &[SAVE_PROJECT];

/// Build the native application menu for the current platform.
pub fn build(app: &App) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;

    #[cfg(target_os = "macos")]
    build_macos(app, &menu)?;

    #[cfg(not(target_os = "macos"))]
    build_windows_linux(app, &menu)?;

    Ok(menu)
}

/// Enable or disable menu items that depend on whether a project is open.
/// Also rebuilds the "Open Recent" submenu with the latest project list.
pub fn set_project_context(app: &AppHandle, has_project: bool) {
    let Some(menu) = app.menu() else { return };
    for id in PROJECT_ITEMS {
        if let Some(MenuItemKind::MenuItem(item)) = menu.get(*id) {
            let _ = item.set_enabled(has_project);
        }
    }

    // Rebuild Open Recent submenu
    if let Some(MenuItemKind::Submenu(recent)) = menu.get("open_recent") {
        let _ = rebuild_recent_submenu(app, &recent);
    }
}

/// Populate the "Open Recent" submenu from the project store.
fn rebuild_recent_submenu(app: &AppHandle, submenu: &Submenu<Wry>) -> tauri::Result<()> {
    // Clear existing items
    while submenu.remove_at(0).is_ok() {}

    let projects = crate::project_store::list_projects().unwrap_or_default();
    if projects.is_empty() {
        let empty = MenuItem::with_id(
            app,
            "recent_empty",
            "No Recent Projects",
            false,
            None::<&str>,
        )?;
        submenu.append(&empty)?;
    } else {
        // Show the 8 most recent
        for p in projects.iter().take(8) {
            let id = format!("{}{}", RECENT_PREFIX, p.id);
            let item = MenuItem::with_id(app, id, &p.title, true, None::<&str>)?;
            submenu.append(&item)?;
        }
    }
    Ok(())
}

/// Register the Help submenu with macOS so the system adds
/// the built-in search field that searches all menu items.
#[cfg(target_os = "macos")]
pub fn set_native_help_menu() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{MainThreadMarker, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(main_menu) = app.mainMenu() else {
        return;
    };
    let help_title = NSString::from_str("Help");
    let Some(help_item) = main_menu.itemWithTitle(&help_title) else {
        return;
    };
    let Some(help_submenu) = help_item.submenu() else {
        return;
    };
    app.setHelpMenu(Some(&help_submenu));
}

// ─── macOS ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn build_macos(app: &App, menu: &Menu<Wry>) -> tauri::Result<()> {
    // ── Narrator ──
    let app_menu = Submenu::new(app, "Narrator", true)?;
    app_menu.append(&PredefinedMenuItem::about(
        app,
        Some("About Narrator"),
        None,
    )?)?;
    app_menu.append(&PredefinedMenuItem::separator(app)?)?;
    app_menu.append(&MenuItem::with_id(
        app,
        OPEN_SETTINGS,
        "Settings…",
        true,
        Some("CmdOrCtrl+,"),
    )?)?;
    app_menu.append(&PredefinedMenuItem::separator(app)?)?;
    app_menu.append(&PredefinedMenuItem::services(app, None)?)?;
    app_menu.append(&PredefinedMenuItem::separator(app)?)?;
    app_menu.append(&PredefinedMenuItem::hide(app, Some("Hide Narrator"))?)?;
    app_menu.append(&PredefinedMenuItem::hide_others(app, None)?)?;
    app_menu.append(&PredefinedMenuItem::show_all(app, None)?)?;
    app_menu.append(&PredefinedMenuItem::separator(app)?)?;
    app_menu.append(&PredefinedMenuItem::quit(app, Some("Quit Narrator"))?)?;
    menu.append(&app_menu)?;

    // ── File ──
    let file_menu = Submenu::new(app, "File", true)?;
    file_menu.append(&MenuItem::with_id(
        app,
        NEW_PROJECT,
        "New Project…",
        true,
        Some("CmdOrCtrl+N"),
    )?)?;
    file_menu.append(&MenuItem::with_id(
        app,
        OPEN_PROJECT,
        "Open Project…",
        true,
        Some("CmdOrCtrl+O"),
    )?)?;
    let recent_sub = Submenu::with_id(app, "open_recent", "Open Recent", true)?;
    let empty = MenuItem::with_id(
        app,
        "recent_empty",
        "No Recent Projects",
        false,
        None::<&str>,
    )?;
    recent_sub.append(&empty)?;
    file_menu.append(&recent_sub)?;
    file_menu.append(&PredefinedMenuItem::separator(app)?)?;
    file_menu.append(&MenuItem::with_id(
        app,
        SAVE_PROJECT,
        "Save Project",
        false,
        Some("CmdOrCtrl+S"),
    )?)?;
    file_menu.append(&PredefinedMenuItem::separator(app)?)?;
    file_menu.append(&PredefinedMenuItem::close_window(app, None)?)?;
    menu.append(&file_menu)?;

    // ── Edit ──
    let edit_menu = Submenu::new(app, "Edit", true)?;
    edit_menu.append(&PredefinedMenuItem::undo(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::redo(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::separator(app)?)?;
    edit_menu.append(&PredefinedMenuItem::cut(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::copy(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::paste(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::select_all(app, None)?)?;
    menu.append(&edit_menu)?;

    // ── Window ──
    let window_menu = Submenu::new(app, "Window", true)?;
    window_menu.append(&PredefinedMenuItem::minimize(app, None)?)?;
    window_menu.append(&PredefinedMenuItem::maximize(app, None)?)?;
    window_menu.append(&PredefinedMenuItem::separator(app)?)?;
    window_menu.append(&PredefinedMenuItem::fullscreen(app, None)?)?;
    menu.append(&window_menu)?;

    // ── Help ──
    let help_menu = Submenu::new(app, "Help", true)?;
    help_menu.append(&MenuItem::with_id(
        app,
        NARRATOR_HELP,
        "Narrator Help",
        true,
        None::<&str>,
    )?)?;
    menu.append(&help_menu)?;

    Ok(())
}

// ─── Windows / Linux ─────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
fn build_windows_linux(app: &App, menu: &Menu<Wry>) -> tauri::Result<()> {
    // ── &File ──  (& prefix = Windows mnemonic/access key)
    let file_menu = Submenu::new(app, "&File", true)?;
    file_menu.append(&MenuItem::with_id(
        app,
        NEW_PROJECT,
        "&New Project…",
        true,
        Some("CmdOrCtrl+N"),
    )?)?;
    file_menu.append(&MenuItem::with_id(
        app,
        OPEN_PROJECT,
        "&Open Project…",
        true,
        Some("CmdOrCtrl+O"),
    )?)?;
    let recent_sub = Submenu::with_id(app, "open_recent", "Open &Recent", true)?;
    let empty = MenuItem::with_id(
        app,
        "recent_empty",
        "No Recent Projects",
        false,
        None::<&str>,
    )?;
    recent_sub.append(&empty)?;
    file_menu.append(&recent_sub)?;
    file_menu.append(&PredefinedMenuItem::separator(app)?)?;
    file_menu.append(&MenuItem::with_id(
        app,
        SAVE_PROJECT,
        "&Save Project",
        false,
        Some("CmdOrCtrl+S"),
    )?)?;
    file_menu.append(&PredefinedMenuItem::separator(app)?)?;
    file_menu.append(&MenuItem::with_id(
        app,
        OPEN_SETTINGS,
        "Se&ttings",
        true,
        Some("CmdOrCtrl+,"),
    )?)?;
    file_menu.append(&PredefinedMenuItem::separator(app)?)?;
    file_menu.append(&PredefinedMenuItem::quit(app, Some("E&xit"))?)?;
    menu.append(&file_menu)?;

    // ── &Edit ──
    let edit_menu = Submenu::new(app, "&Edit", true)?;
    edit_menu.append(&PredefinedMenuItem::undo(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::redo(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::separator(app)?)?;
    edit_menu.append(&PredefinedMenuItem::cut(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::copy(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::paste(app, None)?)?;
    edit_menu.append(&PredefinedMenuItem::select_all(app, None)?)?;
    menu.append(&edit_menu)?;

    // ── &View ──
    let view_menu = Submenu::new(app, "&View", true)?;
    view_menu.append(&MenuItem::with_id(
        app,
        "toggle_fullscreen",
        "Toggle &Full Screen",
        true,
        Some("F11"),
    )?)?;
    menu.append(&view_menu)?;

    // ── &Help ──
    let help_menu = Submenu::new(app, "&Help", true)?;
    help_menu.append(&PredefinedMenuItem::about(
        app,
        Some("&About Narrator"),
        None,
    )?)?;
    help_menu.append(&PredefinedMenuItem::separator(app)?)?;
    help_menu.append(&MenuItem::with_id(
        app,
        NARRATOR_HELP,
        "Narrator &Help",
        true,
        Some("F1"),
    )?)?;
    menu.append(&help_menu)?;

    Ok(())
}
