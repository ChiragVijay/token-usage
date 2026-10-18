#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod ui;
use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
gpui::actions!(token_dock, [Quit]);

fn main() {
    // A downloaded exe copies itself, starts the installed copy, and exits.
    // The lock belongs to the process that continues on and opens the window.
    #[cfg(windows)]
    token_dock::install::install_and_handoff();
    #[cfg(windows)]
    if !ui::claim_single_instance() {
        std::process::exit(0);
    }
    Application::new()
        .with_assets(ui::assets())
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
            cx.bind_keys([gpui::KeyBinding::new("secondary-q", Quit, None)]);
            #[cfg(target_os = "macos")]
            cx.set_menus(vec![gpui::Menu {
                name: "TokenDock".into(),
                items: vec![gpui::MenuItem::action("Quit TokenDock", Quit)],
            }]);
            let bounds = Bounds::centered(None, size(px(1280.), px(860.)), cx);
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(420.), px(560.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(ui::WINDOW_TITLE.into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    let dashboard = cx.new(|cx| ui::Dashboard::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(dashboard, window, cx))
                },
            );
            if result.is_err() {
                eprintln!("TokenDock could not open its native window");
                cx.quit();
                return;
            }
            cx.activate(true);
        });
}
