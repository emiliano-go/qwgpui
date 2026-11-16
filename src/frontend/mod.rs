use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
mod theme;

use gpui::{
    div, px, rgba, uniform_list, ClipboardItem, Context, Div, FocusHandle, Focusable, KeyDownEvent,
    Keystroke, Render, ScrollStrategy, SharedString, Timer, UniformListScrollHandle, Window,
};
use zeroize::Zeroizing;

use crate::backend::{Backend, VaultError};
use crate::clipboard::{should_clear, ClipboardGuard};
use crate::config::{self, Config, CopyField, Theme};
use crate::model::{ItemDetails, ItemMeta, ItemType};
use crate::password;
use crate::search;
use theme::Palette;

const TOTP_STEP: u64 = 30;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Favorites,
    Logins,
    Notes,
    Cards,
    Identities,
}

impl Filter {
    const ALL: [Filter; 6] = [
        Filter::All,
        Filter::Favorites,
        Filter::Logins,
        Filter::Notes,
        Filter::Cards,
        Filter::Identities,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All items",
            Self::Favorites => "Favorites",
            Self::Logins => "Logins",
            Self::Notes => "Secure notes",
            Self::Cards => "Cards",
            Self::Identities => "Identities",
        }
    }

    fn accepts(self, item: &ItemMeta, favorites: &HashSet<String>) -> bool {
        match self {
            Self::All => true,
            Self::Favorites => favorites.contains(&item.id),
            Self::Logins => item.item_type == ItemType::Login,
            Self::Notes => item.item_type == ItemType::SecureNote,
            Self::Cards => item.item_type == ItemType::Card,
            Self::Identities => item.item_type == ItemType::Identity,
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Favorites => "Favorites",
            Self::Logins => "Logins",
            Self::Notes => "Notes",
            Self::Cards => "Cards",
            Self::Identities => "Identities",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::All => "icons/list.svg",
            Self::Favorites => "icons/star.svg",
            Self::Logins => "icons/key.svg",
            Self::Notes => "icons/note.svg",
            Self::Cards => "icons/card.svg",
            Self::Identities => "icons/identity.svg",
        }
    }
}

enum Status {
    Starting,
    Locked,
    Ready,
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Vault,
    Settings,
    Generator,
}

struct Toast {
    text: String,
    seq: u64,
}

pub struct VaultApp {
    focus: FocusHandle,
    backend: Backend,
    config: Config,
    system_dark: bool,
    clipboard: ClipboardGuard,
    favorites: HashSet<String>,
    items: Vec<ItemMeta>,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
    filter: Filter,
    status: Status,
    details: Option<ItemDetails>,
    details_loading: bool,
    show_password: bool,
    totp: Option<Zeroizing<String>>,
    totp_remaining: u64,
    detail_generation: u64,
    toast: Option<Toast>,
    toast_seq: u64,
    screen: Screen,
    generated: Option<Zeroizing<String>>,
    list_scroll: UniformListScrollHandle,
}

impl VaultApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (config, notice) = Config::load();
        let system_dark = theme::start_watcher();
        let favorites = config::load_favorites();
        let mut app = Self {
            focus: cx.focus_handle(),
            backend: Backend::new(config.script_path().to_string()),
            config,
            system_dark,
            clipboard: ClipboardGuard::default(),
            favorites: favorites.into_iter().collect(),
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            query: String::new(),
            filter: Filter::All,
            status: Status::Starting,
            details: None,
            details_loading: false,
            show_password: false,
            totp: None,
            totp_remaining: TOTP_STEP,
            detail_generation: 0,
            toast: None,
            toast_seq: 0,
            screen: Screen::Vault,
            generated: None,
            list_scroll: UniformListScrollHandle::new(),
        };
        if let Some(notice) = notice {
            app.show_toast(notice, cx);
        }
        app.refresh_vault(cx);
        app.watch_system_theme(cx);
        app
    }

    fn watch_system_theme(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |weak, cx| loop {
            Timer::after(Duration::from_secs(1)).await;
            let dark = theme::system_dark();
            let _ = weak.update(cx, |this, cx| {
                if this.config.theme == Theme::System && this.system_dark != dark {
                    this.system_dark = dark;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn clipboard_guard(&self) -> ClipboardGuard {
        self.clipboard.clone()
    }

    pub fn clear_on_exit(&self) -> bool {
        self.config.clear_clipboard_on_exit
    }

    fn palette(&self) -> &'static Palette {
        theme::palette(self.config.theme, self.system_dark)
    }

    fn show_toast(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.push_toast(text, cx);
    }

    /// Informational toasts obey the notification preference; errors always show.
    fn notify(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        if self.config.notifications {
            self.push_toast(text, cx);
        }
    }

    fn push_toast(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.toast_seq += 1;
        let seq = self.toast_seq;
        self.toast = Some(Toast {
            text: text.into(),
            seq,
        });
        cx.spawn(async move |weak, cx| {
            Timer::after(Duration::from_secs(4)).await;
            let _ = weak.update(cx, |this, cx| {
                if this.toast.as_ref().map(|toast| toast.seq) == Some(seq) {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn save_config(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = self.config.save() {
            self.show_toast(format!("Could not save settings: {err}"), cx);
        }
    }

    // ---------------------------------------------------------------- backend

    fn refresh_vault(&mut self, cx: &mut Context<Self>) {
        self.status = Status::Starting;
        self.details = None;
        self.detail_generation += 1;
        cx.notify();
        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| {
            let result: Result<Option<Vec<ItemMeta>>, VaultError> = async {
                backend.is_installed().await?;
                if !backend.is_unlocked().await? {
                    return Ok(None);
                }
                Ok(Some(backend.list_items().await?))
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(Some(items)) => {
                        this.items = items;
                        this.status = Status::Ready;
                    }
                    Ok(None) => {
                        this.items.clear();
                        this.status = Status::Locked;
                    }
                    Err(err) if err.is_locked() => {
                        this.items.clear();
                        this.status = Status::Locked;
                    }
                    Err(err) => {
                        this.items.clear();
                        this.status = Status::Error(err.user_message());
                    }
                }
                this.recompute();
                cx.notify();
            });
        })
        .detach();
    }

    fn unlock(&mut self, cx: &mut Context<Self>) {
        self.notify("Unlock requested — complete the rbw prompt", cx);
        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| {
            let result = backend.unlock().await;
            let _ = weak.update(cx, |this, cx| match result {
                Ok(()) => this.refresh_vault(cx),
                Err(err) => {
                    if err.is_locked() {
                        this.status = Status::Locked;
                    }
                    this.show_toast(err.user_message(), cx);
                }
            });
        })
        .detach();
    }

    fn lock(&mut self, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| {
            let result = backend.lock().await;
            let _ = weak.update(cx, |this, cx| {
                this.items.clear();
                this.details = None;
                this.detail_generation += 1;
                this.recompute();
                match result {
                    Ok(()) => {
                        this.status = Status::Locked;
                        this.notify("Vault locked", cx);
                    }
                    Err(err) => this.show_toast(err.user_message(), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        self.notify("Syncing…", cx);
        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| {
            let result = backend.sync().await;
            let _ = weak.update(cx, |this, cx| match result {
                Ok(()) => {
                    this.notify("Vault synced", cx);
                    this.refresh_vault(cx);
                }
                Err(err) => this.handle_error(err, cx),
            });
        })
        .detach();
    }

    fn handle_error(&mut self, err: VaultError, cx: &mut Context<Self>) {
        if err.is_locked() {
            self.status = Status::Locked;
            self.details = None;
            self.detail_generation += 1;
            cx.notify();
        }
        self.show_toast(err.user_message(), cx);
    }

    // ---------------------------------------------------------------- search

    fn recompute(&mut self) {
        let previous_id = self.selected_meta().map(|item| item.id.clone());
        let query = self.query.trim().to_string();
        let favorites_first = self.config.favorites_first;
        let mut scored: Vec<(i32, bool, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| self.filter.accepts(item, &self.favorites))
            .filter_map(|(index, item)| {
                search::item_score(&query, item)
                    .map(|score| (score, self.favorites.contains(&item.id), index))
            })
            .collect();
        scored.sort_by(|a, b| {
            if favorites_first && a.1 != b.1 {
                return b.1.cmp(&a.1);
            }
            b.0.cmp(&a.0).then_with(|| {
                self.items[a.2]
                    .name
                    .to_lowercase()
                    .cmp(&self.items[b.2].name.to_lowercase())
            })
        });
        self.filtered = scored.into_iter().map(|(_, _, index)| index).collect();

        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = previous_id
            .and_then(|id| {
                self.filtered
                    .iter()
                    .position(|index| self.items[*index].id == id)
            })
            .unwrap_or(0)
            .min(self.filtered.len() - 1);
        self.list_scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
    }

    fn selected_meta(&self) -> Option<&ItemMeta> {
        self.filtered
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let next = (self.selected as isize + delta).rem_euclid(len);
        self.selected = next as usize;
        self.list_scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        self.details = None;
        self.detail_generation += 1;
        cx.notify();
    }

    fn select_row(&mut self, position: usize, cx: &mut Context<Self>) {
        if position >= self.filtered.len() {
            return;
        }
        self.selected = position;
        self.list_scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        self.open_selected(cx);
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.selected = 0;
        self.details = None;
        self.detail_generation += 1;
        self.recompute();
        cx.notify();
    }

    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.selected_meta() else {
            return;
        };
        let id = item.id.clone();
        let added = if self.favorites.contains(&id) {
            self.favorites.remove(&id);
            false
        } else {
            self.favorites.insert(id);
            true
        };
        let favorites: Vec<String> = self.favorites.iter().cloned().collect();
        if let Err(err) = config::save_favorites(&favorites) {
            self.show_toast(format!("Could not save favorites: {err}"), cx);
        }
        self.recompute();
        self.notify(
            if added {
                "Added to favorites"
            } else {
                "Removed from favorites"
            },
            cx,
        );
    }

    // --------------------------------------------------------------- details

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let Some(meta) = self.selected_meta().cloned() else {
            return;
        };
        self.detail_generation += 1;
        let generation = self.detail_generation;
        self.details = None;
        self.details_loading = true;
        self.show_password = false;
        self.totp = None;
        cx.notify();

        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| {
            let result = backend.get_item(meta.id.clone()).await;
            let _ = weak.update(cx, |this, cx| {
                if this.detail_generation != generation {
                    return;
                }
                this.details_loading = false;
                match result {
                    Ok(mut details) => {
                        if details.id.is_empty() {
                            details.id = meta.id.clone();
                        }
                        let has_totp = details.has_totp;
                        let id = details.id.clone();
                        this.details = Some(details);
                        if has_totp {
                            this.start_totp_loop(id, cx);
                        }
                    }
                    Err(err) => this.handle_error(err, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_details(&mut self, cx: &mut Context<Self>) {
        self.details = None;
        self.details_loading = false;
        self.detail_generation += 1;
        self.totp = None;
        self.show_password = false;
        cx.notify();
    }

    fn start_totp_loop(&mut self, id: String, cx: &mut Context<Self>) {
        let generation = self.detail_generation;
        self.totp = None;
        self.totp_remaining = remaining_totp_seconds();
        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| loop {
            let result = backend.get_totp(id.clone()).await;
            let alive = weak
                .update(cx, |this, cx| {
                    if this.detail_generation != generation || this.details.is_none() {
                        return false;
                    }
                    match result {
                        Ok(code) => this.totp = Some(Zeroizing::new(code)),
                        Err(_) => this.totp = None,
                    }
                    this.totp_remaining = remaining_totp_seconds();
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !alive {
                return;
            }
            loop {
                Timer::after(Duration::from_secs(1)).await;
                let remaining = remaining_totp_seconds();
                let alive = weak
                    .update(cx, |this, cx| {
                        if this.detail_generation != generation || this.details.is_none() {
                            return false;
                        }
                        this.totp_remaining = remaining;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    return;
                }
                if remaining == 0 {
                    break;
                }
            }
        })
        .detach();
    }

    // -------------------------------------------------------------- clipboard

    fn copy_field(&mut self, field: CopyField, cx: &mut Context<Self>) {
        if self.screen != Screen::Vault {
            return;
        }
        let Some(meta) = self.selected_meta().cloned() else {
            self.show_toast("No entry selected", cx);
            return;
        };
        match field {
            CopyField::Username => {
                if let Some(username) = meta.username.clone() {
                    self.copy_value(Zeroizing::new(username), "Username copied", cx);
                } else {
                    self.copy_fetched(meta.id, field, cx);
                }
            }
            CopyField::Totp => {
                if let Some(code) = self.totp.as_ref() {
                    self.copy_value(Zeroizing::new(code.to_string()), "TOTP copied", cx);
                } else {
                    self.copy_fetched(meta.id, field, cx);
                }
            }
            CopyField::Password => self.copy_fetched(meta.id, field, cx),
        }
    }

    fn copy_fetched(&mut self, id: String, field: CopyField, cx: &mut Context<Self>) {
        let backend = self.backend.clone();
        cx.spawn(async move |weak, cx| {
            let value: Result<Zeroizing<String>, VaultError> = match field {
                CopyField::Password => backend.get_item(id.clone()).await.map(|item| {
                    Zeroizing::new(
                        item.password
                            .as_ref()
                            .map(|password| password.expose().to_string())
                            .unwrap_or_default(),
                    )
                }),
                CopyField::Username => backend
                    .get_item(id.clone())
                    .await
                    .map(|item| Zeroizing::new(item.username.clone().unwrap_or_default())),
                CopyField::Totp => backend.get_totp(id.clone()).await.map(Zeroizing::new),
            };
            let _ = weak.update(cx, |this, cx| match value {
                Ok(value) if !value.is_empty() => {
                    let label = match field {
                        CopyField::Username => "Username copied",
                        CopyField::Password => "Password copied",
                        CopyField::Totp => "TOTP copied",
                    };
                    this.copy_value(value, label, cx);
                }
                Ok(_) => this.show_toast(
                    format!("No {} for this entry", field.label().to_lowercase()),
                    cx,
                ),
                Err(err) => this.handle_error(err, cx),
            });
        })
        .detach();
    }

    fn copy_value(&mut self, value: Zeroizing<String>, label: &str, cx: &mut Context<Self>) {
        if value.is_empty() {
            self.show_toast("Nothing to copy", cx);
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
        self.clipboard.remember(value.as_str());
        self.notify(
            format!(
                "{label} — clears in {}s",
                self.config.clipboard_timeout_secs
            ),
            cx,
        );

        let timeout = self.config.clipboard_timeout_secs;
        if timeout == 0 {
            self.clipboard.forget();
            return;
        }
        let expected = value;
        cx.spawn(async move |weak, cx| {
            Timer::after(Duration::from_secs(timeout)).await;
            let _ = weak.update(cx, |this, cx| {
                let current = cx.read_from_clipboard().and_then(|item| item.text());
                this.clipboard.forget_if_matches(expected.as_str());
                if should_clear(current.as_deref(), expected.as_str()) {
                    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
                    this.notify("Clipboard cleared", cx);
                }
            });
        })
        .detach();
    }

    // ------------------------------------------------------------------ input

    fn shortcut_matches(shortcut: &str, keystroke: &Keystroke) -> bool {
        Keystroke::parse(shortcut)
            .map(|parsed| parsed.key == keystroke.key && parsed.modifiers == keystroke.modifiers)
            .unwrap_or(false)
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let shortcuts = self.config.shortcuts.clone();

        if modifiers.control || modifiers.alt || modifiers.platform {
            if Self::shortcut_matches(&shortcuts.focus_search, keystroke)
                || (modifiers.control && keystroke.key == "f")
            {
                self.screen = Screen::Vault;
                window.focus(&self.focus);
                self.query.clear();
                self.recompute();
                cx.notify();
            } else if Self::shortcut_matches(&shortcuts.copy_default, keystroke) {
                self.copy_field(self.config.default_copy_field, cx);
            } else if Self::shortcut_matches(&shortcuts.copy_password, keystroke) {
                self.copy_field(CopyField::Password, cx);
            } else if Self::shortcut_matches(&shortcuts.copy_username, keystroke) {
                self.copy_field(CopyField::Username, cx);
            } else if Self::shortcut_matches(&shortcuts.copy_totp, keystroke) {
                self.copy_field(CopyField::Totp, cx);
            } else if Self::shortcut_matches(&shortcuts.sync, keystroke) {
                self.sync(cx);
            } else if Self::shortcut_matches(&shortcuts.generator, keystroke) {
                if self.screen == Screen::Generator {
                    self.screen = Screen::Vault;
                } else {
                    self.screen = Screen::Generator;
                    self.generate_password(cx);
                }
                cx.notify();
            } else if Self::shortcut_matches(&shortcuts.toggle_favorite, keystroke) {
                self.toggle_favorite(cx);
            } else if modifiers.control && keystroke.key == "," {
                self.screen = if self.screen == Screen::Settings {
                    Screen::Vault
                } else {
                    Screen::Settings
                };
                cx.notify();
            }
            return;
        }

        match keystroke.key.as_str() {
            "escape" => {
                if self.screen != Screen::Vault {
                    self.screen = Screen::Vault;
                } else if self.details.is_some() {
                    self.close_details(cx);
                } else {
                    self.query.clear();
                    self.recompute();
                }
                cx.notify();
            }
            "backspace" => {
                self.query.pop();
                self.recompute();
                cx.notify();
            }
            "up" => self.move_selection(-1, cx),
            "down" => self.move_selection(1, cx),
            "enter" => {
                if matches!(self.status, Status::Locked) {
                    self.unlock(cx);
                } else if self.screen == Screen::Vault {
                    self.open_selected(cx);
                }
            }
            "tab" => self.move_selection(1, cx),
            " " => self.push_text(" ", cx),
            _ => {
                if let Some(text) = &keystroke.key_char {
                    if !text.is_empty() && !text.chars().any(char::is_control) {
                        self.push_text(text, cx);
                    }
                }
            }
        }
    }

    fn push_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.screen != Screen::Vault || self.query.len() >= 512 {
            return;
        }
        self.query.push_str(text);
        self.filter = Filter::All;
        self.recompute();
        cx.notify();
    }

    // ------------------------------------------------------------- generator

    fn generate_password(&mut self, cx: &mut Context<Self>) {
        match password::generate(&self.config.generator) {
            Ok(generated) => self.generated = Some(Zeroizing::new(generated)),
            Err(err) => self.show_toast(err, cx),
        }
        cx.notify();
    }

    fn toggle_generator_option(&mut self, which: u8, cx: &mut Context<Self>) {
        let options = &mut self.config.generator;
        match which {
            0 => options.uppercase = !options.uppercase,
            1 => options.lowercase = !options.lowercase,
            2 => options.numbers = !options.numbers,
            3 => options.symbols = !options.symbols,
            4 => options.exclude_ambiguous = !options.exclude_ambiguous,
            _ => options.passphrase = !options.passphrase,
        }
        self.save_config(cx);
        self.generate_password(cx);
    }

    // ------------------------------------------------------------------ views

    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let (status_text, status_color) = match &self.status {
            Status::Starting => ("Connecting…", p.warning),
            Status::Locked => ("Locked", p.warning),
            Status::Ready => ("Synced", p.success),
            Status::Error(_) => ("rbw unavailable", p.danger),
        };
        let locked = matches!(self.status, Status::Locked);
        let errored = matches!(self.status, Status::Error(_));
        let query_text = if self.query.is_empty() {
            SharedString::from("Search vault")
        } else {
            SharedString::from(format!("{}▏", self.query))
        };

        let search = div()
            .id("search")
            .flex()
            .items_center()
            .gap_2()
            .h(px(44.))
            .w(px(560.))
            .px_4()
            .rounded(px(4.))
            .bg(rgba(p.input))
            .border_1()
            .border_color(rgba(p.border_strong))
            .when(self.query.is_empty(), |this| this.text_color(rgba(p.muted)))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_size(px(13.))
                    .child(query_text),
            )
            .child(keycap(p, "Ctrl K"));

        let mut actions = div().flex().items_center().gap_2().child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .mr_2()
                .text_size(px(13.))
                .text_color(rgba(p.text2))
                .child(dot(status_color))
                .child(status_text),
        );
        if errored {
            actions = actions.child(
                primary_icon_button(p, "icons/refresh.svg", "Retry")
                    .id("btn-retry")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_vault(cx))),
            );
        } else if locked {
            actions = actions.child(
                primary_icon_button(p, "icons/unlock.svg", "Unlock")
                    .id("btn-unlock")
                    .on_click(cx.listener(|this, _, _, cx| this.unlock(cx))),
            );
        } else {
            actions = actions
                .child(
                    icon_button(p, "icons/lock.svg", p.muted)
                        .id("btn-lock")
                        .on_click(cx.listener(|this, _, _, cx| this.lock(cx))),
                )
                .child(
                    primary_icon_button(p, "icons/refresh.svg", "Sync")
                        .id("btn-sync")
                        .on_click(cx.listener(|this, _, _, cx| this.sync(cx))),
                );
        }

        div()
            .flex()
            .items_center()
            .gap_5()
            .h(px(72.))
            .px_6()
            .bg(rgba(p.surface))
            .border_b_1()
            .border_color(rgba(p.border_strong))
            .shadow_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .w(px(260.))
                    .child(
                        div()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgba(p.accent))
                            .child(icon("icons/shield.svg", 18., p.accent_ink)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Vault"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgba(p.muted))
                                    .child("Bitwarden · rbw"),
                            ),
                    ),
            )
            .child(search)
            .child(div().flex_1())
            .child(actions)
    }

    fn filter_count(&self, filter: Filter) -> usize {
        self.items
            .iter()
            .filter(|item| filter.accepts(item, &self.favorites))
            .count()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        let mut sidebar = div()
            .flex()
            .flex_col()
            .w(px(280.))
            .h_full()
            .py_3()
            .bg(rgba(p.sidebar))
            .border_r_1()
            .border_color(rgba(p.border_strong));

        sidebar = sidebar.child(
            div()
                .id("account")
                .flex()
                .items_center()
                .gap_2()
                .mx_3()
                .mt_1()
                .mb_4()
                .p_4()
                .rounded_md()
                .bg(rgba(p.card))
                .border_1()
                .border_color(rgba(p.border_strong))
                .shadow_sm()
                .child(
                    div()
                        .size(px(36.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgba(p.accent))
                        .child(icon("icons/shield.svg", 18., p.accent_ink)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(div().text_size(px(15.)).child("Bitwarden"))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgba(p.muted))
                                .child("rbw vault"),
                        ),
                ),
        );

        sidebar = sidebar.child(section_label(p, "Vault"));
        for filter in Filter::ALL {
            let active = self.filter == filter;
            sidebar = sidebar.child(
                sidebar_row(
                    p,
                    filter.icon(),
                    filter.label(),
                    self.filter_count(filter),
                    active,
                )
                .id(SharedString::from(format!(
                    "filter-{}",
                    filter.short_label()
                )))
                .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter, cx))),
            );
        }

        sidebar = sidebar.child(section_label(p, "Tools"));
        sidebar = sidebar
            .child(
                sidebar_row(
                    p,
                    "icons/dice.svg",
                    "Generator",
                    0,
                    self.screen == Screen::Generator,
                )
                .id("side-generator")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.screen = Screen::Generator;
                    this.generate_password(cx);
                    cx.notify();
                })),
            )
            .child(
                sidebar_row(
                    p,
                    "icons/settings.svg",
                    "Settings",
                    0,
                    self.screen == Screen::Settings,
                )
                .id("side-settings")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.screen = Screen::Settings;
                    cx.notify();
                })),
            );

        sidebar.child(div().flex_1())
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let mut container = div().flex().flex_col().flex_1().bg(rgba(p.surface));

        let mut chips: Vec<gpui::AnyElement> = Vec::new();
        for filter in [
            Filter::All,
            Filter::Logins,
            Filter::Notes,
            Filter::Cards,
            Filter::Identities,
        ] {
            chips.push(
                chip(p, filter.short_label(), self.filter == filter)
                    .id(SharedString::from(format!(
                        "list-filter-{}",
                        filter.short_label()
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter, cx)))
                    .into_any_element(),
            );
        }

        container = container.child(
            div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(rgba(p.border_strong))
                .shadow_sm()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .h(px(60.))
                        .px_5()
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(self.filter.label()),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgba(p.muted))
                                .child(format!("{} items", self.filtered.len())),
                        )
                        .child(div().flex_1())
                        .child(
                            icon_button(p, "icons/refresh.svg", p.muted)
                                .id("list-sync")
                                .on_click(cx.listener(|this, _, _, cx| this.sync(cx))),
                        )
                        .child(
                            icon_button(p, "icons/star.svg", p.muted)
                                .id("list-favorite")
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_5()
                        .pb_3()
                        .children(chips),
                ),
        );

        if matches!(self.status, Status::Starting) {
            return container.child(empty_state(p, "Loading vault…"));
        }
        if self.filtered.is_empty() {
            return container.child(empty_state(
                p,
                if self.query.is_empty() {
                    "No entries"
                } else {
                    "No matches"
                },
            ));
        }

        let count = self.filtered.len().min(300);
        let list = uniform_list(
            "item-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                range
                    .map(|position| this.render_row(position, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.list_scroll.clone())
        .flex_1();

        container = container.child(list);
        if self.filtered.len() > 300 {
            container = container.child(empty_state(
                p,
                "Showing first 300 matches — refine your search",
            ));
        }
        container
    }

    fn render_row(&self, position: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let item = &self.items[self.filtered[position]];
        let selected = position == self.selected;
        let favorite = self.favorites.contains(&item.id);
        let domain = item.domain().unwrap_or_default();
        let mut row = div()
            .id(("row", position))
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .px_5()
            .py_3()
            .cursor_pointer()
            .bg(rgba(if selected { p.selected } else { p.surface }))
            .border_b_1()
            .border_color(rgba(p.message_divider))
            .hover(move |style| style.bg(rgba(p.hover)))
            .on_click(cx.listener(move |this, _, _, cx| this.select_row(position, cx)))
            .child(div().w(px(20.)).flex().items_center().child(icon(
                item_type_icon(item.item_type),
                16.,
                if selected { p.accent } else { p.muted },
            )))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(16.))
                            .text_color(rgba(p.text))
                            .child(truncate_text(item.name.clone(), 200)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .text_size(px(13.))
                            .text_color(rgba(p.muted))
                            .child(truncate_text(item.username.clone().unwrap_or_default(), 40))
                            .child(if domain.is_empty() {
                                SharedString::from(item.item_type.label())
                            } else {
                                SharedString::from(domain)
                            }),
                    ),
            );
        if favorite {
            row = row.child(icon("icons/star-filled.svg", 16., p.accent));
        }
        row
    }

    fn render_details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let favorite = self
            .selected_meta()
            .map(|item| self.favorites.contains(&item.id))
            .unwrap_or(false);

        let mut panel = div()
            .flex()
            .flex_col()
            .w(px(520.))
            .h_full()
            .bg(rgba(p.surface))
            .border_l_1()
            .border_color(rgba(p.border_strong));

        if self.details_loading {
            return panel.child(
                div()
                    .p_6()
                    .text_size(px(15.))
                    .text_color(rgba(p.muted))
                    .child("Loading entry…"),
            );
        }
        let Some(details) = &self.details else {
            return panel;
        };

        let mut meta: Vec<String> = Vec::new();
        if let Some(username) = &details.username {
            meta.push(username.clone());
        }
        if let Some(domain) = details.uris.first().map(|uri| crate::model::domain_of(uri)) {
            if !domain.is_empty() {
                meta.push(domain);
            }
        }
        meta.push(details.item_type.label().to_string());
        let meta_line = meta.join("  ·  ");

        let has_totp = details.has_totp;
        let mut toolbar = div()
            .flex()
            .items_center()
            .gap_1()
            .h(px(48.))
            .px_3()
            .border_b_1()
            .border_color(rgba(p.border))
            .child(
                icon_button(
                    p,
                    if favorite {
                        "icons/star-filled.svg"
                    } else {
                        "icons/star.svg"
                    },
                    if favorite { p.accent } else { p.muted },
                )
                .id("details-favorite")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_favorite(cx))),
            )
            .child(
                icon_button(p, "icons/user.svg", p.muted)
                    .id("details-user")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.copy_field(CopyField::Username, cx)),
                    ),
            )
            .child(
                icon_button(p, "icons/key.svg", p.muted)
                    .id("details-pass")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.copy_field(CopyField::Password, cx)),
                    ),
            );
        if has_totp {
            toolbar = toolbar.child(
                icon_button(p, "icons/clock.svg", p.muted)
                    .id("details-totp")
                    .on_click(cx.listener(|this, _, _, cx| this.copy_field(CopyField::Totp, cx))),
            );
        }
        toolbar = toolbar.child(div().flex_1()).child(
            icon_button(p, "icons/close.svg", p.muted)
                .id("details-close")
                .on_click(cx.listener(|this, _, _, cx| this.close_details(cx))),
        );

        let mut body = div()
            .id("details-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .overflow_hidden()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_6()
                    .pt_5()
                    .pb_4()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(truncate_text(details.name.clone(), 200)),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgba(p.muted))
                            .child(truncate_text(meta_line, 300)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(tag(p, details.item_type.label()))
                            .when(has_totp, |this| this.child(tag(p, "TOTP")))
                            .when(favorite, |this| this.child(tag(p, "Favorite"))),
                    ),
            );

        if details.item_type == ItemType::Login || details.username.is_some() {
            body = body.child(detail_field(
                p,
                "Username",
                div()
                    .text_size(px(13.))
                    .child(details.username.clone().unwrap_or_else(|| "—".into())),
                copy_action(
                    p,
                    "details-user-copy",
                    cx.listener(|this, _, _, cx| this.copy_field(CopyField::Username, cx)),
                ),
            ));
        }

        if details.item_type == ItemType::Login || details.password.is_some() {
            let password_text = match &details.password {
                Some(password) if self.show_password => password.expose().to_string(),
                Some(password) => password.masked(),
                None => "—".into(),
            };
            body = body.child(detail_field(
                p,
                "Password",
                div()
                    .text_size(px(16.))
                    .truncate()
                    .child(truncate_text(password_text, 200)),
                div()
                    .flex()
                    .gap_1()
                    .child(
                        icon_button(
                            p,
                            if self.show_password {
                                "icons/eye-off.svg"
                            } else {
                                "icons/eye.svg"
                            },
                            p.muted,
                        )
                        .id("details-password-toggle")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_password = !this.show_password;
                            cx.notify();
                        })),
                    )
                    .child(copy_action(
                        p,
                        "details-password-copy",
                        cx.listener(|this, _, _, cx| this.copy_field(CopyField::Password, cx)),
                    )),
            ));
        }

        if !details.uris.is_empty() {
            let mut uris = div().flex().flex_col().gap_1();
            for uri in &details.uris {
                uris = uris.child(
                    div()
                        .truncate()
                        .text_size(px(15.))
                        .text_color(rgba(p.accent_text))
                        .child(truncate_text(uri.clone(), 300)),
                );
            }
            body = body.child(detail_field(p, "Website", uris, div()));
        }

        if has_totp {
            let code = self
                .totp
                .as_ref()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "…".into());
            body = body.child(detail_field(
                p,
                "TOTP",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_size(px(22.)).child(code))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgba(p.muted))
                            .child(format!("{}s", self.totp_remaining)),
                    ),
                copy_action(
                    p,
                    "details-totp-copy",
                    cx.listener(|this, _, _, cx| this.copy_field(CopyField::Totp, cx)),
                ),
            ));
        }

        if let Some(notes) = &details.notes {
            body = body.child(detail_field(
                p,
                "Notes",
                div()
                    .whitespace_normal()
                    .text_size(px(15.))
                    .text_color(rgba(p.text2))
                    .child(truncate_text(notes.clone(), 20_000)),
                icon_button(p, "icons/copy.svg", p.muted)
                    .id("details-notes-copy")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(notes) = this
                            .details
                            .as_ref()
                            .and_then(|details| details.notes.clone())
                        {
                            this.copy_value(Zeroizing::new(notes), "Notes copied", cx);
                        }
                    })),
            ));
        }

        panel = panel.child(toolbar).child(body);
        panel
    }

    fn render_vault(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette();
        match &self.status {
            Status::Error(message) => centered_state(
                p,
                "Vault unavailable",
                message.clone(),
                primary_icon_button(p, "icons/refresh.svg", "Retry")
                    .id("retry")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_vault(cx))),
                p.danger,
            ),
            Status::Locked => centered_state(
                p,
                "Vault is locked",
                "Unlock with rbw. Your master password is never stored by Vault.".into(),
                primary_icon_button(p, "icons/unlock.svg", "Unlock")
                    .id("unlock")
                    .on_click(cx.listener(|this, _, _, cx| this.unlock(cx))),
                p.warning,
            ),
            _ => {
                let mut main = div()
                    .flex()
                    .flex_1()
                    .child(self.render_sidebar(cx))
                    .child(self.render_list(cx));
                if self.details.is_some() || self.details_loading {
                    main = main.child(self.render_details(cx));
                }
                main
            }
        }
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();

        let appearance = settings_group(
            p,
            "Appearance",
            vec![
                setting_row(
                    p,
                    "Theme",
                    chip(p, format!("{:?}", self.config.theme), false)
                        .id("theme")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.config.theme = match this.config.theme {
                                Theme::System => Theme::Dark,
                                Theme::Dark => Theme::Light,
                                Theme::Light => Theme::System,
                            };
                            this.save_config(cx);
                            cx.notify();
                        })),
                ),
                setting_row(
                    p,
                    "Favorites first",
                    chip(
                        p,
                        on_off(self.config.favorites_first),
                        self.config.favorites_first,
                    )
                    .id("favorites-first")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.favorites_first = !this.config.favorites_first;
                        this.save_config(cx);
                        this.recompute();
                        cx.notify();
                    })),
                ),
                setting_row(
                    p,
                    "Notifications",
                    chip(
                        p,
                        on_off(self.config.notifications),
                        self.config.notifications,
                    )
                    .id("notifications")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.notifications = !this.config.notifications;
                        this.save_config(cx);
                        cx.notify();
                    })),
                ),
            ],
        );

        let clipboard = settings_group(
            p,
            "Clipboard",
            vec![
                setting_row(
                    p,
                    "Clear after",
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            icon_button(p, "icons/minus.svg", p.muted)
                                .id("timeout-down")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.config.clipboard_timeout_secs =
                                        this.config.clipboard_timeout_secs.saturating_sub(5);
                                    this.save_config(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .w(px(44.))
                                .text_size(px(12.))
                                .text_center()
                                .child(format!("{}s", self.config.clipboard_timeout_secs)),
                        )
                        .child(
                            icon_button(p, "icons/plus.svg", p.muted)
                                .id("timeout-up")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.config.clipboard_timeout_secs =
                                        (this.config.clipboard_timeout_secs + 5).min(300);
                                    this.save_config(cx);
                                    cx.notify();
                                })),
                        ),
                ),
                setting_row(
                    p,
                    "Clear on exit",
                    chip(
                        p,
                        on_off(self.config.clear_clipboard_on_exit),
                        self.config.clear_clipboard_on_exit,
                    )
                    .id("clear-exit")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.clear_clipboard_on_exit = !this.config.clear_clipboard_on_exit;
                        this.save_config(cx);
                        cx.notify();
                    })),
                ),
            ],
        );

        let vault = settings_group(
            p,
            "Vault",
            vec![
                setting_row(
                    p,
                    "Default copy field",
                    chip(p, self.config.default_copy_field.label(), false)
                        .id("default-field")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.config.default_copy_field = this.config.default_copy_field.next();
                            this.save_config(cx);
                            cx.notify();
                        })),
                ),
                setting_row(
                    p,
                    "rbw executable",
                    div()
                        .text_size(px(11.))
                        .text_color(rgba(p.muted))
                        .child(self.config.rbw_path.clone()),
                ),
                setting_row(
                    p,
                    "Launch at login",
                    chip(
                        p,
                        on_off(self.config.launch_at_login),
                        self.config.launch_at_login,
                    )
                    .id("launch-login")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.launch_at_login = !this.config.launch_at_login;
                        if let Err(err) = config::set_autostart(this.config.launch_at_login) {
                            this.show_toast(err, cx);
                        }
                        this.save_config(cx);
                        cx.notify();
                    })),
                ),
                setting_row(
                    p,
                    "Start minimized",
                    chip(
                        p,
                        on_off(self.config.start_minimized),
                        self.config.start_minimized,
                    )
                    .id("start-minimized")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.config.start_minimized = !this.config.start_minimized;
                        this.save_config(cx);
                        cx.notify();
                    })),
                ),
            ],
        );

        let shortcuts = &self.config.shortcuts;
        let keyboard = settings_group(
            p,
            "Keyboard",
            vec![
                shortcut_row(p, "Search", &shortcuts.focus_search),
                shortcut_row(p, "Copy default", &shortcuts.copy_default),
                shortcut_row(p, "Copy password", &shortcuts.copy_password),
                shortcut_row(p, "Copy username", &shortcuts.copy_username),
                shortcut_row(p, "Copy TOTP", &shortcuts.copy_totp),
                shortcut_row(p, "Sync", &shortcuts.sync),
                shortcut_row(p, "Generator", &shortcuts.generator),
            ],
        );

        div()
            .id("settings")
            .flex()
            .flex_col()
            .flex_1()
            .p_10()
            .overflow_hidden()
            .overflow_y_scroll()
            .bg(rgba(p.bg))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .max_w(px(840.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(26.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Settings"),
                            )
                            .child(div().text_size(px(12.)).text_color(rgba(p.muted)).child(
                                "Preferences live in config.toml. Vault data is never persisted.",
                            )),
                    )
                    .child(appearance)
                    .child(clipboard)
                    .child(vault)
                    .child(keyboard)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgba(p.muted))
                            .child(format!("Config · {}", config::config_path().display())),
                    )
                    .child(settings_note(p)),
            )
    }

    fn render_generator(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let options = &self.config.generator;

        let mut general = vec![];
        if !options.passphrase {
            general.push(setting_row(
                p,
                "Length",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        icon_button(p, "icons/minus.svg", p.muted)
                            .id("length-down")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.generator.length =
                                    this.config.generator.length.saturating_sub(1).max(8);
                                this.save_config(cx);
                                this.generate_password(cx);
                            })),
                    )
                    .child(
                        div()
                            .w(px(40.))
                            .text_size(px(12.))
                            .text_center()
                            .child(options.length.to_string()),
                    )
                    .child(
                        icon_button(p, "icons/plus.svg", p.muted)
                            .id("length-up")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.generator.length =
                                    (this.config.generator.length + 1).min(256);
                                this.save_config(cx);
                                this.generate_password(cx);
                            })),
                    ),
            ));
        }

        let toggles: [(&str, u8, bool); 6] = [
            ("Uppercase", 0, options.uppercase),
            ("Lowercase", 1, options.lowercase),
            ("Numbers", 2, options.numbers),
            ("Symbols", 3, options.symbols),
            ("Exclude ambiguous", 4, options.exclude_ambiguous),
            ("Passphrase", 5, options.passphrase),
        ];
        for (label, which, active) in toggles {
            general.push(setting_row(
                p,
                label,
                chip(p, on_off(active), active)
                    .id(SharedString::from(format!("generator-{which}")))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_generator_option(which, cx)),
                    ),
            ));
        }

        if options.passphrase {
            general.push(setting_row(
                p,
                "Words",
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        icon_button(p, "icons/minus.svg", p.muted)
                            .id("words-down")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.generator.words =
                                    this.config.generator.words.saturating_sub(1).max(3);
                                this.save_config(cx);
                                this.generate_password(cx);
                            })),
                    )
                    .child(
                        div()
                            .w(px(40.))
                            .text_size(px(12.))
                            .text_center()
                            .child(options.words.to_string()),
                    )
                    .child(
                        icon_button(p, "icons/plus.svg", p.muted)
                            .id("words-up")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.config.generator.words =
                                    (this.config.generator.words + 1).min(24);
                                this.save_config(cx);
                                this.generate_password(cx);
                            })),
                    ),
            ));
        }

        div()
            .id("generator")
            .flex()
            .flex_col()
            .flex_1()
            .p_10()
            .overflow_hidden()
            .overflow_y_scroll()
            .bg(rgba(p.bg))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .max_w(px(800.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(26.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Password generator"),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(rgba(p.text2))
                                    .child("Cryptographically secure random values"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .p_5()
                            .rounded_md()
                            .bg(rgba(p.card))
                            .border_1()
                            .border_color(rgba(p.border_strong))
                            .shadow_sm()
                            .child(
                                div().flex_1().truncate().text_size(px(22.)).child(
                                    self.generated
                                        .as_ref()
                                        .map(|value| value.to_string())
                                        .unwrap_or_else(|| "—".into()),
                                ),
                            )
                            .child(
                                primary_icon_button(p, "icons/copy.svg", "Copy")
                                    .id("generator-copy")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(value) = this.generated.as_ref() {
                                            this.copy_value(
                                                Zeroizing::new(value.to_string()),
                                                "Generated password copied",
                                                cx,
                                            );
                                        }
                                    })),
                            ),
                    )
                    .child(settings_group(p, "Options", general))
                    .child(
                        primary_icon_button(p, "icons/refresh.svg", "Regenerate")
                            .id("regenerate")
                            .w(px(180.))
                            .on_click(cx.listener(|this, _, _, cx| this.generate_password(cx))),
                    ),
            )
    }

    fn render_toast(&self) -> Option<Div> {
        let toast = self.toast.as_ref()?;
        let p = self.palette();
        Some(
            div()
                .px_4()
                .py_2()
                .bg(rgba(p.card))
                .border_t_1()
                .border_color(rgba(p.border))
                .text_size(px(11.))
                .text_color(rgba(p.text2))
                .child(toast.text.clone()),
        )
    }

    fn render_footer(&self) -> Div {
        let p = self.palette();
        let (label, color) = match &self.status {
            Status::Starting => ("Connecting…", p.warning),
            Status::Locked => ("Locked", p.warning),
            Status::Ready => ("Unlocked", p.success),
            Status::Error(_) => ("rbw unavailable", p.danger),
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .h(px(36.))
            .px_5()
            .bg(rgba(p.sidebar))
            .border_t_1()
            .border_color(rgba(p.border_strong))
            .text_size(px(12.))
            .text_color(rgba(p.muted))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(dot(color))
                    .child("rbw")
                    .child("·")
                    .child(label),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(format!("{} items", self.items.len()))
                    .child("·")
                    .child(format!("Clipboard {}s", self.config.clipboard_timeout_secs))
                    .child("·")
                    .child(theme::name(self.config.theme, self.system_dark)),
            )
    }
}

impl Focusable for VaultApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for VaultApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let p = self.palette();
        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgba(p.bg))
            .text_color(rgba(p.text))
            .text_size(px(15.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(self.render_header(cx));

        root = root.child(match self.screen {
            Screen::Vault => self.render_vault(cx).into_any_element(),
            Screen::Settings => self.render_settings(cx).into_any_element(),
            Screen::Generator => self.render_generator(cx).into_any_element(),
        });

        if let Some(toast) = self.render_toast() {
            root = root.child(toast);
        }
        root.child(self.render_footer())
    }
}

fn chip(p: &Palette, label: impl Into<SharedString>, active: bool) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(34.))
        .px_3()
        .rounded(px(4.))
        .border_1()
        .text_size(px(13.))
        .cursor_pointer()
        .when(active, |this| {
            this.bg(rgba(p.strong))
                .border_color(rgba(p.border_strong))
                .text_color(rgba(p.text))
        })
        .when(!active, |this| {
            this.bg(rgba(p.surface))
                .border_color(rgba(p.border))
                .text_color(rgba(p.text2))
                .hover(move |style| style.bg(rgba(p.hover)))
        })
        .child(label.into())
}

fn icon(path: &'static str, size: f32, color: u32) -> impl IntoElement {
    gpui::svg()
        .path(path)
        .size(px(size))
        .text_color(rgba(color))
}

fn icon_button(p: &Palette, path: &'static str, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(36.))
        .rounded(px(4.))
        .bg(rgba(p.surface))
        .border_1()
        .border_color(rgba(p.border))
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(p.hover)))
        .child(icon(path, 18., color))
}

fn primary_icon_button(p: &Palette, path: &'static str, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .h(px(40.))
        .px_5()
        .rounded(px(4.))
        .bg(rgba(p.accent_button))
        .text_color(rgba(p.accent_ink))
        .text_size(px(14.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .shadow_sm()
        .hover(move |style| style.bg(rgba(p.accent_hover)))
        .child(icon(path, 18., p.accent_ink))
        .child(label.into())
}

fn keycap(p: &Palette, label: &'static str) -> Div {
    div()
        .px_2()
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgba(p.surface))
        .border_1()
        .border_color(rgba(p.border_strong))
        .text_size(px(11.))
        .text_color(rgba(p.text2))
        .child(label)
}

fn dot(color: u32) -> Div {
    div().size(px(6.)).rounded_full().bg(rgba(color))
}

fn section_label(p: &Palette, label: &'static str) -> Div {
    div()
        .px_3()
        .pt_5()
        .pb_2()
        .text_size(px(11.))
        .text_color(rgba(p.muted))
        .child(label)
}

fn sidebar_row(
    p: &Palette,
    icon_path: &'static str,
    label: &'static str,
    count: usize,
    active: bool,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .h(px(40.))
        .px_3()
        .rounded(px(4.))
        .w_full()
        .cursor_pointer()
        .when(active, |this| {
            this.bg(rgba(p.selected)).text_color(rgba(p.text))
        })
        .when(!active, |this| {
            this.text_color(rgba(p.text2))
                .hover(move |style| style.bg(rgba(p.hover)))
        })
        .child(div().w(px(20.)).flex().items_center().child(icon(
            icon_path,
            15.,
            if active { p.accent } else { p.muted },
        )))
        .child(div().flex_1().truncate().text_size(px(13.)).child(label))
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgba(p.muted))
                .child(if count > 0 {
                    count.to_string()
                } else {
                    String::new()
                }),
        )
}

fn tag(p: &Palette, label: &str) -> Div {
    div()
        .px_2()
        .py(px(1.))
        .rounded(px(4.))
        .bg(rgba(p.strong))
        .border_1()
        .border_color(rgba(p.border_strong))
        .text_size(px(12.))
        .text_color(rgba(p.text2))
        .child(label.to_string())
}

fn detail_field(
    p: &Palette,
    label: &'static str,
    value: impl IntoElement,
    action: impl IntoElement,
) -> Div {
    div()
        .flex()
        .items_start()
        .justify_between()
        .gap_4()
        .mx_4()
        .mb_3()
        .p_4()
        .rounded_md()
        .bg(rgba(p.field))
        .border_1()
        .border_color(rgba(p.border))
        .shadow_sm()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .flex_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgba(p.muted))
                        .child(label),
                )
                .child(value),
        )
        .child(action)
}

fn copy_action(
    p: &Palette,
    id: &'static str,
    listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    icon_button(p, "icons/copy.svg", p.muted)
        .id(id)
        .on_click(listener)
}

fn settings_group(p: &Palette, title: &'static str, rows: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded_md()
        .border_1()
        .border_color(rgba(p.border))
        .bg(rgba(p.card))
        .overflow_hidden()
        .child(
            div()
                .px_4()
                .py_2()
                .text_size(px(11.))
                .text_color(rgba(p.muted))
                .child(title),
        )
        .children(rows)
}

fn setting_row(p: &Palette, label: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .h(px(56.))
        .px_5()
        .border_t_1()
        .border_color(rgba(p.message_divider))
        .child(div().text_size(px(15.)).child(label))
        .child(control)
}

fn shortcut_row(p: &Palette, label: &'static str, value: &str) -> Div {
    setting_row(
        p,
        label,
        div()
            .text_size(px(13.))
            .text_color(rgba(p.muted))
            .child(value.to_string()),
    )
}

fn centered_state(
    p: &Palette,
    title: &'static str,
    message: String,
    action: impl IntoElement,
    accent: u32,
) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .p_8()
        .bg(rgba(p.bg))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .w(px(480.))
                .p_8()
                .rounded_lg()
                .bg(rgba(p.card))
                .border_1()
                .border_color(rgba(p.border_strong))
                .shadow_md()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(dot(accent))
                        .child(
                            div()
                                .text_size(px(24.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(title),
                        ),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(rgba(p.muted))
                        .child(message),
                )
                .child(div().flex().child(action)),
        )
}

fn empty_state(p: &Palette, text: &'static str) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .p_8()
        .text_size(px(15.))
        .text_color(rgba(p.muted))
        .child(text)
}

fn settings_note(p: &Palette) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgba(p.muted))
        .child(
            "Clipboard clearing is best-effort: clipboard managers and receiving applications may retain copies. Vault only clears the clipboard when it still holds the exact value Vault copied.",
        )
}

fn item_type_icon(item_type: ItemType) -> &'static str {
    match item_type {
        ItemType::Login => "icons/key.svg",
        ItemType::SecureNote => "icons/note.svg",
        ItemType::Card => "icons/card.svg",
        ItemType::Identity => "icons/identity.svg",
        ItemType::Other => "icons/key.svg",
    }
}

fn on_off(value: bool) -> &'static str {
    if value {
        "On"
    } else {
        "Off"
    }
}

fn truncate_text(text: String, max: usize) -> SharedString {
    if text.chars().count() <= max {
        text.into()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>()).into()
    }
}

fn remaining_totp_seconds() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    (TOTP_STEP - (now % TOTP_STEP)) % TOTP_STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_remaining_is_in_range() {
        assert!(remaining_totp_seconds() < TOTP_STEP);
    }

    #[test]
    fn truncation_keeps_short_strings() {
        assert_eq!(
            truncate_text("short".into(), 10),
            SharedString::from("short")
        );
        assert_eq!(
            truncate_text("0123456789".into(), 4),
            SharedString::from("0123…")
        );
    }
}
