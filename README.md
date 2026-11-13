# less-browser

Klavye odaklı, içine AI ajanı gömülecek bir tarayıcı kabuğu. Kendi render motoru yok; sayfaları CEF (Chromium) çizer, arayüz egui ile yapılır. Şimdilik yalnızca macOS.

## Mimari

- **CEF off-screen rendering (OSR):** `on_paint` CPU BGRA buffer verir; RGBA'ya çevrilip egui texture olarak çizilir. UI: eframe/egui 0.37.
- **Sürümler:** `cef` crate `=156.4.0` (CEF 156.0.26, Chromium 156.0.8078.88), [tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs).
- **Message loop:** `eframe::create_native` + elle `pump_eframe_app(16ms)` döngüsü; her pump arasında `cef::do_message_loop_work()`. `eframe::run_native` kullanılamıyor: CEF macOS run loop'unu döndürüp winit'e yeniden giriyor ve çöküyor. winit önce pump edilmeli, yoksa CEF app-finished-launching olayını yutuyor.
- **Remote debugging:** port 9222 (ileride MCP "browser" aracı için).

```
src/main.rs                  init, ana döngü, egui uygulaması, URL normalizasyonu
src/browser.rs               CEF handler'ları: render, display, load + focus takibi JS'i
src/input.rs                 egui -> mac keycode, modifier, mouse, scroll_js
src/bin/less-browser-helper.rs   CEF alt süreçleri (GPU/Renderer/...)
src/bin/bundle.rs            cef::build_util::mac::build_bundle ile .app (5 helper app), ad-hoc codesign
```

## scripts/fetch-cef.sh neden var

cef-rs'in `download-cef` 3.0.0 sürümü şu an hata veriyor (`response body is larger than request limit: 10485760`; CDN `index.json` büyüdü). Script minimal dağıtımı indirir, sha1 doğrular ve `.cef/` altına açar (gitignored); `.cargo/config.toml` `CEF_PATH`'i buraya yönlendirir. `cef` crate'i yükseltilirken script'teki `CEF_VERSION` da güncellenmeli.

## Build / çalıştırma

```sh
git clone --recursive <repo>      # ya da: git submodule update --init
./scripts/fetch-cef.sh            # bir kez, ~128MB
cargo test
cargo run --bin bundle            # -> target/bundle/less-browser.app
open target/bundle/less-browser.app
```

Düz `cargo run` çalışmaz; uygulama `.app` içinden çalışmalı.

## Ortak crate'ler

`crates/less-core` ve `crates/less-input` git submodule'dür (github.com/oguzmuay/less-core, less-input). Bağımlılık: `less-input = { path = "crates/less-input", features = ["egui"] }`. Kullanılanlar: `less_input::PageEngine` (j/k, oklar, d/u, Ctrl-d/u, gg/G, sayılar) ve `less_input::egui::to_less_key`. Bu crate'ler LESS editörü tarafından da kullanılıyor; değişiklikleri koordine et. Diğer ortak crate'ler (less-theme, less-ai) var ama henüz kullanılmıyor.

## Yapılanlar

- OSR sayfa çizimi, resize/DPI
- Mouse move/click (multi-click)/wheel/leave
- Klavye iletimi, Türkçe karakterler (egui Text olayları), copy/cut/paste/select-all/undo/redo
- Adres çubuğu: Enter yönlendirir, `https://` ekler, URL'yi gösterir
- `PageEngine` ile Vim scroll; sayfada input/textarea/select/contenteditable/iframe odaktayken devre dışı. PageEngine/to_less_key geçişi tamamlandı.
- Doğrulandı: build, clippy temiz, testler geçiyor, kısa gizli bir açılışta sayfa yüklendi ve 9222'de DevTools yanıt verdi.
- **Elle doğrulanmadı:** yazma, tıklama, Vim tuşları, Türkçe girdi, pencere kapatma/shutdown, retina.

## Bilinen eksikler

- IME composition (CJK) yok; dead key'ler doğrulanmadı
- `<select>` popup'ları çizilmiyor; cursor şekli hiç değişmiyor
- Her paint'te tam kare kopya; GPU/IOSurface yolu yok
- Sabit 16ms CEF pump (`on_schedule_message_pump_work` boşta CPU'yu düşürür)
- `root_cache_path` yok (ortak CEF user data dizini kullanılıyor)
- Ad-hoc imzayla "Signature validation failed (-67030)" logu
- Cmd+L yok
- Sayfa yüklenirken yazılan harfler Vim tuşu sayılır (focus hook'u load end'de enjekte ediliyor)
- İç scroll'lu sayfalarda j/k çalışmaz
- winit'te `CefAppProtocol` NSApplication sınıfı yok (menü/native input riski)

## Sıradaki adımlar

1. Girdinin elle testi
2. IME
3. Popup'lar + cursor
4. Cmd+L
5. `root_cache_path`
6. Pump scheduling
7. Accelerated paint (IOSurface)
8. less-ai içinde CDP tabanlı MCP browser araçları (sayfa oku, tıkla, doldur, git)
9. Not: Chromium güvenlik güncellemesi ~4 haftada bir; framework ~200MB
