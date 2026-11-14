//! Renders icons and builds the layout ZIP the deck expects (manifest.json + Images/*.png).

use crate::config::{Button, Config, COLS, ROWS};
use ab_glyph::{point, Font, FontVec, Glyph, PxScale, ScaleFont};
use image::{imageops::FilterType, ImageFormat, Rgba, RgbaImage};
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

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
fn draw_glyph(img: &mut RgbaImage, font: &FontVec, text: &str, color: Rgba<u8>) -> Result<(), String> {
    let w = img.width();
    let (mut size, max_w) = (110.0f32, w as f32 - 36.0); // 160 on a normal tile
    let (mut glyphs, mut b) = layout_glyphs(font, text, size)?;
    if b[2] - b[0] > max_w {
        size *= max_w / (b[2] - b[0]);
        (glyphs, b) = layout_glyphs(font, text, size)?;
    }
    let dx = w as f32 / 2.0 - (b[0] + b[2]) / 2.0;
    let dy = 78.0 - (b[1] + b[3]) / 2.0; // label text sits along the bottom
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

fn render_icon(b: Option<&Button>, cfg: &Config, font: Option<&FontVec>, w: u32) -> Result<Vec<u8>, String> {
    if let Some(path) = b.and_then(|b| b.icon.as_deref()) {
        let full = cfg.base_dir.join(path);
        let img = image::open(&full).map_err(|e| format!("icon {}: {e}", full.display()))?;
        return png_bytes(&img.resize_exact(w, ICON, FilterType::Lanczos3).to_rgba8());
    }
    let color = parse_color(b.and_then(|b| b.color.as_deref()).unwrap_or(EMPTY_COLOR))?;
    let mut img = RgbaImage::from_pixel(w, ICON, color);
    if let (Some(b), Some(font)) = (b, font) {
        if let Some(glyph) = b.glyph.as_deref() {
            let gc = parse_color(b.glyph_color.as_deref().unwrap_or("#ffffff"))?;
            draw_glyph(&mut img, font, glyph, gc).map_err(|e| format!("button {:?}: {e}", b.pos))?;
        }
    }
    png_bytes(&img)
}

fn load_font(cfg: &Config) -> Result<Option<FontVec>, String> {
    if !cfg.buttons.iter().any(|b| b.glyph.is_some()) {
        return Ok(None);
    }
    let bytes = std::fs::read(&cfg.glyph_font).map_err(|e| format!("glyph_font {}: {e}", cfg.glyph_font))?;
    FontVec::try_from_vec(bytes).map(Some).map_err(|e| format!("glyph_font {}: {e}", cfg.glyph_font))
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

/// A one-cell update (for command 0x000D): same ZIP layout as the stock app's partial updates.
/// With `rim` the tile gets a colored border; with `None` it is the normal tile (identical to the full layout's image,
/// so it has the same hashed filename).
pub fn cell_zip(cfg: &Config, page: u8, col: u8, row: u8, rim: Option<([u8; 3], u32, u32)>) -> Result<Vec<u8>, String> {
    let glyph_font = load_font(cfg)?;
    let b = cfg.button_at(page, col, row);
    let mut png = render_icon(b, cfg, glyph_font.as_ref(), cell_width(col, row))?;
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
    let text = b.and_then(|b| b.label.as_deref()).unwrap_or("");
    let mut cell = json!({"State":0,"ViewParam":[{"Font":font(),"Icon":name,"Text":text}]});
    if (col, row) == (3, 2) {
        cell["SmallViewMode"] = json!(2); // the wide slot shows our picture instead of the firmware clock
    }
    let manifest = json!({ format!("{col}_{row}"): cell });
    zip.start_file("manifest.json", opts).map_err(|e| e.to_string())?;
    zip.write_all(serde_json::to_string(&manifest).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

/// Full layout: every cell is described so the deck never shows stale content.
pub fn build_zip(cfg: &Config, page: u8) -> Result<Vec<u8>, String> {
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
            let b = cfg.button_at(page, col, row);
            if row == 2 && col == 3 && b.is_none() {
                // no button on the wide slot: keep the firmware clock
                manifest.insert(key, json!({"SmallViewMode":1,"State":0,"ViewParam":[{"Font":font(),"Text":""}]}));
                continue;
            }
            // The deck keeps the first image it unpacked under a given name and ignores later ones, so the
            // filename must change whenever the picture does (the stock app uses a fresh uuid each time).
            let png = render_icon(b, cfg, glyph_font.as_ref(), cell_width(col, row))?;
            let name = format!("Images/{col}_{row}_{:016x}.png", fnv1a(&png));
            zip.start_file(&name, opts).map_err(|e| e.to_string())?;
            zip.write_all(&png).map_err(|e| e.to_string())?;
            let text = b.and_then(|b| b.label.as_deref()).unwrap_or("");
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
