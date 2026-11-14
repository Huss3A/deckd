//! The app icon, drawn in code (no image file to ship). Shared by the program (tray + window) and build.rs (exe icon).

/// RGBA pixels of the icon: a dark rounded square with a 3x2 grid of colored keys.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    let colors = [[0x4c, 0x9a, 0xff], [0xff, 0x9f, 0x43], [0x5f, 0xd3, 0x8d], [0xb5, 0x7b, 0xff], [0xff, 0x5f, 0x7a], [0x4c, 0xd4, 0xe0]];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let (dx, dy) = ((fx - 0.5).abs() - 0.5 + 0.18, (fy - 0.5).abs() - 0.5 + 0.18);
            let outside = dx.max(0.0).hypot(dy.max(0.0)) - 0.18;
            if outside > 0.0 {
                continue;
            }
            let mut c = [0x1c, 0x1f, 0x26];
            let (gx, gy) = ((fx - 0.14) / 0.72 * 3.0, (fy - 0.25) / 0.5 * 2.0);
            if (0.0..3.0).contains(&gx) && (0.0..2.0).contains(&gy) && gx.fract() > 0.12 && gx.fract() < 0.88 && gy.fract() > 0.14 && gy.fract() < 0.86 {
                c = colors[(gy as usize) * 3 + gx as usize];
            }
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    px
}

/// A .ico file with several sizes (32-bit BMP images), for the exe's icon resource.
#[allow(dead_code)]
pub fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let mut images: Vec<Vec<u8>> = Vec::new();
    for &n in sizes {
        let rgba = icon_rgba(n);
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&(n as i32).to_le_bytes());
        d.extend_from_slice(&((n * 2) as i32).to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&32u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        for y in (0..n).rev() {
            for x in 0..n {
                let i = ((y * n + x) * 4) as usize;
                d.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        d.extend(std::iter::repeat_n(0u8, (n * (n.div_ceil(32) * 4)) as usize)); // AND mask (alpha does the work)
        images.push(d);
    }
    let mut out = vec![0, 0, 1, 0];
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&n, img) in sizes.iter().zip(&images) {
        out.extend_from_slice(&[if n >= 256 { 0 } else { n as u8 }, if n >= 256 { 0 } else { n as u8 }, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend(img);
    }
    out
}
