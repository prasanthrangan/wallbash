// --------------------------------------------------------------------- / tittu
// wallbash
// a color generation module for HyDE
//


// --------------------------------------------------------------------- / imports

use image::DynamicImage;


// --------------------------------------------------------------------- / datatypes

pub struct ColorPalette {
    group: &'static str,
    name: &'static str,
    argb: u32,
}


// --------------------------------------------------------------------- / k‑means

pub fn dcol(img: &DynamicImage) -> u32 {
    let small = img.resize_exact(64, 64, image::imageops::FilterType::Nearest);
    let rgb = small.to_rgb8();

    let pixels: Vec<[f64; 3]> = rgb
        .pixels()
        .map(|p| [p[0] as f64, p[1] as f64, p[2] as f64])
        .collect();

    if pixels.is_empty() {
        return rgb_to_argb(128, 128, 128);
    }

    let k = 5;
    let max_iter = 8;
    let mut centroids = Vec::with_capacity(k);
    let mut lcg = 42u32;
    for _ in 0..k {
        lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
        centroids.push(pixels[lcg as usize % pixels.len()]);
    }

    let mut assignments = vec![0usize; pixels.len()];

    for _ in 0..max_iter {
        for (i, pixel) in pixels.iter().enumerate() {
            let mut best_dist = f64::MAX;
            let mut best_c = 0;
            for (c, centroid) in centroids.iter().enumerate() {
                let dr = pixel[0] - centroid[0];
                let dg = pixel[1] - centroid[1];
                let db = pixel[2] - centroid[2];
                let dist = dr * dr + dg * dg + db * db;
                if dist < best_dist { best_dist = dist; best_c = c; }
            }
            assignments[i] = best_c;
        }

        let mut sums = vec![[0.0f64; 3]; k];
        let mut counts = vec![0u32; k];
        for (i, &cluster) in assignments.iter().enumerate() {
            let p = pixels[i];
            sums[cluster][0] += p[0]; sums[cluster][1] += p[1]; sums[cluster][2] += p[2];
            counts[cluster] += 1;
        }
        for c in 0..k {
            if counts[c] > 0 {
                centroids[c][0] = sums[c][0] / counts[c] as f64;
                centroids[c][1] = sums[c][1] / counts[c] as f64;
                centroids[c][2] = sums[c][2] / counts[c] as f64;
            }
        }
    }

    let mut cluster_sizes = vec![0u32; k];
    for &a in &assignments { cluster_sizes[a] += 1; }
    let largest = cluster_sizes.iter().enumerate()
        .max_by_key(|&(_, &s)| s).map(|(i, _)| i).unwrap_or(0);

    let dom = centroids[largest];
    let r = dom[0].round().clamp(0.0, 255.0) as u8;
    let g = dom[1].round().clamp(0.0, 255.0) as u8;
    let b = dom[2].round().clamp(0.0, 255.0) as u8;
    rgb_to_argb(r, g, b)
}


// --------------------------------------------------------------------- / converters

fn rgb_to_argb(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

fn srgb_to_xyz(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let linear = |c: f64| -> f64 {
        if c <= 0.04045 { c / 12.92 }
        else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let rl = linear(r);
    let gl = linear(g);
    let bl = linear(b);

    (
        0.4124564 * rl + 0.3575761 * gl + 0.1804375 * bl,
        0.2126729 * rl + 0.7151522 * gl + 0.0721750 * bl,
        0.0193339 * rl + 0.1191920 * gl + 0.9503041 * bl,
    )
}

fn xyz_to_srgb(x: f64, y: f64, z: f64) -> (u8, u8, u8) {
    let r_lin =  3.2404542 * x - 1.5371385 * y - 0.4985314 * z;
    let g_lin = -0.9692660 * x + 1.8760108 * y + 0.0415560 * z;
    let b_lin =  0.0556434 * x - 0.2040259 * y + 1.0572252 * z;

    let delinearize = |c: f64| -> f64 {
        if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
    };

    (
        (delinearize(r_lin.clamp(0.0, 1.0)) * 255.0).round() as u8,
        (delinearize(g_lin.clamp(0.0, 1.0)) * 255.0).round() as u8,
        (delinearize(b_lin.clamp(0.0, 1.0)) * 255.0).round() as u8,
    )
}

fn rgb_to_cielab(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;

    let (x, y, z) = srgb_to_xyz(r, g, b);

    let xn = 0.95047;
    let yn = 1.0;
    let zn = 1.08883;

    let fx = (x / xn).powf(1.0 / 3.0);
    let fy = (y / yn).powf(1.0 / 3.0);
    let fz = (z / zn).powf(1.0 / 3.0);

    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b_val = 200.0 * (fy - fz);

    (l, a, b_val)
}

fn cielab_to_rgb(l: f64, a: f64, b: f64) -> (u8, u8, u8) {
    let yn = 1.0;
    let xn = 0.95047;
    let zn = 1.08883;

    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;

    let delta: f64 = 6.0 / 29.0;

    let x = if fx > delta { xn * fx.powi(3) } else { (fx - 16.0 / 116.0) * 3.0 * delta * delta * xn };
    let y = if l > 8.0  { yn * fy.powi(3) } else { l / 903.3 * yn };
    let z = if fz > delta { zn * fz.powi(3) } else { (fz - 16.0 / 116.0) * 3.0 * delta * delta * zn };

    xyz_to_srgb(x, y, z)
}


// --------------------------------------------------------------------- / generate palette

pub fn generate_palette(dcol: u32) -> (Vec<ColorPalette>, Vec<ColorPalette>) {
    let r = ((dcol >> 16) & 0xFF) as u8;
    let g = ((dcol >> 8) & 0xFF) as u8;
    let b = (dcol & 0xFF) as u8;

    let (_, a_star, b_star) = rgb_to_cielab(r, g, b);
    let hue_rad = b_star.atan2(a_star);
    let chroma = (a_star * a_star + b_star * b_star).sqrt();

    let roles: [(&str, &str, f64, f64, f64, f64); 27] = [
        ("Primary",   "Primary",             40.0,  80.0, 0.9,  0.0),
        ("Primary",   "On Primary",          100.0, 20.0, 0.0,  0.0),
        ("Primary",   "Primary Container",   90.0,  30.0, 0.7,  0.0),
        ("Primary",   "On Primary Cont.",    10.0,  90.0, 0.0,  2.0),
        ("Secondary", "Secondary",           40.0,  80.0, 0.5,  0.0),
        ("Secondary", "On Secondary",        100.0, 20.0, 0.0,  0.0),
        ("Secondary", "Secondary Container", 90.0,  30.0, 0.4,  0.0),
        ("Secondary", "On Secondary Cont.",  10.0,  90.0, 0.0,  2.0),
        ("Tertiary",  "Tertiary",            40.0,  80.0, 0.6,  0.0),
        ("Tertiary",  "On Tertiary",         100.0, 20.0, 0.0,  0.0),
        ("Tertiary",  "Tertiary Container",  90.0,  30.0, 0.5,  0.0),
        ("Tertiary",  "On Tertiary Cont.",   10.0,  90.0, 0.0,  2.0),
        ("Error",     "Error",               40.0,  80.0, 0.9,  0.0),
        ("Error",     "On Error",            100.0, 20.0, 0.0,  0.0),
        ("Error",     "Error Container",     90.0,  30.0, 0.7,  0.0),
        ("Error",     "On Error Cont.",      10.0,  90.0, 0.0,  2.0),
        ("Surface",   "Background",          98.0,  6.0,  0.05, 0.0),
        ("Surface",   "On Background",       10.0,  90.0, 0.0,  2.0),
        ("Surface",   "Surface",             98.0,  6.0,  0.05, 0.0),
        ("Surface",   "On Surface",          10.0,  90.0, 0.0,  2.0),
        ("Surface",   "Surface Variant",     90.0,  30.0, 0.2,  0.0),
        ("Surface",   "On Surface Variant",  30.0,  80.0, 0.0,  1.0),
        ("Surface",   "Outline",             50.0,  60.0, 0.1,  0.0),
        ("Surface",   "Shadow",              0.0,   0.0,  0.0,  0.0),
        ("Surface",   "Inverse Surface",     20.0,  90.0, 0.2,  0.0),
        ("Surface",   "Inverse On Surface",  95.0,  20.0, 0.0,  0.0),
        ("Surface",   "Inverse Primary",     80.0,  40.0, 0.4,  0.0),
    ];

    let mut light = Vec::new();
    let mut dark = Vec::new();

    for (group, name, light_tone, dark_tone, chroma_factor, min_chroma) in roles {
        let (h, c) = if name.starts_with("Error") {
            (0.436332, 45.0)
        } else if name.contains("Secondary") {
            (hue_rad, chroma * chroma_factor)
        } else if name.contains("Tertiary") {
            (hue_rad + 1.04719755, chroma * chroma_factor)
        } else {
            (hue_rad, chroma * chroma_factor)
        };

        // light theme
        let chroma_val = c.max(min_chroma);
        let a = chroma_val * h.cos();
        let b_val = chroma_val * h.sin();
        let (rr, gg, bb) = cielab_to_rgb(light_tone, a, b_val);
        light.push(ColorPalette { group, name, argb: rgb_to_argb(rr, gg, bb) });

        // dark theme
        let dark_min_chroma = if dark_tone <= 30.0 { 4.0 } else { 0.0 };
        let chroma_val = c.max(min_chroma).max(dark_min_chroma);
        let a = chroma_val * h.cos();
        let b_val = chroma_val * h.sin();
        let (rr, gg, bb) = cielab_to_rgb(dark_tone, a, b_val);
        dark.push(ColorPalette { group, name, argb: rgb_to_argb(rr, gg, bb) });
    }

    (light, dark)
}


// --------------------------------------------------------------------- / print palette

pub fn print_palette(dcol: u32, mode: &str) {
    let (light, dark) = generate_palette(dcol);
    let r = ((dcol >> 16) & 0xFF) as u8;
    let g = ((dcol >> 8) & 0xFF) as u8;
    let b = (dcol & 0xFF) as u8;
    let (_, _, l_star) = rgb_to_cielab(r, g, b);

    print!("\x1b[48;2;{};{};{}m  \x1b[0m", r, g, b);
    if mode == "light" || (mode == "auto" && l_star > 55.0) {
        println!("  #{:06X} :: Light     :: Dominant Color", dcol);
        group_palette(&light);
    } else {
        println!("  #{:06X} :: Dark      :: Dominant Color", dcol);
        group_palette(&dark);
    }
}

fn group_palette(palette: &[ColorPalette]) {
    for entry in palette {
        let r = ((entry.argb >> 16) & 0xFF) as u8;
        let g = ((entry.argb >> 8) & 0xFF) as u8;
        let b = (entry.argb & 0xFF) as u8;
        print!("\x1b[48;2;{};{};{}m  \x1b[0m", r, g, b);
        println!("  #{:06X} :: {:<9} :: {}", entry.argb, entry.group, entry.name);
    }
}

