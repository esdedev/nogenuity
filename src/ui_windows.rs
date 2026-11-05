//! Windows UI: Win32 window drawn with Direct2D / DirectWrite,
//! with a tray icon.

use std::cell::RefCell;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use windows::core::{w, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
    D2D1_GRADIENT_STOP, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1LinearGradientBrush, ID2D1PathGeometry,
    ID2D1RadialGradientBrush, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_GAMMA_2_2, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_TEXT_METRICS, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
    DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateBitmap, CreateDIBSection, DeleteObject, EndPaint, InvalidateRect, ScreenToClient, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::Vector2;

use crate::audio::GameChat;
use crate::backend::{Backend, Cmd, Event};
use crate::eq::{self, F_MAX, F_MIN, GAIN_LIMIT, PRESETS, Q_MAX, Q_MIN};
use crate::devices::{self, Band, Model, Status};
use crate::i18n::t;
use crate::layout::{self, auto_off_chips, level_share, level_text, Frame, Layout, Rect, AUTO_OFF_GAP, HEADSET_ROW_TOP};
use crate::settings::{self, Settings};

pub const CLASS: PCWSTR = w!("NogenuityWindow");
pub const WM_SHOWAPP: u32 = WM_APP + 2;
const WM_BACKEND: u32 = WM_APP + 10;
const WM_TRAY: u32 = WM_APP + 1;
/// Explorer sends it when it creates the taskbar (at sign-in, or on restart): the new one doesn't
/// know the previous icons, and if the app started first, the initial one was never added.
fn wm_taskbar_created() -> u32 {
    static MSG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}
const TIMER_ANIM: usize = 1;
const TIMER_SAVE: usize = 2;
const W: f32 = 760.0;
const G_VIEW: f32 = 12.0;

// --- Palette ---
const fn rgb(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a,
    }
}
const BG: D2D1_COLOR_F = rgb(0x0B0B0D, 1.0);
const CARD: D2D1_COLOR_F = rgb(0x141418, 1.0);
const BORDER: D2D1_COLOR_F = rgb(0x24242B, 1.0);
const TRACK: D2D1_COLOR_F = rgb(0x2A2A31, 1.0);
const GRID: D2D1_COLOR_F = rgb(0x1E1E24, 1.0);
const TEXT: D2D1_COLOR_F = rgb(0xF4F4F5, 1.0);
const MUTED: D2D1_COLOR_F = rgb(0x8D8D96, 1.0);
const DIM: D2D1_COLOR_F = rgb(0x55555D, 1.0);
const RED: D2D1_COLOR_F = rgb(0xE31837, 1.0);
const RED_HI: D2D1_COLOR_F = rgb(0xFF3B4F, 1.0);
const SILVER: D2D1_COLOR_F = rgb(0xC9CED6, 1.0);
const GREEN: D2D1_COLOR_F = rgb(0x2FD27A, 1.0);
const AMBER: D2D1_COLOR_F = rgb(0xF5A524, 1.0);

const fn alpha(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a }
}

fn lerp(a: D2D1_COLOR_F, b: D2D1_COLOR_F, t: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
}

fn v(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

#[derive(Clone, Copy, Default)]
struct R {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl R {
    const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> R {
        R { x0, y0, x1, y1 }
    }
    fn d2d(self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x0, top: self.y0, right: self.x1, bottom: self.y1 }
    }
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

impl From<Rect> for R {
    fn from((x0, y0, x1, y1): Rect) -> R {
        R { x0, y0, x1, y1 }
    }
}

// --- Layout (DIPs) ---
// `layout::Frame` places the cards per headset; this is what goes inside them.

/// EQ graph inside its card.
fn graph_in(eq: R) -> R {
    R::new(eq.x0 + 44.0, eq.y0 + 52.0, eq.x1 - 18.0, eq.y0 + 256.0)
}

/// Balance slider inside its card.
fn slider_in(card: R) -> R {
    let mid = (card.x0 + card.x1) / 2.0;
    R::new(mid - 144.0, card.y0 + 76.0, mid + 144.0, card.y0 + 112.0)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Sidetone,
    Surround,
    Autostart,
    Balance,
    Eq,
    EqWarn,
    Preset(usize),
    Report,
    SidetoneLevel,
    AutoOff(u8),
    VoicePrompts,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Balance,
    Band(usize),
    SidetoneLevel,
}

#[derive(Clone, Copy, PartialEq)]
enum Conn {
    Searching,
    Connected(u16),
    Disconnected,
    Hung,
}

struct Fonts {
    title: IDWriteTextFormat,
    sub: IDWriteTextFormat,
    label: IDWriteTextFormat,
    big: IDWriteTextFormat,
    body: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_r: IDWriteTextFormat,
    small_c: IDWriteTextFormat,
    chip: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    icon_big: IDWriteTextFormat,
}

struct Gfx {
    rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    fill: ID2D1LinearGradientBrush,
    glow: ID2D1RadialGradientBrush,
    size: (u32, u32),
}

struct Anim {
    value: f32,
    target: f32,
}

impl Anim {
    fn new(on: bool) -> Anim {
        let v = on as u8 as f32;
        Anim { value: v, target: v }
    }
    fn set(&mut self, on: bool) {
        self.target = on as u8 as f32;
    }
    fn step(&mut self) -> bool {
        let d = self.target - self.value;
        if d.abs() < 0.01 {
            self.value = self.target;
            false
        } else {
            self.value += d * 0.28;
            true
        }
    }
}

pub struct App {
    chip_widths: Vec<f32>,
    hwnd: HWND,
    d2d: ID2D1Factory,
    fonts: Fonts,
    gfx: Option<Gfx>,
    dpi: f32,
    settings: Settings,
    backend: Backend,
    events: Receiver<Event>,
    audio: Option<GameChat>,
    win_levels: (f32, f32),
    status: Option<Status>,
    conn: Conn,
    /// Connected model, or the last one that connected
    model: Option<&'static Model>,
    layout: Layout,
    frame: Frame,
    log: String,
    hits: Vec<(R, Hit)>,
    drag: Drag,
    sel: Option<usize>,
    freqs: Vec<f32>,
    curve: Vec<f32>,
    autostart: bool,
    a_sidetone: Anim,
    a_surround: Anim,
    a_autostart: Anim,
    a_voice: Anim,
    /// Mic monitoring volume bar (for dragging) and the value while dragging
    level_bar: R,
    level_drag: Option<u8>,
    tray_icon: HICON,
    quit: bool,
    last_click: Instant,
}

// ---------------------------------------------------------------------------------------------
// Icon: red circle with white headphones, generated at runtime.

fn make_icon(size: i32) -> HICON {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(color) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return HICON::default();
        };
        let px = crate::icon::pixels(size as usize);
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
        let mask_bits = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
        let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info).unwrap_or_default();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

// ---------------------------------------------------------------------------------------------

fn text_format(
    dw: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
    align: DWRITE_TEXT_ALIGNMENT,
) -> Result<IDWriteTextFormat> {
    unsafe {
        let f = dw.CreateTextFormat(family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("es-es"))?;
        f.SetTextAlignment(align)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        Ok(f)
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// EQ graph coordinates: frequency <-> x, gain <-> y.
#[derive(Clone, Copy)]
struct Graph(R);

impl Graph {
    fn fx(self, f: f32) -> f32 {
        let g = self.0;
        g.x0 + (f / F_MIN).ln() / (F_MAX / F_MIN).ln() * (g.x1 - g.x0)
    }

    fn xf(self, x: f32) -> f32 {
        let g = self.0;
        let t = ((x - g.x0) / (g.x1 - g.x0)).clamp(0.0, 1.0);
        F_MIN * (F_MAX / F_MIN).powf(t)
    }

    fn gy(self, gain: f32) -> f32 {
        let g = self.0;
        g.y0 + (G_VIEW - gain) / (2.0 * G_VIEW) * (g.y1 - g.y0)
    }

    fn yg(self, y: f32) -> f32 {
        let g = self.0;
        (G_VIEW - (y - g.y0) / (g.y1 - g.y0) * 2.0 * G_VIEW).clamp(-G_VIEW, G_VIEW)
    }
}

impl App {
    fn new(hwnd: HWND) -> Result<App> {
        let settings = Settings::load();
        let d2d: ID2D1Factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let semi = DWRITE_FONT_WEIGHT_SEMI_BOLD;
        let norm = DWRITE_FONT_WEIGHT_NORMAL;
        let (lead, trail, center) = (DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_ALIGNMENT_CENTER);
        let fonts = Fonts {
            title: text_format(&dw, w!("Bahnschrift"), semi, 22.0, lead)?,
            sub: text_format(&dw, w!("Bahnschrift"), norm, 11.0, lead)?,
            label: text_format(&dw, w!("Bahnschrift"), semi, 11.5, lead)?,
            big: text_format(&dw, w!("Bahnschrift"), semi, 17.0, lead)?,
            body: text_format(&dw, w!("Segoe UI"), norm, 13.0, lead)?,
            small: text_format(&dw, w!("Segoe UI"), norm, 11.0, lead)?,
            small_r: text_format(&dw, w!("Segoe UI"), norm, 11.0, trail)?,
            small_c: text_format(&dw, w!("Segoe UI"), norm, 10.0, center)?,
            chip: text_format(&dw, w!("Bahnschrift"), semi, 11.0, center)?,
            icon: text_format(&dw, w!("Segoe MDL2 Assets"), norm, 15.0, center)?,
            icon_big: text_format(&dw, w!("Segoe MDL2 Assets"), norm, 24.0, center)?,
        };
        let chip_widths = t()
            .presets
            .iter()
            .map(|name| {
                let label = wide(&name.to_uppercase());
                unsafe {
                    let layout = dw.CreateTextLayout(&label, &fonts.chip, 1000.0, 50.0)?;
                    let mut m = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut m)?;
                    Ok(m.widthIncludingTrailingWhitespace)
                }
            })
            .collect::<Result<Vec<f32>>>()?;
        let on_connect = settings.on_connect();
        let hwnd_raw = hwnd.0 as isize;
        let notify = std::sync::Arc::new(move || unsafe {
            let _ = PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_BACKEND, WPARAM(0), LPARAM(0));
        });
        let (backend, events) = Backend::start(notify, on_connect);
        let model = settings.model.as_deref().and_then(devices::find);
        let layout = model.map_or_else(Layout::before_connect, |m| Layout::for_caps(&m.caps));
        let audio = model.and_then(|m| m.caps.game_chat_outputs).and_then(GameChat::find);
        let win_levels = audio.as_ref().and_then(GameChat::get).unwrap_or((0.0, 0.0));
        let n = 220;
        let freqs: Vec<f32> = (0..n).map(|i| F_MIN * (F_MAX / F_MIN).powf(i as f32 / (n - 1) as f32)).collect();
        let curve = eq::response_db(&settings.eq, &freqs);
        let autostart = settings::autostart_enabled();
        Ok(App {
            hwnd,
            d2d,
            fonts,
            chip_widths,
            gfx: None,
            dpi: 96.0,
            a_surround: Anim::new(settings.surround.unwrap_or(false)),
            a_sidetone: Anim::new(settings.sidetone.unwrap_or(false)),
            a_autostart: Anim::new(autostart),
            a_voice: Anim::new(settings.voice_prompts.unwrap_or(false)),
            level_bar: R::default(),
            level_drag: None,
            settings,
            backend,
            events,
            audio,
            win_levels,
            status: None,
            conn: Conn::Searching,
            model,
            layout,
            frame: Frame::new(&layout),
            log: t().looking.into(),
            hits: Vec::new(),
            drag: Drag::None,
            sel: None,
            freqs,
            curve,
            autostart,
            tray_icon: make_icon(16),
            quit: false,
            last_click: Instant::now(),
        })
    }

    fn graph(&self) -> Graph {
        Graph(graph_in(self.frame.eq.map_or(R::default(), R::from)))
    }

    fn slider(&self) -> R {
        slider_in(self.frame.balance.map_or(R::default(), R::from))
    }

    fn surround_sub(&self) -> &'static str {
        if self.layout.eq_needs_surround { t().surround_sub } else { t().surround_sub_plain }
    }

    /// Experimental model: saves the report, shows its folder and opens the form.
    fn send_report(&mut self) {
        let Some(model) = self.model else { return };
        let firmware = if let Conn::Connected(fw) = self.conn { fw } else { 0 };
        if let Some(path) = crate::report::save(model, firmware, self.status) {
            crate::sys::open(path.parent().unwrap_or(&path));
            crate::sys::open(std::path::Path::new(crate::report::FORM_URL));
            self.log = t().report_saved.into();
        }
    }

    /// Name of the model's Game and Chat outputs, if it has them.
    fn outputs(&self) -> Option<&'static str> {
        self.model?.caps.game_chat_outputs
    }

    /// Fits the client area to the layout height, without moving the window.
    fn fit_window(&self) {
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd);
            let k = dpi as f32 / 96.0;
            let style = WINDOW_STYLE(GetWindowLongW(self.hwnd, GWL_STYLE) as u32);
            let mut rc = RECT { left: 0, top: 0, right: (W * k) as i32, bottom: (self.frame.height * k) as i32 };
            let _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE::default(), dpi);
            let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = SetWindowPos(self.hwnd, None, 0, 0, rc.right - rc.left, rc.bottom - rc.top, flags);
        }
    }

    /// A model connected: show its cards, and remember it for the next start.
    fn set_model(&mut self, model: &'static Model) {
        self.model = Some(model);
        if self.settings.set_model(model.name) {
            self.update_on_connect();
            self.schedule_save();
        }
        let layout = Layout::for_caps(&model.caps);
        if layout != self.layout {
            self.layout = layout;
            self.frame = Frame::new(&layout);
            self.sel = None;
            self.gfx = None; // the EQ gradient depends on where the graph is
            self.fit_window();
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn start_anim(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_ANIM, 16, None);
        }
    }

    fn schedule_save(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SAVE, 600, None);
        }
    }

    fn update_on_connect(&self) {
        *self.backend.on_connect.lock().unwrap() = self.settings.on_connect();
    }

    // --- tray ---
    fn tray_data(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: self.tray_icon,
            ..Default::default()
        };
        for (i, c) in "Nogenuity".encode_utf16().enumerate() {
            nid.szTip[i] = c;
        }
        nid
    }

    fn add_tray(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &self.tray_data());
        }
    }

    fn remove_tray(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.tray_data());
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.invalidate();
    }

    // --- backend events ---
    fn drain_events(&mut self) {
        while let Ok(e) = self.events.try_recv() {
            match e {
                Event::Connected { model, firmware } => {
                    self.conn = Conn::Connected(firmware);
                    self.log = model.name.into();
                    self.set_model(model);
                }
                Event::Disconnected => {
                    self.conn = Conn::Disconnected;
                    self.status = None;
                    self.log = t().connect_headset.into();
                    self.cancel_level_drag();
                }
                Event::Hung => {
                    self.conn = Conn::Hung;
                    self.log = t().hung.into();
                }
                Event::Status(s) => {
                    self.status = Some(s);
                    if !matches!(self.conn, Conn::Connected(_)) {
                        self.conn = Conn::Connected(0);
                    }
                    self.a_sidetone.set(s.sidetone);
                    self.a_surround.set(s.surround);
                    if self.settings.remember(&self.layout, &s) {
                        self.update_on_connect();
                        self.schedule_save();
                    }
                    self.a_voice.set(self.voice_prompts());
                    // Turned off mid-drag: the volume can no longer be sent
                    if s.asleep {
                        self.cancel_level_drag();
                    }
                    self.start_anim();
                    if self.audio.is_none() {
                        self.audio = self.outputs().and_then(GameChat::find);
                    }
                    if let Some(levels) = self.audio.as_ref().and_then(GameChat::get) {
                        self.win_levels = levels;
                    }
                }
                Event::Button(b) => {
                    let tr = t();
                    self.log = format!(
                        "{}  ·  7.1 {}  ·  {} {:+.0} dB  ·  {} {:+.0} dB",
                        tr.control_box,
                        if b.surround { "ON" } else { "OFF" },
                        tr.game_lc,
                        b.game_db,
                        tr.chat_lc,
                        b.chat_db
                    );
                }
                Event::Log(s) => self.log = s,
            }
        }
        self.invalidate();
    }

    // --- actions ---
    fn set_balance(&mut self, value: i32) {
        let value = if value.abs() < 6 { 0 } else { value.clamp(-100, 100) };
        let Some(audio) = &self.audio else { return };
        if self.settings.balance_base.is_none() || (self.settings.balance == 0 && value != 0) {
            // Without the current volumes there's nothing to work the new ones out from
            let Some(base) = audio.get() else {
                self.log = t().no_outputs.into();
                return;
            };
            self.settings.balance_base = Some(base);
        }
        let (bg, bc) = self.settings.balance_base.unwrap();
        audio.set(bg * (1.0 - value.max(0) as f32 / 100.0), bc * (1.0 - (-value).max(0) as f32 / 100.0));
        self.settings.balance = value;
        if let Some(levels) = audio.get() {
            self.win_levels = levels;
        }
        self.schedule_save();
    }

    fn eq_changed(&mut self) {
        self.curve = eq::response_db(&self.settings.eq, &self.freqs);
        self.update_on_connect();
        self.schedule_save();
    }

    fn set_band(&mut self, i: usize, band: Band) {
        self.settings.eq[i] = band;
        self.settings.preset = None;
        self.backend.band(i, band);
        self.eq_changed();
    }

    fn apply_preset(&mut self, i: usize) {
        self.settings.eq = PRESETS[i].to_vec();
        self.settings.preset = Some(i);
        self.sel = None;
        self.backend.send(Cmd::Eq(self.settings.eq.clone()));
        self.eq_changed();
    }

    fn set_surround(&mut self, on: bool) {
        self.settings.surround = Some(on);
        if let Some(s) = &mut self.status {
            s.surround = on;
        }
        self.a_surround.set(on);
        self.start_anim();
        self.backend.send(Cmd::Surround(on));
        self.update_on_connect();
        self.schedule_save();
    }

    /// Mic monitoring volume on releasing the bar (not on every move: each value is a command).
    fn set_sidetone_level(&mut self, level: u8) {
        self.settings.sidetone_level = Some(level);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.sidetone_level = Some(level);
        }
        self.backend.send(Cmd::SidetoneLevel(level));
    }

    fn set_auto_off(&mut self, minutes: u8) {
        self.settings.auto_off = Some(minutes);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.auto_off = Some(minutes);
        }
        self.backend.send(Cmd::AutoOff(minutes));
    }

    fn set_voice_prompts(&mut self, on: bool) {
        self.settings.voice_prompts = Some(on);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.voice_prompts = Some(on);
        }
        self.a_voice.set(on);
        self.start_anim();
        self.backend.send(Cmd::VoicePrompts(on));
    }

    /// Headset card values: those reported by the headset or, if it doesn't report them (or is
    /// off), the last known ones.
    fn sidetone_level(&self) -> Option<u8> {
        self.level_drag.or(self.status.and_then(|s| s.sidetone_level)).or(self.settings.sidetone_level)
    }

    fn auto_off(&self) -> Option<u8> {
        self.status.and_then(|s| s.auto_off).or(self.settings.auto_off)
    }

    fn voice_prompts(&self) -> bool {
        self.status.and_then(|s| s.voice_prompts).or(self.settings.voice_prompts).unwrap_or(false)
    }

    /// End of a drag (on release or on losing the mouse): the volume is sent then.
    fn end_drag(&mut self) {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::None => return,
            Drag::SidetoneLevel => {
                if let Some(level) = self.level_drag.take() {
                    self.set_sidetone_level(level);
                }
            }
            Drag::Balance | Drag::Band(_) => self.schedule_save(),
        }
        unsafe {
            let _ = ReleaseCapture();
        }
        self.invalidate();
    }

    /// The headset went away mid volume drag: discard it without sending.
    fn cancel_level_drag(&mut self) {
        if self.drag == Drag::SidetoneLevel {
            self.level_drag = None;
            self.end_drag();
        }
    }

    fn set_sidetone(&mut self, on: bool) {
        self.settings.sidetone = Some(on);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.sidetone = on;
        }
        self.a_sidetone.set(on);
        self.start_anim();
        self.backend.send(Cmd::Sidetone(on));
    }

    fn nearest_band(&self, x: f32, y: f32) -> Option<usize> {
        let g = self.graph();
        self.settings
            .eq
            .iter()
            .enumerate()
            .map(|(i, b)| (i, (g.fx(b.0) - x).hypot(g.gy(b.1) - y)))
            .filter(|(_, d)| *d < 16.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h)
    }

    fn drag_to(&mut self, x: f32, y: f32) {
        match self.drag {
            Drag::Balance => {
                let slider = self.slider();
                let t = ((x - slider.x0) / (slider.x1 - slider.x0)).clamp(0.0, 1.0);
                self.set_balance((t * 200.0 - 100.0).round() as i32);
            }
            Drag::Band(i) => {
                let (_, _, q, kind) = self.settings.eq[i];
                let graph = self.graph();
                let f = graph.xf(x).round();
                let g = ((graph.yg(y) * 2.0).round() / 2.0).clamp(-GAIN_LIMIT, GAIN_LIMIT);
                self.set_band(i, (f, g, q, kind));
            }
            Drag::SidetoneLevel => {
                let bar = self.level_bar;
                let t = ((x - bar.x0) / (bar.x1 - bar.x0)).clamp(0.0, 1.0);
                let max = self.model.map_or(0, |m| m.caps.sidetone_level_max);
                self.level_drag = Some((t * max as f32).round() as u8);
            }
            Drag::None => {}
        }
        self.invalidate();
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        let double = self.last_click.elapsed().as_millis() < 350;
        self.last_click = Instant::now();
        match self.hit(x, y) {
            Some(Hit::Sidetone) => {
                let on = !self.status.map_or(false, |s| s.sidetone);
                self.set_sidetone(on);
            }
            Some(Hit::Surround) => {
                let on = !self.status.map_or(self.settings.surround.unwrap_or(false), |s| s.surround);
                self.set_surround(on);
            }
            Some(Hit::EqWarn) => self.set_surround(true),
            Some(Hit::Autostart) => {
                settings::set_autostart(!self.autostart);
                self.autostart = settings::autostart_enabled();
                self.a_autostart.set(self.autostart);
                self.start_anim();
            }
            Some(Hit::Preset(i)) => self.apply_preset(i),
            Some(Hit::Report) => self.send_report(),
            Some(Hit::SidetoneLevel) => {
                self.drag = Drag::SidetoneLevel;
                self.drag_to(x, y);
            }
            Some(Hit::AutoOff(minutes)) => self.set_auto_off(minutes),
            Some(Hit::VoicePrompts) => {
                let on = !self.voice_prompts();
                self.set_voice_prompts(on);
            }
            Some(Hit::Balance) => {
                if double {
                    self.set_balance(0);
                } else {
                    self.drag = Drag::Balance;
                    self.drag_to(x, y);
                }
            }
            Some(Hit::Eq) => {
                self.sel = self.nearest_band(x, y);
                if let Some(i) = self.sel {
                    if double {
                        let (f, _, q, k) = self.settings.eq[i];
                        self.set_band(i, (f, 0.0, q, k));
                    } else {
                        self.drag = Drag::Band(i);
                    }
                }
            }
            None => self.sel = None,
        }
        if self.drag != Drag::None {
            unsafe {
                SetCapture(self.hwnd);
            }
        }
        self.invalidate();
    }

    fn wheel(&mut self, x: f32, y: f32, delta: i16) {
        if !self.layout.eq || !self.graph().0.contains(x, y) {
            return;
        }
        let Some(i) = self.sel.or_else(|| self.nearest_band(x, y)) else { return };
        let (f, g, q, k) = self.settings.eq[i];
        let factor = if delta > 0 { 1.15 } else { 1.0 / 1.15 };
        let q = ((q * factor * 100.0).round() / 100.0).clamp(Q_MIN, Q_MAX);
        self.sel = Some(i);
        self.set_band(i, (f, g, q, k));
        self.invalidate();
    }

    // --- drawing ---
    fn ensure_gfx(&mut self) -> Result<()> {
        let mut rc = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut rc)? };
        let size = ((rc.right - rc.left).max(1) as u32, (rc.bottom - rc.top).max(1) as u32);
        if let Some(g) = &mut self.gfx {
            if g.size != size {
                unsafe { g.rt.Resize(&D2D_SIZE_U { width: size.0, height: size.1 })? };
                g.size = size;
            }
            unsafe { g.rt.SetDpi(self.dpi, self.dpi) };
            return Ok(());
        }
        let graph = self.graph().0;
        unsafe {
            let rt = self.d2d.CreateHwndRenderTarget(
                // CPU rendering: plenty for a window like this and it doesn't load the GPU driver
                // (with hardware the process went from ~10 to ~65 MB and ~1,000 handles)
                &D2D1_RENDER_TARGET_PROPERTIES { r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE, ..Default::default() },
                &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: self.hwnd,
                    pixelSize: D2D_SIZE_U { width: size.0, height: size.1 },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                },
            )?;
            rt.SetDpi(self.dpi, self.dpi);
            let brush = rt.CreateSolidColorBrush(&TEXT, None)?;
            let fill_stops = rt.CreateGradientStopCollection(
                &[
                    D2D1_GRADIENT_STOP { position: 0.0, color: alpha(RED, 0.45) },
                    D2D1_GRADIENT_STOP { position: 0.5, color: alpha(RED, 0.02) },
                    D2D1_GRADIENT_STOP { position: 1.0, color: alpha(RED, 0.45) },
                ],
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )?;
            let fill = rt.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: v(0.0, graph.y0), endPoint: v(0.0, graph.y1) },
                None,
                &fill_stops,
            )?;
            let glow_stops = rt.CreateGradientStopCollection(
                &[
                    D2D1_GRADIENT_STOP { position: 0.0, color: alpha(RED, 0.22) },
                    D2D1_GRADIENT_STOP { position: 1.0, color: alpha(RED, 0.0) },
                ],
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )?;
            let glow = rt.CreateRadialGradientBrush(
                &D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center: v(60.0, 10.0),
                    gradientOriginOffset: v(0.0, 0.0),
                    radiusX: 420.0,
                    radiusY: 260.0,
                },
                None,
                &glow_stops,
            )?;
            self.gfx = Some(Gfx { rt, brush, fill, glow, size });
        }
        Ok(())
    }

    fn paint(&mut self) {
        if self.ensure_gfx().is_err() {
            return;
        }
        self.hits.clear();
        let g = self.gfx.as_ref().unwrap();
        let (rt, brush) = (g.rt.clone(), g.brush.clone());
        let (fill, glow) = (g.fill.clone(), g.glow.clone());
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&BG));
            rt.FillRectangle(&R::new(0.0, 0.0, W, self.frame.height).d2d(), &glow);
        }
        let d = Draw { rt: &rt, brush: &brush };
        self.paint_header(&d);
        if let Some(c) = self.frame.balance {
            self.paint_balance(&d, c.into());
        }
        if let Some(c) = self.frame.mic {
            self.paint_mic(&d, c.into());
        }
        if let Some(c) = self.frame.headset {
            self.paint_headset(&d, c.into());
        }
        if let Some(c) = self.frame.eq {
            self.paint_eq(&d, c.into(), &fill);
        }
        self.paint_footer(&d);
        let result = unsafe { rt.EndDraw(None, None) };
        if result.is_err() {
            self.gfx = None; // D2DERR_RECREATE_TARGET: recreated on the next WM_PAINT
            self.invalidate();
        }
    }

    fn paint_header(&mut self, d: &Draw) {
        // Logo: glowing red square with the headphones icon
        d.glow_round(R::new(20.0, 20.0, 64.0, 64.0), 12.0, RED, 0.35);
        d.round(R::new(20.0, 20.0, 64.0, 64.0), 12.0, RED);
        d.text("\u{E7F6}", &self.fonts.icon_big, R::new(20.0, 20.0, 64.0, 64.0), TEXT);
        // Model name without the brand: the app is already HyperX-only
        let title = self.model.map_or("NOGENUITY".to_string(), |m| m.name.trim_start_matches("HyperX ").to_uppercase());
        d.text(&title, &self.fonts.title, R::new(78.0, 18.0, 400.0, 46.0), TEXT);
        d.text(t().subtitle, &self.fonts.sub, R::new(79.0, 44.0, 420.0, 62.0), MUTED);
        if self.model.is_some_and(|m| !m.tested) {
            let badge = R::new(406.0, 30.0, 516.0, 54.0);
            d.round(badge, 12.0, alpha(AMBER, 0.12));
            d.round_stroke(badge, 12.0, alpha(AMBER, 0.5), 1.0);
            d.text(t().experimental, &self.fonts.chip, badge, AMBER);
        }

        let (txt, col) = match self.conn {
            Conn::Searching => (t().searching.to_string(), MUTED),
            Conn::Connected(fw) => {
                let p = layout::pill(fw, self.status, &self.layout);
                let col = if p == layout::Pill::HeadsetOff { AMBER } else { GREEN };
                (p.text(t().connected, t().charging, t().headset_off), col)
            }
            Conn::Disconnected => (t().disconnected.to_string(), RED_HI),
            Conn::Hung => (t().not_responding.to_string(), AMBER),
        };
        let pill = R::new(530.0, 28.0, 740.0, 56.0);
        d.round(pill, 14.0, alpha(col, 0.10));
        d.round_stroke(pill, 14.0, alpha(col, 0.35), 1.0);
        d.circle(pill.x0 + 18.0, 42.0, 4.0, col);
        d.glow_circle(pill.x0 + 18.0, 42.0, 9.0, col);
        d.text(&txt, &self.fonts.chip, R::new(pill.x0 + 26.0, pill.y0, pill.x1 - 6.0, pill.y1), TEXT);
        // Red line under the header, fading out
        for i in 0..40 {
            let t = i as f32 / 40.0;
            d.line(20.0 + t * 720.0, 72.0, 20.0 + (t + 1.0 / 40.0) * 720.0, 72.0, alpha(RED, 0.55 * (1.0 - t)), 1.0);
        }
    }

    fn card(&self, d: &Draw, r: R, title: &str, icon: &str) {
        d.round(r, 14.0, CARD);
        d.round_stroke(r, 14.0, BORDER, 1.0);
        d.round(R::new(r.x0 + 18.0, r.y0 + 20.0, r.x0 + 21.0, r.y0 + 34.0), 1.5, RED);
        d.text(icon, &self.fonts.icon, R::new(r.x0 + 26.0, r.y0 + 17.0, r.x0 + 46.0, r.y0 + 37.0), MUTED);
        d.text(title, &self.fonts.label, R::new(r.x0 + 50.0, r.y0 + 17.0, r.x1 - 16.0, r.y0 + 37.0), TEXT);
    }

    fn paint_balance(&mut self, d: &Draw, c: R) {
        self.card(d, c, t().balance, "\u{E767}");
        if !self.layout.balance_slider {
            self.paint_box_levels(d, c);
            return;
        }
        if self.layout.box_levels {
            let levels = match self.status {
                Some(s) => format!("{}  ·  {} {:+.0} dB  ·  {} {:+.0} dB", t().control_box, t().game_lc, s.game_db, t().chat_lc, s.chat_db),
                None => format!("{}  ·  —", t().control_box),
            };
            d.text(&levels, &self.fonts.small, R::new(c.x0 + 26.0, c.y1 - 34.0, c.x1 - 16.0, c.y1 - 16.0), MUTED);
        }
        let slider = self.slider();
        let value = self.settings.balance as f32;
        let (x0, x1, y) = (slider.x0 + 30.0, slider.x1 - 30.0, (slider.y0 + slider.y1) / 2.0);
        let mid = (x0 + x1) / 2.0;
        let x = mid + value / 100.0 * (x1 - x0) / 2.0;
        // End icons
        d.text("\u{E7FC}", &self.fonts.icon, R::new(slider.x0 - 4.0, y - 12.0, slider.x0 + 24.0, y + 12.0), RED_HI);
        d.text("\u{E8BD}", &self.fonts.icon, R::new(slider.x1 - 24.0, y - 12.0, slider.x1 + 4.0, y + 12.0), SILVER);
        d.round(R::new(x0, y - 3.0, x1, y + 3.0), 3.0, TRACK);
        let col = if value < 0.0 { RED_HI } else { SILVER };
        d.round(R::new(x.min(mid), y - 3.0, x.max(mid), y + 3.0), 3.0, col);
        d.line(mid, y - 9.0, mid, y + 9.0, DIM, 1.0);
        d.glow_circle(x, y, 18.0, if value < 0.0 { RED } else { SILVER });
        d.circle(x, y, 10.0, TEXT);
        d.circle(x, y, 4.0, if value < 0.0 { RED } else { rgb(0x6B6F78, 1.0) });
        d.text(t().game, &self.fonts.small, R::new(x0 - 4.0, y + 14.0, x0 + 60.0, y + 30.0), MUTED);
        d.text(t().chat, &self.fonts.small_r, R::new(x1 - 60.0, y + 14.0, x1 + 4.0, y + 30.0), MUTED);
        self.hits.push((slider, Hit::Balance));

        let (wg, wc) = self.win_levels;
        let line1 = if self.audio.is_some() {
            format!("Windows  ·  {} {:.0} %  ·  {} {:.0} %", t().game_lc, wg * 100.0, t().chat_lc, wc * 100.0)
        } else {
            t().no_outputs.into()
        };
        d.text(&line1, &self.fonts.body, R::new(c.x0 + 26.0, c.y1 - 56.0, c.x1 - 16.0, c.y1 - 36.0), TEXT);
    }

    /// Without Game and Chat outputs in the system there's no slider: just the box levels, large.
    fn paint_box_levels(&self, d: &Draw, c: R) {
        let y = c.y0 + 94.0;
        let mid = (c.x0 + c.x1) / 2.0;
        let level = |db: fn(&Status) -> f32| self.status.map_or("—".to_string(), |s| format!("{:+.0} dB", db(&s)));
        let sides = [
            ("\u{E7FC}", RED_HI, t().game, level(|s| s.game_db), c.x0 + 26.0),
            ("\u{E8BD}", SILVER, t().chat, level(|s| s.chat_db), mid + 8.0),
        ];
        for (icon, col, name, value, x) in sides {
            d.text(icon, &self.fonts.icon, R::new(x, y - 12.0, x + 28.0, y + 12.0), col);
            d.text(&value, &self.fonts.big, R::new(x + 36.0, y - 16.0, x + 160.0, y + 6.0), TEXT);
            d.text(name, &self.fonts.small, R::new(x + 36.0, y + 6.0, x + 160.0, y + 22.0), MUTED);
        }
        d.text(t().control_box, &self.fonts.small, R::new(c.x0 + 26.0, c.y1 - 34.0, c.x1 - 16.0, c.y1 - 16.0), MUTED);
    }

    /// Headset card: mic monitoring volume, auto power-off and voice prompts. Each
    /// row is as tall as those of the mic card.
    fn paint_headset(&mut self, d: &Draw, c: R) {
        self.card(d, c, t().headset_card, "\u{E7F6}");
        let connected = self.status.is_some_and(|s| !s.asleep);
        let caps = self.model.map(|m| m.caps).unwrap_or_default();
        let [level_y, auto_off_y, voice_y] = self.layout.headset_rows();
        // Row at height `y` in the card; all but the first get a line above
        let row = |y: f32| {
            let r = R::new(c.x0 + 18.0, c.y0 + y, c.x1 - 18.0, c.y0 + y + 40.0);
            if y > HEADSET_ROW_TOP {
                d.line(r.x0, r.y0 - 4.0, r.x1, r.y0 - 4.0, BORDER, 1.0);
            }
            r
        };

        if let Some(y) = level_y {
            let r = row(y);
            let share = level_share(self.sidetone_level(), caps.sidetone_level_max);
            d.text(t().sidetone_level, &self.fonts.body, R::new(r.x0, r.y0 + 2.0, r.x1 - 60.0, r.y0 + 21.0), TEXT);
            let value = level_text(self.sidetone_level(), caps.sidetone_level_max, caps.sidetone_level_zero);
            d.text(&value, &self.fonts.small_r, R::new(r.x1 - 60.0, r.y0 + 2.0, r.x1, r.y0 + 21.0), if connected { SILVER } else { DIM });
            let (x0, x1, by) = (r.x0 + 8.0, r.x1 - 8.0, r.y0 + 31.0);
            d.round(R::new(x0, by - 3.0, x1, by + 3.0), 3.0, TRACK);
            if let Some(s) = share {
                let x = x0 + s * (x1 - x0);
                d.round(R::new(x0, by - 3.0, x, by + 3.0), 3.0, if connected { RED_HI } else { DIM });
                if connected {
                    d.glow_circle(x, by, 14.0, RED);
                }
                d.circle(x, by, 8.0, if connected { TEXT } else { DIM });
                d.circle(x, by, 3.0, if connected { RED } else { TRACK });
            }
            self.level_bar = R::new(x0, by, x1, by);
            if connected {
                self.hits.push((R::new(x0 - 10.0, by - 12.0, x1 + 10.0, by + 12.0), Hit::SidetoneLevel));
            }
        }

        if let Some(y) = auto_off_y {
            let r = row(y);
            // One chip per option, spread across the width
            let tr = t();
            let (chips, short) = auto_off_chips(caps.auto_off, r.x1 - r.x0, tr.never, tr.never_short, tr.minutes);
            let title = if short { format!("{} ({})", tr.auto_off, tr.minutes) } else { tr.auto_off.to_string() };
            d.text(&title, &self.fonts.body, R::new(r.x0, r.y0 + 2.0, r.x1, r.y0 + 21.0), TEXT);
            let n = chips.len() as f32;
            let w = ((r.x1 - r.x0) - AUTO_OFF_GAP * (n - 1.0)) / n;
            let current = self.auto_off();
            for (i, (label, minutes)) in chips.into_iter().enumerate() {
                let x = r.x0 + i as f32 * (w + AUTO_OFF_GAP);
                let chip = R::new(x, r.y0 + 21.0, x + w, r.y0 + 39.0);
                let active = current == Some(minutes);
                if active {
                    d.round(chip, 9.0, if connected { RED } else { TRACK });
                } else {
                    d.round(chip, 9.0, rgb(0x1F1F25, 1.0));
                    d.round_stroke(chip, 9.0, BORDER, 1.0);
                }
                let col = if !connected { DIM } else if active { TEXT } else { SILVER };
                d.text(&label, &self.fonts.chip, chip, col);
                if connected {
                    self.hits.push((chip, Hit::AutoOff(minutes)));
                }
            }
        }

        if let Some(y) = voice_y {
            let r = row(y);
            d.text(t().voice_prompts, &self.fonts.body, R::new(r.x0, r.y0 + 2.0, r.x1 - 60.0, r.y0 + 21.0), TEXT);
            d.text(t().voice_prompts_sub, &self.fonts.small, R::new(r.x0, r.y0 + 20.0, r.x1 - 60.0, r.y0 + 37.0), MUTED);
            d.toggle(r.x1 - 44.0, r.y0 + 8.0, self.a_voice.value, connected);
            if connected {
                self.hits.push((r, Hit::VoicePrompts));
            }
        }
    }

    fn paint_mic(&mut self, d: &Draw, c: R) {
        self.card(d, c, t().mic_sound, "\u{E720}");
        // With the headset off (receiver only) the controls don't respond
        let connected = self.status.is_some_and(|s| !s.asleep);
        if self.layout.mic_status {
            self.paint_mic_status(d, c);
        }
        let all = [
            (self.layout.sidetone, Hit::Sidetone, t().mic_monitoring, t().mic_monitoring_sub, self.a_sidetone.value),
            (self.layout.surround, Hit::Surround, t().surround, self.surround_sub(), self.a_surround.value),
        ];
        let rows = all.into_iter().filter(|r| r.0).map(|(_, hit, title, sub, t)| (hit, title, sub, t));
        // Without the mic indicator, the rows move up into place
        let top = c.y0 + self.layout.mic_rows_top();
        // One full-width row per setting: title and description on the left, toggle on the right
        for (i, (hit, title, sub, t)) in rows.enumerate() {
            let y = top + i as f32 * 46.0;
            let row = R::new(c.x0 + 18.0, y, c.x1 - 18.0, y + 40.0);
            // The line separates from the indicator or the row above; not right under the title
            if self.layout.mic_status || i > 0 {
                d.line(row.x0, y - 4.0, row.x1, y - 4.0, BORDER, 1.0);
            }
            d.text(title, &self.fonts.body, R::new(row.x0, row.y0 + 2.0, row.x1 - 60.0, row.y0 + 21.0), TEXT);
            d.text(sub, &self.fonts.small, R::new(row.x0, row.y0 + 20.0, row.x1 - 60.0, row.y0 + 37.0), MUTED);
            d.toggle(row.x1 - 44.0, row.y0 + 8.0, t, connected);
            if connected {
                self.hits.push((row, hit));
            }
        }
    }

    fn paint_mic_status(&self, d: &Draw, c: R) {
        let muted = self.status.is_some_and(|s| s.mic_muted);
        // With the headset off (receiver only) the controls don't respond
        let connected = self.status.is_some_and(|s| !s.asleep);
        // Mic indicator
        let (cx, cy) = (c.x0 + 48.0, c.y0 + 78.0);
        let ring = if !connected { DIM } else if muted { RED_HI } else { GREEN };
        d.glow_circle(cx, cy, 30.0, ring);
        d.circle(cx, cy, 21.0, rgb(0x1C1C22, 1.0));
        d.circle_stroke(cx, cy, 21.0, ring, 2.0);
        d.text("\u{E720}", &self.fonts.icon, R::new(cx - 14.0, cy - 14.0, cx + 14.0, cy + 14.0), TEXT);
        if muted {
            d.line(cx - 11.0, cy + 11.0, cx + 11.0, cy - 11.0, RED_HI, 2.5);
        }
        let (state, hint) = if !connected {
            ("—", t().connect_headset)
        } else if muted {
            (t().muted, if self.layout.box_levels { t().unmute_hint } else { t().unmute_hint_headset })
        } else {
            (t().live, if self.layout.box_levels { t().mute_hint } else { t().mute_hint_headset })
        };
        d.text(state, &self.fonts.big, R::new(cx + 34.0, cy - 20.0, c.x1 - 18.0, cy + 2.0), if muted { RED_HI } else { TEXT });
        d.text(hint, &self.fonts.small, R::new(cx + 34.0, cy + 2.0, c.x1 - 18.0, cy + 20.0), MUTED);
    }

    fn paint_eq(&mut self, d: &Draw, c: R, fill: &ID2D1LinearGradientBrush) {
        self.card(d, c, t().equalizer, "\u{E9E9}");
        let graph = self.graph();
        let (gr, fx, gy) = (graph.0, |f| graph.fx(f), |g| graph.gy(g));
        // Selected band readout, or help
        let info = match self.sel {
            Some(i) => {
                let (f, g, q, k) = self.settings.eq[i];
                let hz = if f >= 1000.0 { format!("{:.1} kHz", f / 1000.0) } else { format!("{f:.0} Hz") };
                format!("{} {}  ·  {hz}  ·  {g:+.1} dB  ·  Q {q:.2}{}", t().band, i + 1, if k == 1 { "  ·  SHELF" } else { "" })
            }
            None => t().eq_help.into(),
        };
        d.text(&info, &self.fonts.small_r, R::new(c.x0 + 300.0, c.y0 + 17.0, c.x1 - 18.0, c.y0 + 37.0), if self.sel.is_some() { RED_HI } else { MUTED });

        let surround_on = self.status.map_or(self.settings.surround.unwrap_or(false), |s| s.surround);
        if self.layout.eq_needs_surround && !surround_on {
            let chip = R::new(c.x0 + 170.0, c.y0 + 15.0, c.x0 + 430.0, c.y0 + 39.0);
            d.round(chip, 12.0, alpha(AMBER, 0.12));
            d.round_stroke(chip, 12.0, alpha(AMBER, 0.5), 1.0);
            d.text(t().eq_needs_surround, &self.fonts.chip, chip, AMBER);
            self.hits.push((chip, Hit::EqWarn));
        }

        // Grid and out-of-range zones
        d.rect(R::new(gr.x0, gy(G_VIEW), gr.x1, gy(GAIN_LIMIT)), rgb(0x101013, 1.0));
        d.rect(R::new(gr.x0, gy(-GAIN_LIMIT), gr.x1, gy(-G_VIEW)), rgb(0x101013, 1.0));
        for g in [-12.0f32, -6.0, 0.0, 6.0, 12.0] {
            let y = gy(g);
            d.line(gr.x0, y, gr.x1, y, if g == 0.0 { DIM } else { GRID }, 1.0);
            let label = if g == 0.0 { "0".to_string() } else { format!("{g:+.0}") };
            d.text(&label, &self.fonts.small_r, R::new(gr.x0 - 40.0, y - 8.0, gr.x0 - 8.0, y + 8.0), MUTED);
        }
        for (f, label) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k"), (20000.0, "20k")] {
            let x = fx(f);
            d.line(x, gr.y0, x, gr.y1, GRID, 1.0);
            d.text(label, &self.fonts.small_c, R::new(x - 20.0, gr.y1 + 4.0, x + 20.0, gr.y1 + 20.0), MUTED);
        }

        // Curve with gradient fill and glow
        let pts: Vec<Vector2> = self
            .freqs
            .iter()
            .zip(&self.curve)
            .map(|(&f, &db)| v(fx(f), gy(db.clamp(-G_VIEW, G_VIEW))))
            .collect();
        if let (Ok(area), Ok(line)) = (self.path(&pts, Some(gy(0.0))), self.path(&pts, None)) {
            unsafe {
                d.rt.FillGeometry(&area, fill, None::<&ID2D1Brush>);
            }
            d.geometry(&line, alpha(RED, 0.10), 10.0);
            d.geometry(&line, alpha(RED, 0.25), 5.0);
            d.geometry(&line, RED_HI, 2.2);
        }
        for (i, &(f, g, _, _)) in self.settings.eq.iter().enumerate() {
            let (x, y) = (fx(f), gy(g));
            let selected = self.sel == Some(i);
            if selected {
                d.glow_circle(x, y, 20.0, RED);
                d.circle(x, y, 8.0, RED_HI);
                d.circle(x, y, 3.0, TEXT);
            } else {
                d.circle(x, y, 6.5, TEXT);
                d.circle_stroke(x, y, 6.5, RED, 2.0);
            }
            d.text(&(i + 1).to_string(), &self.fonts.small_c, R::new(x - 10.0, y - 26.0, x + 10.0, y - 12.0), if selected { RED_HI } else { MUTED });
        }
        self.hits.push((R::new(gr.x0 - 10.0, gr.y0 - 10.0, gr.x1 + 10.0, gr.y1 + 6.0), Hit::Eq));

        // Presets: chips sized to the actual text width, spread to fill exactly the card width.
        // If they don't fit in one row (e.g. with longer names), two rows are used.
        const PAD: f32 = 18.0;
        const GAP: f32 = 8.0;
        let (x0, x1) = (c.x0 + 22.0, c.x1 - 22.0);
        let n = self.chip_widths.len();
        let natural: f32 = self.chip_widths.iter().map(|w| w + PAD).sum::<f32>() + GAP * (n - 1) as f32;
        let rows: Vec<std::ops::Range<usize>> =
            if natural <= x1 - x0 { vec![0..n] } else { vec![0..n.div_ceil(2), n.div_ceil(2)..n] };
        let (chip_h, row_step) = if rows.len() == 1 { (30.0, 0.0) } else { (26.0, 31.0) };
        let mut y = if rows.len() == 1 { gr.y1 + 36.0 } else { gr.y1 + 26.0 };
        for range in rows {
            let count = range.len() as f32;
            let used: f32 = self.chip_widths[range.clone()].iter().map(|w| w + PAD).sum::<f32>() + GAP * (count - 1.0);
            let extra = ((x1 - x0) - used) / count;
            let mut x = x0;
            for i in range {
                let w = self.chip_widths[i] + PAD + extra;
                self.chip(d, i, R::new(x, y, x + w, y + chip_h));
                x += w + GAP;
            }
            y += row_step;
        }
    }

    fn chip(&mut self, d: &Draw, i: usize, chip: R) {
        let name = t().presets[i];
        {
            let radius = (chip.y1 - chip.y0) / 2.0;
            let active = self.settings.preset == Some(i);
            if active {
                d.glow_round(chip, radius, RED, 0.35);
                d.round(chip, radius, RED);
            } else {
                d.round(chip, radius, rgb(0x1F1F25, 1.0));
                d.round_stroke(chip, radius, BORDER, 1.0);
            }
            d.text(&name.to_uppercase(), &self.fonts.chip, chip, if active { TEXT } else { SILVER });
            self.hits.push((chip, Hit::Preset(i)));
        }
    }

    fn paint_footer(&mut self, d: &Draw) {
        let f = R::from(self.frame.footer);
        let mid = (f.y0 + f.y1) / 2.0;
        d.round(f, 12.0, CARD);
        d.round_stroke(f, 12.0, BORDER, 1.0);
        let dot = match self.conn {
            Conn::Connected(_) => GREEN,
            Conn::Hung => AMBER,
            Conn::Disconnected => RED_HI,
            Conn::Searching => MUTED,
        };
        d.circle(f.x0 + 18.0, mid, 3.5, dot);
        // Experimental models: link to send a report
        let mut log_end = f.x1 - 230.0;
        if self.model.is_some_and(|m| !m.tested) {
            let link = R::new(f.x1 - 340.0, f.y0, f.x1 - 224.0, f.y1);
            d.text(t().send_report, &self.fonts.small_r, link, RED_HI);
            self.hits.push((link, Hit::Report));
            log_end = link.x0 - 10.0;
        }
        d.text(&self.log, &self.fonts.small, R::new(f.x0 + 30.0, f.y0, log_end, f.y1), MUTED);
        d.line(f.x1 - 214.0, f.y0 + 9.0, f.x1 - 214.0, f.y1 - 9.0, BORDER, 1.0);
        let row = R::new(f.x1 - 206.0, f.y0, f.x1 - 10.0, f.y1);
        d.text(t().start_with_windows, &self.fonts.small_r, R::new(row.x0, f.y0, row.x1 - 52.0, f.y1), SILVER);
        d.toggle(row.x1 - 44.0, mid - 12.0, self.a_autostart.value, true);
        self.hits.push((row, Hit::Autostart));
    }

    /// Line through the points; with `fill_to`, an area closed down to that y (the 0 dB one).
    fn path(&self, pts: &[Vector2], fill_to: Option<f32>) -> Result<ID2D1PathGeometry> {
        unsafe {
            let geom = self.d2d.CreatePathGeometry()?;
            let sink = geom.Open()?;
            if let Some(zero) = fill_to {
                sink.BeginFigure(v(pts[0].X, zero), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLines(pts);
                sink.AddLine(v(pts[pts.len() - 1].X, zero));
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            } else {
                sink.BeginFigure(pts[0], D2D1_FIGURE_BEGIN_HOLLOW);
                sink.AddLines(&pts[1..]);
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close()?;
            Ok(geom)
        }
    }

    fn on_timer(&mut self, id: usize) {
        if id == TIMER_SAVE {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
            }
            self.settings.save();
            return;
        }
        let moving = self.a_sidetone.step() | self.a_surround.step() | self.a_autostart.step() | self.a_voice.step();
        if !moving {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_ANIM);
            }
        }
        self.invalidate();
    }

    fn dip(&self, lp: LPARAM) -> (f32, f32) {
        let x = (lp.0 & 0xFFFF) as i16 as f32;
        let y = ((lp.0 >> 16) & 0xFFFF) as i16 as f32;
        let k = 96.0 / self.dpi;
        (x * k, y * k)
    }

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<isize> {
        if msg == wm_taskbar_created() {
            self.add_tray();
            return Some(0);
        }
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                unsafe { BeginPaint(self.hwnd, &mut ps) };
                self.paint();
                unsafe {
                    let _ = EndPaint(self.hwnd, &ps);
                }
                Some(0)
            }
            WM_ERASEBKGND => Some(1),
            WM_BACKEND => {
                self.drain_events();
                Some(0)
            }
            WM_TIMER => {
                self.on_timer(wp.0);
                Some(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = self.dip(lp);
                self.mouse_down(x, y);
                Some(0)
            }
            WM_MOUSEMOVE => {
                if self.drag != Drag::None {
                    let (x, y) = self.dip(lp);
                    self.drag_to(x, y);
                }
                Some(0)
            }
            WM_LBUTTONUP => {
                self.end_drag();
                Some(0)
            }
            // Another window took the mouse mid-drag (Alt+Tab, a notification...)
            WM_CAPTURECHANGED => {
                self.end_drag();
                Some(0)
            }
            WM_MOUSEWHEEL => {
                let mut pt = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
                unsafe {
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                let k = 96.0 / self.dpi;
                self.wheel(pt.x as f32 * k, pt.y as f32 * k, ((wp.0 >> 16) & 0xFFFF) as i16);
                Some(0)
            }
            WM_SETCURSOR => {
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                let k = 96.0 / self.dpi;
                if (lp.0 & 0xFFFF) as u32 == HTCLIENT && self.hit(pt.x as f32 * k, pt.y as f32 * k).is_some() {
                    unsafe {
                        let _ = SetCursor(LoadCursorW(None, IDC_HAND).ok());
                    }
                    return Some(1);
                }
                None
            }
            WM_SIZE => {
                self.invalidate();
                None
            }
            WM_DPICHANGED => {
                self.dpi = (wp.0 & 0xFFFF) as f32;
                let r = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(self.hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                self.invalidate();
                Some(0)
            }
            WM_CLOSE => {
                // Closing the window hides it in the tray; quitting is done from the icon menu.
                // While hidden it draws nothing: release drawing resources and memory.
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                self.settings.save();
                self.gfx = None;
                unsafe {
                    let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
                }
                Some(0)
            }
            WM_SHOWAPP => {
                self.show();
                Some(0)
            }
            WM_TRAY => {
                if (lp.0 & 0xFFFF) as u32 == WM_LBUTTONUP {
                    self.show();
                }
                Some(0)
            }
            _ => None,
        }
    }
}

/// Drawing shortcuts on the render target with a single reused solid brush.
struct Draw<'a> {
    rt: &'a ID2D1HwndRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
}

impl Draw<'_> {
    fn color(&self, c: D2D1_COLOR_F) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&c) };
        self.brush
    }
    fn rect(&self, r: R, c: D2D1_COLOR_F) {
        unsafe { self.rt.FillRectangle(&r.d2d(), self.color(c)) }
    }
    fn round(&self, r: R, radius: f32, c: D2D1_COLOR_F) {
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.color(c)) }
    }
    fn round_stroke(&self, r: R, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let rr = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F { left: r.x0 + 0.5, top: r.y0 + 0.5, right: r.x1 - 0.5, bottom: r.y1 - 0.5 },
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { self.rt.DrawRoundedRectangle(&rr, self.color(c), width, None::<&ID2D1StrokeStyle>) }
    }
    /// Soft glow around a rounded rectangle (concentric translucent layers).
    fn glow_round(&self, r: R, radius: f32, c: D2D1_COLOR_F, strength: f32) {
        for i in (1..=6).rev() {
            let e = i as f32 * 2.0;
            self.round(R::new(r.x0 - e, r.y0 - e, r.x1 + e, r.y1 + e), radius + e, alpha(c, strength * 0.12 / i as f32));
        }
    }
    fn circle(&self, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F) {
        let e = D2D1_ELLIPSE { point: v(x, y), radiusX: radius, radiusY: radius };
        unsafe { self.rt.FillEllipse(&e, self.color(c)) }
    }
    fn circle_stroke(&self, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F, width: f32) {
        let e = D2D1_ELLIPSE { point: v(x, y), radiusX: radius, radiusY: radius };
        unsafe { self.rt.DrawEllipse(&e, self.color(c), width, None::<&ID2D1StrokeStyle>) }
    }
    fn glow_circle(&self, x: f32, y: f32, radius: f32, c: D2D1_COLOR_F) {
        for i in 1..=6 {
            let r = radius * i as f32 / 6.0;
            self.circle(x, y, r, alpha(c, 0.05));
        }
    }
    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: D2D1_COLOR_F, width: f32) {
        unsafe { self.rt.DrawLine(v(x0, y0), v(x1, y1), self.color(c), width, None::<&ID2D1StrokeStyle>) }
    }
    fn geometry(&self, g: &ID2D1PathGeometry, c: D2D1_COLOR_F, width: f32) {
        unsafe { self.rt.DrawGeometry(g, self.color(c), width, None::<&ID2D1StrokeStyle>) }
    }
    fn text(&self, s: &str, f: &IDWriteTextFormat, r: R, c: D2D1_COLOR_F) {
        let w = wide(s);
        unsafe {
            self.rt.DrawText(&w, f, &r.d2d(), self.color(c), D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL)
        }
    }
    /// Animated toggle: t goes from 0 (off) to 1 (on).
    fn toggle(&self, x: f32, y: f32, t: f32, enabled: bool) {
        let track = R::new(x, y, x + 44.0, y + 24.0);
        let on = if enabled { lerp(TRACK, RED, t) } else { rgb(0x1F1F25, 1.0) };
        if enabled && t > 0.05 {
            self.glow_round(track, 12.0, RED, 0.5 * t);
        }
        self.round(track, 12.0, on);
        let kx = x + 12.0 + t * 20.0;
        self.circle(kx, y + 12.0, 9.0, if enabled { TEXT } else { DIM });
    }
}

// ---------------------------------------------------------------------------------------------

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let cell = &*ptr;

        // The icon menu runs a modal loop: show it without holding the App borrow
        if msg == WM_TRAY && (lp.0 & 0xFFFF) as u32 == WM_RBUTTONUP {
            let id = tray_menu(hwnd);
            if let Ok(mut app) = cell.try_borrow_mut() {
                if id == 1 {
                    app.show();
                } else if id == 2 {
                    app.quit = true;
                }
            }
        } else if let Ok(mut app) = cell.try_borrow_mut() {
            if let Some(r) = app.handle(msg, wp, lp) {
                return LRESULT(r);
            }
        }

        // Reentrant messages (e.g. those triggered by ShowWindow) arrive with the App already borrowed:
        // never use borrow() here, only try_borrow()
        if cell.try_borrow().is_ok_and(|a| a.quit) {
            let app = cell.borrow();
            app.settings.save();
            app.remove_tray();
            drop(app);
            let _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn tray_menu(hwnd: HWND) -> i32 {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        let (open, exit) = (HSTRING::from(t().open), HSTRING::from(t().exit));
        let _ = AppendMenuW(menu, MF_STRING, 1, &open);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, 2, &exit);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd); // without this the menu doesn't close when clicking outside
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None);
        let _ = DestroyMenu(menu);
        id.0
    }
}

/// Single instance: if another one is already open, ask it to show its window and return
/// false. Also prepares the process (per-monitor DPI and COM for audio).
pub fn claim_single_instance() -> bool {
    unsafe {
        let Ok(mutex) = CreateMutexW(None, true, w!("Local\\Nogenuity-App")) else { return true };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(hwnd) = FindWindowW(CLASS, PCWSTR::null()) {
                let _ = PostMessageW(Some(hwnd), WM_SHOWAPP, WPARAM(0), LPARAM(0));
            }
            return false;
        }
        let _ = mutex; // the handle is never closed: the mutex lives until the process exits
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    true
}

pub fn run(start_hidden: bool) {
    if let Err(e) = run_window(start_hidden) {
        panic!("could not create the window: {e}");
    }
}

fn run_window(start_hidden: bool) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let big_icon = make_icon(32);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hIcon: big_icon,
            hIconSm: make_icon(16),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        wm_taskbar_created(); // registered before the window that receives it exists
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            CLASS,
            w!("Nogenuity"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            700,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // Dark title bar, matching the background color
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        let caption: u32 = 0x000D0B0B; // COLORREF 0x00BBGGRR
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);

        // Exact client area size for the monitor DPI (based on the cards), centered
        let dpi = GetDpiForWindow(hwnd);

        let mut app = App::new(hwnd)?;
        app.dpi = dpi as f32;
        app.fit_window();
        let mut rc = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rc);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let _ = SetWindowPos(hwnd, None, (sw - ww) / 2, (sh - wh) / 2, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
        app.add_tray();
        let cell = Box::new(RefCell::new(app));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*cell as *const _ as isize);

        if start_hidden {
            let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
        } else {
            cell.borrow().show();
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        drop(cell);
    }
    Ok(())
}
