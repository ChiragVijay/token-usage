use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuId, MenuItem, PredefinedMenuItem},
};

pub struct Tray {
    pub icon: TrayIcon,
    pub overview: MenuId,
    pub compact: MenuId,
    pub refresh: MenuId,
    pub quit: MenuId,
}
pub fn create() -> anyhow::Result<Tray> {
    let menu = Menu::new();
    let overview = MenuItem::new("Open dashboard", true, None);
    let compact = MenuItem::new("Quick view", true, None);
    let refresh = MenuItem::new("Refresh usage", true, None);
    let quit = MenuItem::new("Quit Token Usage", true, None);
    menu.append_items(&[
        &overview,
        &compact,
        &refresh,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let pixels = if cfg!(target_os = "macos") {
        include_bytes!("../../assets/TrayIconTemplate.rgba").to_vec()
    } else {
        include_bytes!("../../assets/TrayIcon.rgba").to_vec()
    };
    let icon = TrayIconBuilder::new()
        .with_icon(Icon::from_rgba(pixels, 32, 32)?)
        .with_tooltip("Token Usage")
        .with_menu(Box::new(menu))
        .with_icon_as_template(cfg!(target_os = "macos"))
        .with_menu_on_left_click(cfg!(target_os = "macos"))
        .build()?;
    Ok(Tray {
        icon,
        overview: overview.id().clone(),
        compact: compact.id().clone(),
        refresh: refresh.id().clone(),
        quit: quit.id().clone(),
    })
}
