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
    let quit = MenuItem::new("Quit TokenDock", true, None);
    menu.append_items(&[
        &overview,
        &compact,
        &refresh,
        &PredefinedMenuItem::separator(),
        &quit,
    ])?;
    let mut pixels = vec![0u8; 32 * 32 * 4];
    for y in 5..27 {
        for x in 5..27 {
            let active = ((5..10).contains(&x) && (10..25).contains(&y))
                || ((13..18).contains(&x) && (5..22).contains(&y))
                || ((21..26).contains(&x) && (13..27).contains(&y));
            if active {
                let i = (y * 32 + x) * 4;
                pixels[i] = 77;
                pixels[i + 1] = 130;
                pixels[i + 2] = 222;
                pixels[i + 3] = 255;
            }
        }
    }
    let icon = TrayIconBuilder::new()
        .with_icon(Icon::from_rgba(pixels, 32, 32)?)
        .with_tooltip("TokenDock")
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
