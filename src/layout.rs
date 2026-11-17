//! Renders icons and builds the layout ZIP the deck expects (manifest.json + Images/*.png).

use crate::config::{Button, Config, Face, COLS, ROWS};
use crate::sysstats::Stats;
use ab_glyph::{point, Font, FontVec, Glyph, PxScale, ScaleFont};
use image::{imageops::FilterType, ImageFormat, Rgba, RgbaImage};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

/// Everything that changes from one redraw to the next: the time, the system load and the animation clock.
#[derive(Clone)]
pub struct Live {
    pub now: chrono::DateTime<chrono::Local>,
    pub stats: Stats,
    /// Milliseconds on the animation clock (decides which GIF frame is shown).
    pub tick_ms: u64,
    /// Per button index: is its state "on" (the alternate look is shown)?
    pub active: [bool; 14],
}

impl Live {
    /// Made-up load history so `deckd preview` shows what the stats faces look like without sampling the machine.
    pub fn demo() -> Live {
        let wave = |phase: f32, base: f32| (0..60).map(|i| (base + 30.0 * ((i as f32 + phase) / 6.0).sin()).clamp(2.0, 98.0)).collect::<Vec<f32>>();
        let (hist_cpu, hist_mem) = (wave(0.0, 40.0), wave(20.0, 62.0));
        Live {
            now: chrono::Local::now(),
            stats: Stats { cpu: *hist_cpu.last().unwrap_or(&0.0), mem: *hist_mem.last().unwrap_or(&0.0), hist_cpu, hist_mem },
            tick_ms: 0,
            active: [false; 14],
        }
    }

    /// A snapshot for offline rendering (previews, full layouts built outside the service).
    pub fn snapshot() -> Live {
        Live { now: chrono::Local::now(), stats: Stats::default(), tick_ms: 0, active: [false; 14] }
    }
}

// ---- GIF animation ----
struct Gif {
    mtime: Option<std::time::SystemTime>,
    frames: Vec<(RgbaImage, u64)>, // picture, delay in ms
    total_ms: u64,
}

const GIF_MAX_FRAMES: usize = 400;
const GIF_MIN_DELAY_MS: u64 = 100; // the deck cannot take updates faster than this comfortably

fn gif_cache() -> &'static Mutex<HashMap<(String, u32), Arc<Gif>>> {
    static C: std::sync::OnceLock<Mutex<HashMap<(String, u32), Arc<Gif>>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn is_gif(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".gif")
}

fn load_gif(full: &std::path::Path, w: u32) -> Result<Arc<Gif>, String> {
    use image::AnimationDecoder;
    let key = (full.display().to_string(), w);
    let mtime = std::fs::metadata(full).and_then(|m| m.modified()).ok();
    if let Some(g) = gif_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if g.mtime == mtime {
            return Ok(g.clone());
        }
    }
    let file = std::fs::File::open(full).map_err(|e| format!("gif {}: {e}", full.display()))?;
    let dec = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(file)).map_err(|e| format!("gif {}: {e}", full.display()))?;
    let mut frames = Vec::new();
    for f in dec.into_frames().take(GIF_MAX_FRAMES) {
        let f = f.map_err(|e| format!("gif {}: {e}", full.display()))?;
        let (n, d) = f.delay().numer_denom_ms();
        let ms = if d == 0 { 100 } else { (n as u64 / d as u64).max(GIF_MIN_DELAY_MS) };
        let img = image::imageops::resize(f.buffer(), w, ICON, FilterType::Triangle);
        frames.push((img, ms));
    }
    if frames.is_empty() {
        return Err(format!("gif {}: no frames", full.display()));
    }
    let total_ms = frames.iter().map(|f| f.1).sum();
    let g = Arc::new(Gif { mtime, frames, total_ms });
    gif_cache().lock().unwrap_or_else(|e| e.into_inner()).insert(key, g.clone());
    Ok(g)
}

/// Frame index and how long it stays up (ms) for a GIF at animation time `tick_ms`.
fn gif_at(g: &Gif, tick_ms: u64) -> (usize, u64) {
    let mut t = tick_ms % g.total_ms;
    for (i, (_, d)) in g.frames.iter().enumerate() {
        if t < *d {
            return (i, *d - t);
        }
        t -= *d;
    }
    (0, g.frames[0].1)
}

/// For an animated GIF button: the index of the frame shown at `tick_ms` (None = not an animation, or a single frame).
pub fn gif_frame_index(b: &Button, cfg: &Config, wide: bool, tick_ms: u64) -> Option<usize> {
    let path = b.icon.as_deref().filter(|p| is_gif(p) && !b.face.is_live())?;
    let g = load_gif(&cfg.base_dir.join(path), if wide { WIDE_W } else { ICON }).ok()?;
    (g.frames.len() > 1).then(|| gif_at(&g, tick_ms).0)
}

/// For the window: which GIF frame shows at `tick_ms` and how many ms it stays up. None when unreadable.
pub fn gif_index_at(full: &std::path::Path, tick_ms: u64) -> Option<(usize, u64)> {
    let g = load_gif(full, ICON).ok()?;
    Some(gif_at(&g, tick_ms))
}

/// One frame of a GIF (square tile size).
pub fn gif_frame_image(full: &std::path::Path, idx: usize) -> Option<RgbaImage> {
    let g = load_gif(full, ICON).ok()?;
    g.frames.get(idx).map(|f| f.0.clone())
}

// ---- drawing helpers for live faces ----
fn fill_rect(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba<u8>) {
    for y in y0.max(0)..y1.min(img.height() as i32) {
        for x in x0.max(0)..x1.min(img.width() as i32) {
            img.put_pixel(x as u32, y as u32, c);
        }
    }
}

fn thick_line(img: &mut RgbaImage, a: (f32, f32), b: (f32, f32), c: Rgba<u8>) {
    let steps = (b.0 - a.0).abs().max((b.1 - a.1).abs()).ceil().max(1.0) as i32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        fill_rect(img, x as i32 - 1, y as i32 - 1, x as i32 + 1, y as i32 + 1, c);
    }
}

/// Draw one line of text with its left edge at `x` and its vertical middle at `yc`.
fn text_left(img: &mut RgbaImage, font: &FontVec, text: &str, size: f32, c: Rgba<u8>, x: f32, yc: f32) -> Result<(), String> {
    let (glyphs, b) = layout_glyphs(font, text, size)?;
    blit(img, font, glyphs, c, x - b[0], yc - (b[1] + b[3]) / 2.0);
    Ok(())
}

fn blit(img: &mut RgbaImage, font: &FontVec, glyphs: Vec<Glyph>, c: Rgba<u8>, dx: f32, dy: f32) {
    let (w, h) = img.dimensions();
    for g in glyphs {
        if let Some(o) = font.outline_glyph(g) {
            let r = o.px_bounds();
            o.draw(|x, y, cov| {
                let (px, py) = ((r.min.x + x as f32 + dx) as i32, (r.min.y + y as f32 + dy) as i32);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    for i in 0..3 {
                        p.0[i] = (p.0[i] as f32 * (1.0 - cov) + c.0[i] as f32 * cov) as u8;
                    }
                }
            });
        }
    }
}

/// Draw (possibly multi-line) text centered on (cx, cy), shrinking it until it fits max_w x max_h.
fn text_block(img: &mut RgbaImage, font: &FontVec, text: &str, c: Rgba<u8>, cx: f32, cy: f32, max_w: f32, max_h: f32, start: f32) -> Result<(), String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut size = start;
    loop {
        let mut laid = Vec::new();
        let mut widest = 0.0f32;
        for l in &lines {
            if l.trim().is_empty() {
                laid.push(None);
                continue;
            }
            let (g, b) = layout_glyphs(font, l, size)?;
            widest = widest.max(b[2] - b[0]);
            laid.push(Some((g, b)));
        }
        let line_h = size * 1.1;
        let total = line_h * lines.len() as f32;
        if (widest <= max_w && total <= max_h) || size <= 8.0 {
            for (i, item) in laid.into_iter().enumerate() {
                if let Some((g, b)) = item {
                    let yc = cy - total / 2.0 + line_h * (i as f32 + 0.5);
                    blit(img, font, g, c, cx - (b[0] + b[2]) / 2.0, yc - (b[1] + b[3]) / 2.0);
                }
            }
            return Ok(());
        }
        size *= 0.92;
    }
}

fn need_font<'a>(font: Option<&'a FontVec>, b: &Button) -> Result<&'a FontVec, String> {
    font.ok_or_else(|| format!("button {:?}: live faces need a readable glyph_font", b.pos))
}

fn colors(b: &Button) -> Result<(Rgba<u8>, Rgba<u8>), String> {
    Ok((parse_color(b.color.as_deref().unwrap_or(EMPTY_COLOR))?, parse_color(b.glyph_color.as_deref().unwrap_or("#ffffff"))?))
}

fn render_live_clock(b: &Button, font: Option<&FontVec>, w: u32, live: &Live) -> Result<Vec<u8>, String> {
    let font = need_font(font, b)?;
    let (bg, fg) = colors(b)?;
    let mut img = RgbaImage::from_pixel(w, ICON, bg);
    let text = live.now.format(b.clock_format.as_deref().unwrap_or("%H:%M")).to_string();
    text_block(&mut img, font, &text, fg, w as f32 / 2.0, 82.0, w as f32 - 24.0, 140.0, 130.0).map_err(|e| format!("button {:?}: {e}", b.pos))?;
    png_bytes(&img)
}

fn render_weather(b: &Button, cfg: &Config, font: Option<&FontVec>, w: u32) -> Result<Vec<u8>, String> {
    let font = need_font(font, b)?;
    let (bg, fg) = colors(b)?;
    let mut img = RgbaImage::from_pixel(w, ICON, bg);
    let dim = Rgba([fg.0[0], fg.0[1], fg.0[2], 255]);
    let (cx, big_w) = (w as f32 / 2.0, w as f32 - 24.0);
    if cfg.weather_city.trim().is_empty() {
        text_block(&mut img, font, "Set a city\nin Settings", dim, cx, 98.0, big_w, 150.0, 40.0).map_err(|e| format!("button {:?}: {e}", b.pos))?;
    } else if let Some(wx) = crate::weather::current(&cfg.weather_city) {
        let (sym, words) = crate::weather::describe(wx.code, wx.is_day);
        let temp = if cfg.weather_fahrenheit { format!("{:.0}°F", wx.temp_c * 9.0 / 5.0 + 32.0) } else { format!("{:.0}°C", wx.temp_c) };
        text_block(&mut img, font, &format!("{sym} {temp}"), fg, cx, 70.0, big_w, 100.0, 110.0).map_err(|e| format!("button {:?}: {e}", b.pos))?;
        text_block(&mut img, font, &format!("{words} · {}", wx.place), dim, cx, 150.0, big_w, 36.0, 30.0).map_err(|e| format!("button {:?}: {e}", b.pos))?;
    } else {
        text_block(&mut img, font, "Weather…", dim, cx, 98.0, big_w, 60.0, 44.0).map_err(|e| format!("button {:?}: {e}", b.pos))?;
    }
    png_bytes(&img)
}

/// Time (big), date and weather in one picture. The clock format only changes the time line.
fn render_everything(b: &Button, cfg: &Config, font: Option<&FontVec>, w: u32, live: &Live) -> Result<Vec<u8>, String> {
    let font = need_font(font, b)?;
    let (bg, fg) = colors(b)?;
    let mut img = RgbaImage::from_pixel(w, ICON, bg);
    let cx = w as f32 / 2.0;
    let big_w = w as f32 - 24.0;
    let err = |e: String| format!("button {:?}: {e}", b.pos);
    // Time and date are one big block, both lines the same size. A clock format that already contains a date ("time + date")
    // is drawn as is; otherwise the date is added as the second line.
    let fmt = b.clock_format.as_deref().filter(|f| !f.trim().is_empty()).unwrap_or("%H:%M");
    let has_date = ["%a", "%A", "%d", "%e", "%b", "%B", "%m", "%y", "%Y", "%j", "%D", "%F", "%x", "%c"].iter().any(|c| fmt.contains(c));
    let time = live.now.format(&if has_date { fmt.to_string() } else { format!("{fmt}\n%a %d %b") }).to_string();
    let weather = if cfg.weather_city.trim().is_empty() {
        None
    } else {
        Some(match crate::weather::current(&cfg.weather_city) {
            Some(wx) => {
                let (sym, words) = crate::weather::describe(wx.code, wx.is_day);
                let temp = if cfg.weather_fahrenheit { format!("{:.0}°F", wx.temp_c * 9.0 / 5.0 + 32.0) } else { format!("{:.0}°C", wx.temp_c) };
                format!("{sym} {temp} · {words}")
            }
            None => "Weather…".to_string(),
        })
    };
    if let Some(wl) = weather {
        text_block(&mut img, font, &time, fg, cx, 60.0, big_w, 116.0, 110.0).map_err(err)?;
        text_block(&mut img, font, &wl, fg, cx, 152.0, big_w, 62.0, 56.0).map_err(err)?;
    } else {
        text_block(&mut img, font, &time, fg, cx, 98.0, big_w, 180.0, 130.0).map_err(err)?;
    }
    png_bytes(&img)
}

fn render_live_stats(b: &Button, font: Option<&FontVec>, w: u32, live: &Live) -> Result<Vec<u8>, String> {
    let font = need_font(font, b)?;
    let (bg, fg) = colors(b)?;
    let custom = b.glyph_color.is_some();
    let cpu_c = if custom { fg } else { Rgba([0x58, 0xa6, 0xff, 255]) };
    let mem_c = if custom { fg } else { Rgba([0xd2, 0x99, 0x22, 255]) };
    let track = Rgba([bg.0[0].saturating_add(40), bg.0[1].saturating_add(40), bg.0[2].saturating_add(40), 255]);
    let mut img = RgbaImage::from_pixel(w, ICON, bg);
    let wf = w as f32;
    let (cpu_t, mem_t) = (format!("CPU {:.0}%", live.stats.cpu), format!("RAM {:.0}%", live.stats.mem));
    let err = |e: String| format!("button {:?}: {e}", b.pos);
    match b.stats_style.as_deref().unwrap_or("bars") {
        "text" => {
            text_block(&mut img, font, &format!("{cpu_t}\n{mem_t}"), fg, wf / 2.0, 90.0, wf - 24.0, 150.0, 60.0).map_err(err)?;
        }
        "line" => {
            text_left(&mut img, font, &cpu_t, 26.0, cpu_c, 14.0, 22.0).map_err(err)?;
            text_left(&mut img, font, &mem_t, 26.0, mem_c, wf / 2.0 + 4.0, 22.0).map_err(err)?;
            let (x0, x1, y0, y1) = (14.0f32, wf - 14.0, 46.0f32, 182.0f32);
            for p in [0.0f32, 0.5, 1.0] {
                let y = (y1 - (y1 - y0) * p) as i32;
                fill_rect(&mut img, x0 as i32, y, x1 as i32, y + 1, track);
            }
            for (hist, c) in [(&live.stats.hist_cpu, cpu_c), (&live.stats.hist_mem, mem_c)] {
                let n = 60usize;
                let pts: Vec<(f32, f32)> = hist
                    .iter()
                    .rev()
                    .take(n)
                    .enumerate()
                    .map(|(i, v)| (x1 - (x1 - x0) * i as f32 / (n - 1) as f32, y1 - (y1 - y0) * (v / 100.0).clamp(0.0, 1.0)))
                    .collect();
                for pair in pts.windows(2) {
                    thick_line(&mut img, pair[0], pair[1], c);
                }
            }
        }
        _ => {
            for (i, (t, v, c)) in [(&cpu_t, live.stats.cpu, cpu_c), (&mem_t, live.stats.mem, mem_c)].into_iter().enumerate() {
                let top = 12.0 + i as f32 * 90.0;
                text_left(&mut img, font, t, 34.0, fg, 14.0, top + 20.0).map_err(err)?;
                let (x0, x1, y0) = (14, w as i32 - 14, top as i32 + 48);
                fill_rect(&mut img, x0, y0, x1, y0 + 26, track);
                fill_rect(&mut img, x0, y0, x0 + ((x1 - x0) as f32 * (v / 100.0).clamp(0.0, 1.0)) as i32, y0 + 26, c);
            }
        }
    }
    png_bytes(&img)
}

const ICON: u32 = 196;
const WIDE_W: u32 = 458;
const EMPTY_COLOR: &str = "#202020";

/// FNV-1a 64-bit: a stable content hash for filenames (not for security).
fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn dos_now() -> zip::DateTime {
    use chrono::{Datelike, Timelike};
    let n = chrono::Local::now();
    zip::DateTime::from_date_and_time(n.year() as u16, n.month() as u8, n.day() as u8, n.hour() as u8, n.minute() as u8, n.second() as u8)
        .unwrap_or_default()
}

fn font() -> Value {
    json!({"Align":"bottom","Color":16777215,"FontName":"Source Han Sans SC","ShowTitle":true,"Size":10,"Weight":80})
}

fn parse_color(s: &str) -> Result<Rgba<u8>, String> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return Err(format!("bad color {s:?}: use #rrggbb"));
    }
    let v = u32::from_str_radix(h, 16).map_err(|_| format!("bad color {s:?}: use #rrggbb"))?;
    Ok(Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]))
}

fn png_bytes(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Lay out `text` at `size` px; returns the glyphs and their pixel bounds (min_x, min_y, max_x, max_y).
fn layout_glyphs(font: &FontVec, text: &str, size: f32) -> Result<(Vec<Glyph>, [f32; 4]), String> {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let (mut x, mut glyphs) = (0.0f32, Vec::new());
    for c in text.chars() {
        let id = font.glyph_id(c);
        if id.0 == 0 {
            return Err(format!("font has no glyph for {c:?}; pick another glyph or set glyph_font"));
        }
        glyphs.push(id.with_scale_and_position(scale, point(x, scaled.ascent())));
        x += scaled.h_advance(id);
    }
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for g in &glyphs {
        if let Some(o) = font.outline_glyph(g.clone()) {
            let r = o.px_bounds();
            b = [b[0].min(r.min.x), b[1].min(r.min.y), b[2].max(r.max.x), b[3].max(r.max.y)];
        }
    }
    if b[0] > b[2] {
        return Err(format!("glyph {text:?} has no visible outline"));
    }
    Ok((glyphs, b))
}

/// Draw `text` centered in the area above the label line, shrinking it if it is too wide.
/// With no label under it (`centered`) the symbol sits in the middle of the tile instead of above the label line.
fn draw_glyph(img: &mut RgbaImage, font: &FontVec, text: &str, color: Rgba<u8>, centered: bool) -> Result<(), String> {
    let w = img.width();
    let (mut size, max_w) = (110.0f32, w as f32 - 36.0); // 160 on a normal tile
    let (mut glyphs, mut b) = layout_glyphs(font, text, size)?;
    if b[2] - b[0] > max_w {
        size *= max_w / (b[2] - b[0]);
        (glyphs, b) = layout_glyphs(font, text, size)?;
    }
    let dx = w as f32 / 2.0 - (b[0] + b[2]) / 2.0;
    let dy = if centered { ICON as f32 / 2.0 } else { 78.0 } - (b[1] + b[3]) / 2.0; // label text sits along the bottom
    for g in glyphs {
        if let Some(o) = font.outline_glyph(g) {
            let r = o.px_bounds();
            o.draw(|x, y, cov| {
                let (px, py) = ((r.min.x + x as f32 + dx) as i32, (r.min.y + y as f32 + dy) as i32);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < ICON {
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    for i in 0..3 {
                        p.0[i] = (p.0[i] as f32 * (1.0 - cov) + color.0[i] as f32 * cov) as u8;
                    }
                }
            });
        }
    }
    Ok(())
}

/// Picture width for a cell: the wide slot (3,2) is 458 px wide, every other cell 196.
fn cell_width(col: u8, row: u8) -> u32 {
    if (col, row) == (3, 2) { WIDE_W } else { ICON }
}

fn render_icon(b: Option<&Button>, cfg: &Config, font: Option<&FontVec>, w: u32, live: &Live) -> Result<Vec<u8>, String> {
    if let Some(b) = b {
        match b.face {
            Face::LiveClock => return render_live_clock(b, font, w, live),
            Face::LiveStats => return render_live_stats(b, font, w, live),
            Face::LiveWeather => return render_weather(b, cfg, font, w),
            Face::Everything => return render_everything(b, cfg, font, w, live),
            // alternate every 6 s: clock, then weather
            Face::ClockWeather if (live.now.timestamp() / 6) % 2 == 1 => return render_weather(b, cfg, font, w),
            Face::ClockWeather => return render_live_clock(b, font, w, live),
            _ => {}
        }
        if let Some(path) = b.icon.as_deref().filter(|p| is_gif(p)) {
            // animated: show the frame for the current animation time
            let g = load_gif(&cfg.base_dir.join(path), w)?;
            return png_bytes(&g.frames[gif_at(&g, live.tick_ms).0].0);
        }
    }
    if let Some(path) = b.and_then(|b| b.icon.as_deref()) {
        let full = cfg.base_dir.join(path);
        let img = image::open(&full).map_err(|e| format!("icon {}: {e}", full.display()))?;
        return png_bytes(&img.resize_exact(w, ICON, FilterType::Lanczos3).to_rgba8());
    }
    let color = parse_color(b.and_then(|b| b.color.as_deref()).unwrap_or(EMPTY_COLOR))?;
    let mut img = RgbaImage::from_pixel(w, ICON, color);
    if let Some(b) = b {
        let has_label = has_label(b);
        if let (Some(glyph), Some(font)) = (b.glyph.as_deref(), font) {
            let gc = parse_color(b.glyph_color.as_deref().unwrap_or("#ffffff"))?;
            draw_glyph(&mut img, font, glyph, gc, !has_label).map_err(|e| format!("button {:?}: {e}", b.pos))?;
        } else if b.glyph.is_none() && b.face.is_image() {
            if let Some(app) = crate::appicon::image_for(b) {
                if b.color.is_none() {
                    if let Some(c) = crate::appicon::plate_color(&app) {
                        img = RgbaImage::from_pixel(w, ICON, Rgba([c[0], c[1], c[2], 255])); // the icon's own plate becomes the tile
                    }
                }
                draw_app_icon(&mut img, &app, has_label);
            } else if let (true, Some(font)) = (has_label, font) {
                let gc = parse_color(b.glyph_color.as_deref().unwrap_or("#ffffff"))?;
                draw_centered_label(&mut img, font, b.label.as_deref().unwrap_or(""), gc)?;
            }
        }
    }
    png_bytes(&img)
}

fn has_label(b: &Button) -> bool {
    b.label.as_deref().is_some_and(|l| !l.trim().is_empty())
}

/// Does deckd paint this button's label into the picture (text-only button, centered) instead of the firmware drawing it
/// along the bottom? Then the manifest's Text must stay empty so it is not drawn twice.
fn label_in_picture(b: Option<&Button>, font: Option<&FontVec>) -> bool {
    let Some(b) = b else { return false };
    font.is_some() && has_label(b) && b.icon.is_none() && b.glyph.is_none() && b.face.is_image() && crate::appicon::image_for(b).is_none()
}

/// The text the firmware should draw under the picture.
fn firmware_text<'a>(b: Option<&'a Button>, font: Option<&FontVec>) -> &'a str {
    if label_in_picture(b, font) { "" } else { b.and_then(|b| b.label.as_deref()).unwrap_or("") }
}

/// A program's icon in the middle of the tile (above the label line when there is a label).
fn draw_app_icon(img: &mut RgbaImage, app: &RgbaImage, has_label: bool) {
    let (side, cy) = if has_label { (120u32, 74u32) } else { (148u32, ICON / 2) };
    let side = side.min(app.width().max(app.height()) * 4); // do not blow tiny icons up absurdly
    let scaled = image::imageops::resize(app, side, side, FilterType::Lanczos3);
    let x = (img.width() as i64 - side as i64) / 2;
    image::imageops::overlay(img, &scaled, x, cy as i64 - side as i64 / 2);
}

/// A label with no picture: large, in the middle of the tile, wrapped on spaces, shrunk to fit.
fn draw_centered_label(img: &mut RgbaImage, font: &FontVec, text: &str, c: Rgba<u8>) -> Result<(), String> {
    let (max_w, max_h) = (img.width() as f32 - 28.0, ICON as f32 - 36.0);
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut size = 52.0f32;
    loop {
        // greedy word wrap at this size
        let mut lines: Vec<String> = Vec::new();
        let mut fits = true;
        for w in &words {
            let cand = match lines.last() {
                Some(l) => format!("{l} {w}"),
                None => w.to_string(),
            };
            if lines.last().is_some() && text_w(font, &cand, size)? > max_w {
                lines.push(w.to_string());
            } else if lines.is_empty() {
                lines.push(cand);
            } else {
                *lines.last_mut().unwrap() = cand;
            }
        }
        for l in &lines {
            if text_w(font, l, size)? > max_w {
                fits = false;
            }
        }
        let total = size * 1.15 * lines.len() as f32;
        if (fits && total <= max_h) || size <= 10.0 {
            return text_block(img, font, &lines.join("\n"), c, img.width() as f32 / 2.0, ICON as f32 / 2.0, max_w + 200.0, max_h + 200.0, size);
        }
        size *= 0.92;
    }
}

fn text_w(font: &FontVec, text: &str, size: f32) -> Result<f32, String> {
    let b = layout_glyphs(font, text, size)?.1;
    Ok(b[2] - b[0])
}

/// The glyph font, read once and cached (live faces redraw every second or so).
fn load_font(cfg: &Config) -> Result<Option<Arc<FontVec>>, String> {
    load_font_for(cfg, &cfg.buttons)
}

fn load_font_for(cfg: &Config, buttons: &[Button]) -> Result<Option<Arc<FontVec>>, String> {
    let needs = |b: &Button| b.glyph.is_some() || b.face.is_live() || b.alt.as_ref().is_some_and(|a| a.glyph.is_some());
    let wants = |b: &Button| has_label(b) || b.alt.as_ref().is_some_and(|a| a.label.is_some());
    if !buttons.iter().any(|b| needs(b) || wants(b)) {
        return Ok(None);
    }
    let must = buttons.iter().any(needs);
    static CACHE: Mutex<Option<(String, Arc<FontVec>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((path, f)) = c.as_ref() {
        if *path == cfg.glyph_font {
            return Ok(Some(f.clone()));
        }
    }
    let loaded = std::fs::read(&cfg.glyph_font).map_err(|e| format!("glyph_font {}: {e}", cfg.glyph_font)).and_then(|bytes| FontVec::try_from_vec(bytes).map_err(|e| format!("glyph_font {}: {e}", cfg.glyph_font)));
    let f = match loaded {
        Ok(f) => Arc::new(f),
        Err(_) if !must => return Ok(None), // only labels want it: the firmware can still draw them
        Err(e) => return Err(e),
    };
    *c = Some((cfg.glyph_font.clone(), f.clone()));
    Ok(Some(f))
}

/// Paint a colored rim (border) of `width` px, fading inward, onto a rendered tile.
fn draw_rim(img: &mut RgbaImage, rgb: [u8; 3], width: u32, radius: u32) {
    let (w, h) = img.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let r = (radius as f32).min(cx.min(cy));
    let (hx, hy) = (cx - r, cy - r);
    for y in 0..h {
        for x in 0..w {
            // signed distance to a rounded rectangle: > 0 outside the shape, < 0 inside
            let (qx, qy) = ((x as f32 + 0.5 - cx).abs() - hx, (y as f32 + 0.5 - cy).abs() - hy);
            let outside = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
            let inside = -outside;
            // solid rim of `width` with a crisp edge (1 px antialiasing only)
            let a = (width as f32 + 0.5 - inside).clamp(0.0, 1.0);
            // outside the rounded corner the tile goes black (with a 1 px antialiased edge)
            let cover = (0.5 + inside).clamp(0.0, 1.0);
            let p = img.get_pixel_mut(x, y);
            for i in 0..3 {
                let colored = p.0[i] as f32 * (1.0 - a) + rgb[i] as f32 * a;
                p.0[i] = (colored * cover) as u8;
            }
        }
    }
}

/// Hash of the picture a cell would show (same content = same hash = same filename on the deck).
/// None for a cell the firmware draws itself.
pub fn cell_png_hash(cfg: &Config, page: u8, col: u8, row: u8, rim: Option<([u8; 3], u32, u32)>, live: &Live) -> Option<u64> {
    let glyph_font = load_font(cfg).ok()?;
    let shown = cfg.button_at(page, col, row).map(|b| b.shown(live.active[(row * COLS + col) as usize]));
    if shown.as_ref().is_some_and(|b| b.face.is_firmware()) || ((col, row) == (3, 2) && shown.is_none()) {
        return None;
    }
    let mut png = render_icon(shown.as_ref(), cfg, glyph_font.as_deref(), cell_width(col, row), live).ok()?;
    if let Some((rgb, width, radius)) = rim {
        let mut img = image::load_from_memory(&png).ok()?.to_rgba8();
        draw_rim(&mut img, rgb, width, radius);
        png = png_bytes(&img).ok()?;
    }
    Some(fnv1a(&png))
}

/// A one-cell update (for command 0x000D): same ZIP layout as the stock app's partial updates.
/// With `rim` the tile gets a colored border; with `None` it is the normal tile (identical to the full layout's image,
/// so it has the same hashed filename).
pub fn cell_zip(cfg: &Config, page: u8, col: u8, row: u8, rim: Option<([u8; 3], u32, u32)>, live: &Live) -> Result<Vec<u8>, String> {
    let glyph_font = load_font(cfg)?;
    let shown = cfg.button_at(page, col, row).map(|b| b.shown(live.active[(row * COLS + col) as usize]));
    let b = shown.as_ref();
    let mut png = render_icon(b, cfg, glyph_font.as_deref(), cell_width(col, row), live)?;
    if let Some((rgb, width, radius)) = rim {
        let mut img = image::load_from_memory(&png).map_err(|e| e.to_string())?.to_rgba8();
        draw_rim(&mut img, rgb, width, radius);
        png = png_bytes(&img)?;
    }
    let name = format!("Images/{col}_{row}_{:016x}.png", fnv1a(&png));
    let stamp = dos_now();
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).unix_permissions(0o666).last_modified_time(stamp);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.add_directory("Images/", SimpleFileOptions::default().unix_permissions(0o777).last_modified_time(stamp)).map_err(|e| e.to_string())?;
    zip.start_file(&name, opts).map_err(|e| e.to_string())?;
    zip.write_all(&png).map_err(|e| e.to_string())?;
    let text = firmware_text(b, glyph_font.as_deref());
    let mut cell = json!({"State":0,"ViewParam":[{"Font":font(),"Icon":name,"Text":text}]});
    if (col, row) == (3, 2) {
        cell["SmallViewMode"] = json!(2); // the wide slot shows our picture instead of the firmware clock
    }
    let manifest = json!({ format!("{col}_{row}"): cell });
    zip.start_file("manifest.json", opts).map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string(&manifest).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// The tile exactly as the deck gets it (PNG bytes), for the window's live preview. `b` may be an unsaved edit;
/// `alt` renders its alternate look. `wide` = the 458 px wide slot.
pub fn preview_png(cfg: &Config, b: &Button, wide: bool, alt: bool, live: &Live) -> Result<Vec<u8>, String> {
    let glyph_font = load_font_for(cfg, std::slice::from_ref(b))?;
    let shown = b.shown(alt);
    if shown.face.is_firmware() {
        return Err("the deck draws this itself".into());
    }
    render_icon(Some(&shown), cfg, glyph_font.as_deref(), if wide { WIDE_W } else { ICON }, live)
}

/// Full layout: every cell is described so the deck never shows stale content.
pub fn build_zip(cfg: &Config, page: u8) -> Result<Vec<u8>, String> {
    build_zip_live(cfg, page, &Live::snapshot())
}

/// Full layout with the live faces / GIF frames drawn for `live`.
pub fn build_zip_live(cfg: &Config, page: u8, live: &Live) -> Result<Vec<u8>, String> {
    let glyph_font = load_font(cfg)?;
    let mut manifest = serde_json::Map::new();
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    // The deck ignores images in a ZIP whose entries carry the zip crate's default 1980 timestamp
    // (verified: identical content with current timestamps shows all icons), so stamp the current time.
    let stamp = dos_now();
    let opts = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o666)
        .last_modified_time(stamp);
    // The stock app's ZIP also starts with an explicit "Images/" directory entry; keep it.
    zip.add_directory("Images/", SimpleFileOptions::default().unix_permissions(0o777).last_modified_time(stamp))
        .map_err(|e| e.to_string())?;

    for row in 0..ROWS {
        for col in 0..COLS {
            if row == 2 && col == 4 {
                continue; // covered by the wide clock slot
            }
            let key = format!("{col}_{row}");
            let shown = cfg.button_at(page, col, row).map(|b| b.shown(live.active[(row * COLS + col) as usize]));
            let b = shown.as_ref();
            if row == 2 && col == 3 && b.is_none_or(|b| b.face.is_firmware()) {
                // no button on the wide slot, or one that hands it to the firmware: clock (mode 1) or stats (mode 0), no picture
                let mode = if b.is_some_and(|b| b.face == Face::Stats) { 0 } else { 1 };
                manifest.insert(key, json!({"SmallViewMode":mode,"State":0,"ViewParam":[{"Font":font(),"Text":""}]}));
                continue;
            }
            // The deck keeps the first image it unpacked under a given name and ignores later ones, so the
            // filename must change whenever the picture does (the stock app uses a fresh uuid each time).
            let png = render_icon(b, cfg, glyph_font.as_deref(), cell_width(col, row), live)?;
            let name = format!("Images/{col}_{row}_{:016x}.png", fnv1a(&png));
            zip.start_file(&name, opts).map_err(|e| e.to_string())?;
            zip.write_all(&png).map_err(|e| e.to_string())?;
            let text = firmware_text(b, glyph_font.as_deref());
            let mut cell = json!({"State":0,"ViewParam":[{"Font":font(),"Icon":name,"Text":text}]});
            if row == 2 && col == 3 {
                cell["SmallViewMode"] = json!(2); // wide slot with our own picture (community docs: mode 2 = background image)
            }
            manifest.insert(key, cell);
        }
    }
    zip.start_file("manifest.json", opts).map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string(&manifest).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}
