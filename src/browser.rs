//! CEF side: process-level `App`, the per-browser `Client` and its handlers.
//!
//! Everything runs on the main thread (external message pump), so handlers share state with the
//! egui app through `Rc<RefCell<Shared>>`.

use cef::*;
use eframe::egui;
use std::{cell::RefCell, rc::Rc};

/// Console prefix the focus hook uses to report whether an editable element has focus.
const EDITABLE_MSG: &str = "__less_editable:";

/// Injected into every main frame: reports editable focus changes over `console.log`.
/// An `IFRAME` counts as editable because its inner focus is invisible from the top document.
const FOCUS_HOOK_JS: &str = r#"(() => {
  const report = () => {
    let e = document.activeElement;
    while (e && e.shadowRoot && e.shadowRoot.activeElement) e = e.shadowRoot.activeElement;
    const editable = !!e && (e.isContentEditable || /^(INPUT|TEXTAREA|SELECT|IFRAME)$/.test(e.tagName));
    console.log("__less_editable:" + editable);
  };
  addEventListener("focusin", report, true);
  addEventListener("focusout", () => setTimeout(report), true);
  report();
})()"#;

/// State written by CEF callbacks and read by the egui app each frame.
pub struct Shared {
    /// Latest painted view as RGBA (premultiplied), in physical pixels.
    pub frame: Option<([usize; 2], Vec<u8>)>,
    /// View size in logical points (DIP), what CEF lays out against.
    pub view: (i32, i32),
    pub scale: f32,
    /// New main-frame URL, taken by the UI to refresh the address bar.
    pub url: Option<String>,
    /// Whether an editable element inside the page has focus (from the JS hook).
    pub editable: bool,
    pub ctx: egui::Context,
}

pub type SharedRc = Rc<RefCell<Shared>>;

wrap_app! {
    pub struct LessApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(command_line) = command_line {
                // Avoids the macOS keychain prompt for Chromium's "Safe Storage" key in dev builds.
                command_line.append_switch(Some(&"use-mock-keychain".into()));
            }
        }
    }
}

wrap_render_handler! {
    struct LessRender {
        shared: SharedRc,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let (w, h) = self.shared.borrow().view;
                rect.width = w.max(1);
                rect.height = h.max(1);
            }
        }

        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> ::std::os::raw::c_int {
            let Some(info) = info else { return 0 };
            info.device_scale_factor = self.shared.borrow().scale;
            1
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            if type_ != PaintElementType::VIEW || buffer.is_null() || width <= 0 || height <= 0 {
                return;
            }
            let (w, h) = (width as usize, height as usize);
            // SAFETY: CEF hands a w*h*4 BGRA buffer that stays valid for the duration of this call.
            let bgra = unsafe { std::slice::from_raw_parts(buffer, w * h * 4) };
            let mut shared = self.shared.borrow_mut();
            shared.frame = Some(([w, h], bgra_to_rgba(bgra)));
            shared.ctx.request_repaint();
        }
    }
}

wrap_display_handler! {
    struct LessDisplay {
        shared: SharedRc,
    }

    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if let (Some(frame), Some(url)) = (frame, url)
                && frame.is_main() == 1
            {
                let mut shared = self.shared.borrow_mut();
                shared.url = Some(url.to_string());
                shared.editable = false;
                shared.ctx.request_repaint();
            }
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let message = message.map(|m| m.to_string()).unwrap_or_default();
            match message.strip_prefix(EDITABLE_MSG) {
                Some(value) => {
                    self.shared.borrow_mut().editable = value == "true";
                    1
                }
                None => 0,
            }
        }
    }
}

wrap_load_handler! {
    struct LessLoad;

    impl LoadHandler {
        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _http_status_code: ::std::os::raw::c_int) {
            if let Some(frame) = frame
                && frame.is_main() == 1
            {
                frame.execute_java_script(Some(&FOCUS_HOOK_JS.into()), None, 0);
            }
        }
    }
}

wrap_client! {
    struct LessClient {
        render: RenderHandler,
        display: DisplayHandler,
        load: LoadHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.display.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load.clone())
        }
    }
}

/// Creates the single off-screen browser tab, rendering into `shared.frame`.
pub fn create_browser(shared: &SharedRc, url: &str) -> Browser {
    let window_info = WindowInfo {
        windowless_rendering_enabled: 1,
        ..Default::default()
    };
    let settings = BrowserSettings {
        windowless_frame_rate: 60,
        ..Default::default()
    };
    let mut client = LessClient::new(
        LessRender::new(shared.clone()),
        LessDisplay::new(shared.clone()),
        LessLoad::new(),
    );
    browser_host_create_browser_sync(
        Some(&window_info),
        Some(&mut client),
        Some(&url.into()),
        Some(&settings),
        None,
        None,
    )
    .expect("CEF failed to create the browser")
}

/// Swaps CEF's BGRA bytes into the RGBA order egui textures expect.
fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let mut rgba = bgra.to_vec();
    for px in rgba.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    rgba
}

#[cfg(test)]
mod tests {
    #[test]
    fn bgra_swaps_red_and_blue() {
        assert_eq!(super::bgra_to_rgba(&[1, 2, 3, 4, 5, 6, 7, 8]), vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }
}
