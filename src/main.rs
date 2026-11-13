//! less-browser: a keyboard-focused browser shell. One eframe window, one off-screen CEF tab.

mod browser;
mod input;

use browser::{Shared, SharedRc};
use cef::{ImplBrowser, ImplBrowserHost, ImplFrame};
use eframe::EframePumpStatus;
use eframe::egui::{self, Event, EventFilter, Id, Key, MouseWheelUnit, Pos2, Rect, Sense, TextureHandle};
use less_input::PageEngine;
use std::{cell::RefCell, process::ExitCode, rc::Rc, time::Duration};
use winit::event_loop::EventLoop;

const HOME_URL: &str = "https://duckduckgo.com";
const REMOTE_DEBUGGING_PORT: i32 = 9222;
const PUMP_INTERVAL: Duration = Duration::from_millis(16);

/// Boots CEF (subprocess dispatch, then browser-process init with an external message pump)
/// and runs the egui window on a manually pumped winit loop, as the cef-rs OSR example does.
/// `do_message_loop_work` must run outside winit's event handler: on macOS CEF spins the
/// NSApplication run loop, and doing that from inside an egui frame re-enters winit and panics.
fn main() -> ExitCode {
    #[cfg(target_os = "macos")]
    let _loader = {
        let exe = std::env::current_exe().expect("current exe path");
        let loader = cef::library_loader::LibraryLoader::new(&exe, false);
        assert!(loader.load(), "cannot load the CEF framework; run the bundled .app");
        loader
    };
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    let args = cef::args::Args::new();
    let mut app = browser::LessApp::new();
    let code = cef::execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if code >= 0 {
        return ExitCode::from(code as u8);
    }

    let settings = cef::Settings {
        windowless_rendering_enabled: 1,
        external_message_pump: 1,
        remote_debugging_port: REMOTE_DEBUGGING_PORT,
        ..Default::default()
    };
    assert_eq!(
        cef::initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()),
        1,
        "CEF initialize failed"
    );

    let mut event_loop = EventLoop::<eframe::UserEvent>::with_user_event().build().expect("winit event loop");
    let mut window = eframe::create_native(
        "less-browser",
        eframe::NativeOptions::default(),
        Box::new(|cc| Ok(Box::new(BrowserApp::new(&cc.egui_ctx)))),
        &event_loop,
    );
    // winit pumps first so it, not CEF, receives NSApplication's did-finish-launching.
    let code = loop {
        // shortcut: fixed <=16 ms CEF pump, switch to BrowserProcessHandler::on_schedule_message_pump_work when idle CPU matters.
        if let EframePumpStatus::Exit(code) = window.pump_eframe_app(&mut event_loop, Some(PUMP_INTERVAL)) {
            break code;
        }
        cef::do_message_loop_work();
    };
    drop(window);
    // Let the browser finish closing before shutdown.
    for _ in 0..50 {
        cef::do_message_loop_work();
        std::thread::sleep(Duration::from_millis(10));
    }
    cef::shutdown();
    ExitCode::from(code as u8)
}

struct BrowserApp {
    shared: SharedRc,
    browser: cef::Browser,
    address: String,
    texture: Option<TextureHandle>,
    page_focused: bool,
    page_keys: PageEngine,
    /// Last press (time, pos, button, count) to derive CEF click counts.
    last_click: Option<(f64, Pos2, egui::PointerButton, i32)>,
}

impl BrowserApp {
    fn new(ctx: &egui::Context) -> Self {
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let shared = Rc::new(RefCell::new(Shared {
            frame: None,
            view: (800, 600),
            scale: ctx.pixels_per_point(),
            url: None,
            editable: false,
            ctx: ctx.clone(),
        }));
        let browser = browser::create_browser(&shared, HOME_URL);
        Self {
            shared,
            browser,
            address: HOME_URL.into(),
            texture: None,
            page_focused: false,
            page_keys: PageEngine::default(),
            last_click: None,
        }
    }

    fn host(&self) -> Option<cef::BrowserHost> {
        self.browser.host()
    }

    fn run_js(&self, code: &str) {
        if let Some(frame) = self.browser.main_frame() {
            frame.execute_java_script(Some(&code.into()), None, 0);
        }
    }

    /// Keeps CEF's view size and scale in sync with the page rect.
    fn sync_view(&self, rect: Rect, scale: f32) {
        let view = (rect.width().round() as i32, rect.height().round() as i32);
        let mut shared = self.shared.borrow_mut();
        let (resized, rescaled) = (shared.view != view, shared.scale != scale);
        shared.view = view;
        shared.scale = scale;
        drop(shared);
        if let Some(host) = self.host() {
            if rescaled {
                host.notify_screen_info_changed();
            }
            if resized || rescaled {
                host.was_resized();
            }
        }
    }

    /// Uploads the latest CEF paint and draws it at the top-left of `rect`.
    fn paint_page(&mut self, ui: &egui::Ui, rect: Rect) {
        if let Some((size, rgba)) = self.shared.borrow_mut().frame.take() {
            let image = egui::ColorImage::from_rgba_premultiplied(size, &rgba);
            match &mut self.texture {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ui.ctx().load_texture("page", image, egui::TextureOptions::LINEAR)),
            }
        }
        if let Some(t) = &self.texture {
            let size = t.size_vec2() / ui.ctx().pixels_per_point();
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            ui.painter_at(rect).image(t.id(), Rect::from_min_size(rect.min, size), uv, egui::Color32::WHITE);
        }
    }

    /// Forwards this frame's egui events to CEF: mouse always (when over the page or dragging),
    /// keyboard and text only while the page has focus. Vim scroll keys are intercepted unless
    /// an editable element in the page is focused.
    fn forward_input(&mut self, ui: &egui::Ui, rect: Rect) {
        let Some(host) = self.host() else { return };
        let frame = self.browser.main_frame();
        let (events, pointer, time) = ui.input(|i| (i.events.clone(), i.pointer.clone(), i.time));
        let held = [egui::PointerButton::Primary, egui::PointerButton::Middle, egui::PointerButton::Secondary]
            .into_iter()
            .filter(|b| pointer.button_down(*b))
            .filter_map(input::mouse_button)
            .fold(0, |f, (_, bit)| f | bit);
        let over_page = pointer.hover_pos().is_some_and(|p| rect.contains(p));
        let mouse = |pos: Pos2, mods: egui::Modifiers| cef::MouseEvent {
            x: (pos.x - rect.min.x) as i32,
            y: (pos.y - rect.min.y) as i32,
            modifiers: input::flags(mods) | held,
        };
        let editable = self.shared.borrow().editable;
        let mut swallow_text = false;

        for event in events {
            match event {
                Event::PointerMoved(pos) if rect.contains(pos) || held != 0 => {
                    host.send_mouse_move_event(Some(&mouse(pos, pointer_mods(ui))), 0);
                }
                Event::PointerGone => host.send_mouse_move_event(Some(&cef::MouseEvent::default()), 1),
                Event::PointerButton { pos, button, pressed, modifiers } if rect.contains(pos) || !pressed => {
                    let Some((kind, _)) = input::mouse_button(button) else { continue };
                    let count = if pressed { self.click_count(time, pos, button) } else { self.last_click.map_or(1, |c| c.3) };
                    host.send_mouse_click_event(Some(&mouse(pos, modifiers)), kind, (!pressed) as i32, count);
                }
                Event::MouseWheel { unit, delta, modifiers, .. } if over_page => {
                    let delta = match unit {
                        MouseWheelUnit::Point => delta,
                        MouseWheelUnit::Line => delta * 40.0,
                        MouseWheelUnit::Page => delta * rect.height(),
                    };
                    let pos = pointer.hover_pos().unwrap_or(rect.min);
                    host.send_mouse_wheel_event(Some(&mouse(pos, modifiers)), delta.x as i32, delta.y as i32);
                }
                _ if !self.page_focused => {}
                Event::Key { key, physical_key, pressed, modifiers, .. } => {
                    if pressed
                        && !editable
                        && let Some(k) = less_input::egui::to_less_key(key, modifiers)
                    {
                        let action = self.page_keys.feed(k);
                        if let Some(action) = action {
                            self.run_js(&input::scroll_js(action));
                        }
                        if action.is_some() || self.page_keys.is_pending() {
                            swallow_text = matches!(k.key, less_input::Key::Char(_)) && !k.mods.ctrl;
                            continue;
                        }
                    }
                    if pressed && modifiers.mac_cmd
                        && let Some(frame) = &frame
                    {
                        match key {
                            Key::A => frame.select_all(),
                            Key::Z if modifiers.shift => frame.redo(),
                            Key::Z => frame.undo(),
                            _ => {}
                        }
                    }
                    if let Some(e) = input::key_event(key, physical_key, modifiers, pressed) {
                        host.send_key_event(Some(&e));
                    }
                    // Enter only submits forms via a keypress (CHAR); egui emits no Text for it.
                    if pressed && key == Key::Enter {
                        host.send_key_event(Some(&input::char_event(0x0D, modifiers)));
                    }
                }
                Event::Text(text) => {
                    if std::mem::take(&mut swallow_text) {
                        continue;
                    }
                    let mods = pointer_mods(ui);
                    for unit in text.encode_utf16() {
                        host.send_key_event(Some(&input::char_event(unit, mods)));
                    }
                }
                Event::Copy => frame.iter().for_each(|f| f.copy()),
                Event::Cut => frame.iter().for_each(|f| f.cut()),
                Event::Paste(_) => frame.iter().for_each(|f| f.paste()),
                _ => {}
            }
        }
    }

    /// 1 for a fresh press, +1 (up to 3) for a quick press near the previous one.
    fn click_count(&mut self, time: f64, pos: Pos2, button: egui::PointerButton) -> i32 {
        let count = match self.last_click {
            Some((t, p, b, n)) if b == button && time - t < 0.5 && p.distance(pos) < 5.0 => n % 3 + 1,
            _ => 1,
        };
        self.last_click = Some((time, pos, button, count));
        count
    }
}

fn pointer_mods(ui: &egui::Ui) -> egui::Modifiers {
    ui.input(|i| i.modifiers)
}

impl eframe::App for BrowserApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let page_id = Id::unique("page");
        let address_id = Id::unique("address");
        let address_focused = ui.memory(|m| m.has_focus(address_id));
        if !address_focused && let Some(url) = self.shared.borrow_mut().url.take() {
            self.address = url;
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            let edit = egui::TextEdit::singleline(&mut self.address).id(address_id).desired_width(f32::INFINITY);
            let response = ui.add(edit);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                if let (Some(url), Some(frame)) = (normalize_url(&self.address), self.browser.main_frame()) {
                    frame.load_url(Some(&url.as_str().into()));
                    self.address = url;
                }
                ui.memory_mut(|m| m.request_focus(page_id));
            }
        });

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let rect = ui.max_rect();
            let response = ui.interact(rect, page_id, Sense::click());
            if response.is_pointer_button_down_on() || (self.texture.is_none() && !address_focused) {
                response.request_focus();
            }
            let focused = response.has_focus();
            if focused {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(page_id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
                });
            }
            if focused != self.page_focused {
                self.page_focused = focused;
                self.page_keys = PageEngine::default();
                if let Some(host) = self.host() {
                    host.set_focus(focused as i32);
                }
            }
            self.sync_view(rect, ui.ctx().pixels_per_point());
            self.forward_input(ui, rect);
            self.paint_page(ui, rect);
        });
    }

    fn on_exit(&mut self) {
        if let Some(host) = self.host() {
            host.close_browser(1);
        }
    }
}

/// Address bar text -> URL to load: trimmed, `https://` added when there is no scheme.
fn normalize_url(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let has_scheme = s.contains("://") || ["about:", "data:", "file:", "chrome:", "view-source:"].iter().any(|p| s.starts_with(p));
    Some(if has_scheme { s.to_string() } else { format!("https://{s}") })
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn url_normalization() {
        assert_eq!(normalize_url("  example.com "), Some("https://example.com".into()));
        assert_eq!(normalize_url("localhost:3000/x"), Some("https://localhost:3000/x".into()));
        assert_eq!(normalize_url("http://example.com"), Some("http://example.com".into()));
        assert_eq!(normalize_url("about:blank"), Some("about:blank".into()));
        assert_eq!(normalize_url("   "), None);
    }
}
