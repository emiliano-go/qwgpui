mod assets;
mod backend;
mod clipboard;
mod config;
mod frontend;
mod ipc;
mod model;
mod password;
mod search;

use gpui::{
    px, size, App, AppContext as _, Application, Bounds, ClipboardItem, TitlebarOptions,
    WindowBounds, WindowOptions,
};

use crate::frontend::VaultApp;

fn main() {
    Application::new()
        .with_assets(crate::assets::Assets)
        .run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1520.), px(960.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Vault".into()),
                        ..Default::default()
                    }),
                    app_id: Some("vault".into()),
                    window_min_size: Some(size(px(1120.), px(720.))),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(VaultApp::new);
                    let guard = view.read(cx).clipboard_guard();
                    let clear_on_exit = view.read(cx).clear_on_exit();
                    cx.on_app_quit(move |app| {
                        if clear_on_exit {
                            let current = app.read_from_clipboard().and_then(|item| item.text());
                            if guard.matches(current.as_deref()) {
                                app.write_to_clipboard(ClipboardItem::new_string(String::new()));
                            }
                        }
                        async {}
                    })
                    .detach();
                    window.focus(&view.read(cx).focus_handle());
                    view
                },
            )
            .expect("failed to open Vault window");
            cx.activate(true);
        });
}
