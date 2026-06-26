// --------------------------------------------------------------------- / tittu
// wallbash
// a color module for HyDE
//


// --------------------------------------------------------------------- / imports

use image::DynamicImage;
use std::f64::consts::PI;


// --------------------------------------------------------------------- / datatypes

struct ColorPalette {
    group: &'static str,
    name:  &'static str,
    argb:  u32,
}

struct ViewingConditions {
    fl:    f64,        // luminance-level adaptation factor
    aw:    f64,        // achromatic response of the white point
    nb:    f64,        // chromatic induction factor (background)
    c:     f64,        // impact of lightness on chroma
    nc:    f64,        // chromatic induction factor (surround)
    n:     f64,        // relative luminance of background
    z:     f64,        // exponent for lightness
    rgb_d: [f64; 3],   // per channel chromatic adaptation
}

const VC: ViewingConditions = ViewingConditions {
    fl:    0.3885,
    aw:    30.20,
    nb:    1.0169,
    c:     0.69,
    nc:    1.0,
    n:     0.1842,
    z:     4.515,
    rgb_d: [1.0215, 0.9863, 0.9339],
};


// --------------------------------------------------------------------- / sRGB ⇆ linear

fn srgb_to_linear(c: f64) -> f64 {

    // decoding sRGB‑to‑linear for calculations (IEC 61966‑2‑1)
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f64) -> f64 {

    // encoding linear‑to‑sRGB for display (IEC 61966‑2‑1)
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}


// --------------------------------------------------------------------- / sRGB ⇆ XYZ

fn argb_to_xyz(argb: u32) -> [f64; 3] {

    // extract rgb bits from 0xAARRGGBB
    let r = srgb_to_linear(((argb >> 16) & 0xFF) as f64 / 255.0);
    let g = srgb_to_linear(((argb >>  8) & 0xFF) as f64 / 255.0);
    let b = srgb_to_linear(( argb        & 0xFF) as f64 / 255.0);

    // xyz color space, source to other colour models
    [
        100.0 * (0.41233895 * r + 0.35762064 * g + 0.18051042 * b), // red‑green sensitivity
        100.0 * (0.21267285 * r + 0.71516868 * g + 0.07215847 * b), // luminance (brightness)
        100.0 * (0.01933082 * r + 0.11919478 * g + 0.95053087 * b), // blue‑yellow sensitivity
    ]
}

fn xyz_to_argb(xyz: [f64; 3]) -> (u32, bool) {
    // normalize to 0‑1 range
    let [x, y, z] = [xyz[0] / 100.0, xyz[1] / 100.0, xyz[2] / 100.0];

    // inverse matrix → linear sRGB unclamped
    let rl =  3.2406254 * x - 1.5372080 * y - 0.4986286 * z;
    let gl = -0.9689307 * x + 1.8757561 * y + 0.0415175 * z;
    let bl =  0.0557101 * x - 0.2040211 * y + 1.0569959 * z;

    // determine if the colour was inside the sRGB gamut
    let in_gamut = rl >= -1e-4 && rl <= 1.0001
        && gl >= -1e-4 && gl <= 1.0001
        && bl >= -1e-4 && bl <= 1.0001;

    // clamp delinearise and round back to srgb
    let to_u8 = |c: f64| (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0).round() as u8;
    let (r, g, b) = (to_u8(rl), to_u8(gl), to_u8(bl));
    (rgb_to_argb(r, g, b), in_gamut)
}

fn rgb_to_argb(r: u8, g: u8, b: u8) -> u32 {

    // build opaque rgb bits as 0xFFRRGGBB
    0xFF_00_00_00 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
}

fn lstar_to_argb(lstar: f64) -> u32 {

    // normalie luminance to 0‑1 range for grey
    let y = lstar_to_y(lstar) / 100.0;

    // clamp and round back to srgb
    let c = (linear_to_srgb(y.clamp(0.0, 1.0)) * 255.0).round() as u8;
    rgb_to_argb(c, c, c)
}


// --------------------------------------------------------------------- / L* (lightness) ⇆ Y (luminance)

fn lstar_to_y(lstar: f64) -> f64 {

    // undo scaling and cuberoot for normal light colour
    if lstar > 8.0 { ((lstar + 16.0) / 116.0).powi(3) * 100.0 }

    // inverse slope and scale for very dark colour
    else { lstar / 903.3 * 100.0 }
}

fn y_to_lstar(y: f64) -> f64 {
    let yn = y / 100.0;

    // forward CIELab function
    let fy = if yn > 0.008856 { yn.cbrt() } else { 7.787 * yn + 16.0 / 116.0 };

    // offset and clamp
    (116.0 * fy - 16.0).clamp(0.0, 100.0)
}


// --------------------------------------------------------------------- / XYZ ⇆ LMS

fn m16_r(x: f64, y: f64, z: f64) -> f64 {  0.401288 * x + 0.650173 * y - 0.051461 * z } // cone responses long
fn m16_g(x: f64, y: f64, z: f64) -> f64 { -0.250268 * x + 1.204414 * y + 0.045854 * z } // cone responses medium
fn m16_b(x: f64, y: f64, z: f64) -> f64 { -0.002079 * x + 0.048952 * y + 0.953127 * z } // cone responses short

fn adapt(x: f64, fl: f64) -> f64 {

    // light adaptation for nonlinear response
    let p = (fl * x.abs() / 100.0).powf(0.42);
    x.signum() * 400.0 * p / (p + 27.13) + 0.1
}


// --------------------------------------------------------------------- / CAM16 ⇆ XYZ

fn xyz_to_cam16(xyz: [f64; 3], vc: &ViewingConditions) -> (f64, f64, f64) {
    let [x, y, z] = xyz;

    // chromatically adapted responses
    let ra = adapt(m16_r(x, y, z) * vc.rgb_d[0], vc.fl);
    let ga = adapt(m16_g(x, y, z) * vc.rgb_d[1], vc.fl);
    let ba = adapt(m16_b(x, y, z) * vc.rgb_d[2], vc.fl);

    // opponent channels
    let a = (11.0 * ra - 12.0 * ga + ba) / 11.0;
    let b_opp = (ra + ga - 2.0 * ba) / 9.0;

    // perceived brightness signal
    let p2 = (40.0 * ra + 20.0 * ga + ba) / 20.0;

    // hue angle
    let hue = (b_opp.atan2(a) * 180.0 / PI).rem_euclid(360.0);
    let h_rad = hue * PI / 180.0;

    // lightness
    let j = 100.0 * (vc.nb * p2 / vc.aw).powf(vc.c * vc.z);

    // colourfulness
    let e_hue = 0.25 * ((h_rad + 2.0).cos() + 3.8);
    let p1 = e_hue * (50000.0 / 13.0) * vc.nc * vc.nb;
    let t = p1 * (a * a + b_opp * b_opp).sqrt() / (p2 + 0.305);
    let alpha = t.powf(0.9) * (1.64 - 0.29_f64.powf(vc.n)).powf(0.73);
    let chroma = alpha * (j / 100.0).sqrt();

    (hue, chroma, j)
}

fn cam16_to_xyz(hue: f64, chroma: f64, j: f64, vc: &ViewingConditions) -> [f64; 3] {

    // handle edge cases
    if j < 1e-10 { return [0.0, 0.0, 0.0]; }

    // recompute supporting factors
    let alpha = if chroma < 1e-10 { 0.0 } else { chroma / (j / 100.0).sqrt() };
    let t = (alpha / (1.64 - 0.29_f64.powf(vc.n)).powf(0.73)).powf(1.0 / 0.9);
    let h_rad = hue * PI / 180.0;
    let e_hue = 0.25 * ((h_rad + 2.0).cos() + 3.8);
    let ac = vc.aw * (j / 100.0).powf(1.0 / (vc.c * vc.z));
    let p1 = e_hue * (50000.0 / 13.0) * vc.nc * vc.nb;
    let p2 = ac / vc.nb;
    let (hs, hc) = (h_rad.sin(), h_rad.cos());

    // recover opponent signals
    let gamma = 23.0 * (p2 + 0.305) * t / (23.0 * p1 + 11.0 * t * hc + 108.0 * t * hs);
    let a = gamma * hc;
    let b = gamma * hs;

    // recover adapted cone responses
    let ra = (460.0 * p2 + 451.0 * a + 288.0 * b) / 1403.0;
    let ga = (460.0 * p2 - 891.0 * a - 261.0 * b) / 1403.0;
    let ba = (460.0 * p2 - 220.0 * a - 6300.0 * b) / 1403.0;

    // inverse nonlinear response
    fn decomp(x: f64, fl: f64) -> f64 {
        let base = (27.13 * (x - 0.1).abs() / (400.0 - (x - 0.1).abs())).max(0.0);
        (x - 0.1).signum() * (100.0 / fl) * base.powf(1.0 / 0.42)
    }
    let r = decomp(ra, vc.fl) / vc.rgb_d[0];
    let g = decomp(ga, vc.fl) / vc.rgb_d[1];
    let b = decomp(ba, vc.fl) / vc.rgb_d[2];

    // convert LMS → XYZ using inverse M16 matrix
    [
         1.8620678 * r - 1.0112547 * g + 0.1491865 * b,
         0.3875265 * r + 0.6214474 * g - 0.0089739 * b,
        -0.0158415 * r - 0.0344560 * g + 1.0502915 * b,
    ]
}


// --------------------------------------------------------------------- / HCT ⇆ sRGB

fn hct_to_argb(hue: f64, chroma: f64, tone: f64, vc: &ViewingConditions) -> u32 {

    // handle edge case achromatic or near-white/black
    if chroma < 1e-4 || tone < 1e-4 || tone > 99.9999 {
        return lstar_to_argb(tone);
    }

    let y_tgt = lstar_to_y(tone);

    // find CAM16 lightness that matches tone (achromatic path)
    let (mut jlo, mut jhi) = (0.0_f64, 100.0_f64);
    for _ in 0..50 {
        let jm = (jlo + jhi) / 2.0;
        if cam16_to_xyz(hue, 0.0, jm, &vc)[1] < y_tgt { jlo = jm; } else { jhi = jm; }
    }
    let j = (jlo + jhi) / 2.0;

    // try the full chroma
    let (argb_full, in_gamut) = xyz_to_argb(cam16_to_xyz(hue, chroma, j, &vc));
    if in_gamut {
        return argb_full;
    }

    // lip chroma to gamut boundary via binary search
    let (mut clo, mut chi) = (0.0_f64, chroma);
    let mut best = lstar_to_argb(tone);
    for _ in 0..50 {
        let cm  = (clo + chi) / 2.0;
        let (argb, in_gamut) = xyz_to_argb(cam16_to_xyz(hue, cm, j, &vc));
        if in_gamut { clo = cm; best = argb; } else { chi = cm; }
    }
    best
}

fn argb_to_hct(argb: u32, vc: &ViewingConditions) -> (f64, f64, f64) {

    // convert and setup sRGB to XYZ
    let xyz = argb_to_xyz(argb);

    // convert XYZ to CAM16 hue and chroma
    let (hue, chroma, _j) = xyz_to_cam16(xyz, &vc);
    (hue, chroma, y_to_lstar(xyz[1]))
}


// --------------------------------------------------------------------- / CIELab ⇆ sRGB

fn argb_to_lab(argb: u32) -> [f64; 3] {

    // convert sRGB to XYZ
    let xyz = argb_to_xyz(argb);

    // constant white D65
    const XN: f64 = 95.047; const YN: f64 = 100.0; const ZN: f64 = 108.883;

    // nonlinear cube root
    fn f(t: f64) -> f64 {
        if t > 0.008856 { t.cbrt() }
        else { 7.787 * t + 16.0 / 116.0 } // linear for dark colours
    }
    let (fx, fy, fz) = (f(xyz[0]/XN), f(xyz[1]/YN), f(xyz[2]/ZN));
    [
        116.0 * fy - 16.0, // L* (lightness)
        500.0 * (fx - fy), // a* (green–red)
        200.0 * (fy - fz)  // b* (blue–yellow)
    ]
}

fn lab_to_argb(lab: [f64; 3]) -> u32 {
    let [l, a, b] = lab;

    // inverse the L* formula
    let fy = (l + 16.0) / 116.0;
    let fx = a / 500.0 + fy;
    let fz = fy - b / 200.0;

    // inverse of the function
    fn fi(t: f64) -> f64 { if t > 0.206897 { t.powi(3) } else { (t - 16.0/116.0) / 7.787 } }

    // reconstruct XYZ
    let xyz = [
        95.047  * fi(fx),
        100.0   * fi(fy),
        108.883 * fi(fz),
    ];
    xyz_to_argb(xyz).0
}


// --------------------------------------------------------------------- / kmeans quantiser

pub fn dcol(img: &DynamicImage, palette: &str) {

    // resize image for performance
    let small = img.resize_exact(64, 64, image::imageops::FilterType::Nearest);
    let rgb = small.to_rgb8();

    // auto detect light or dark image
    let total_l: f64 = rgb.pixels().map(|p| {
        let r = srgb_to_linear(p[0] as f64 / 255.0);
        let g = srgb_to_linear(p[1] as f64 / 255.0);
        let b = srgb_to_linear(p[2] as f64 / 255.0);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }).sum();
    let avg_l = total_l / (rgb.width() as f64 * rgb.height() as f64) * 100.0;
    let palette: String = match palette {
        "dark"  => "dark".into(),
        "light" => "light".into(),
        _       => if avg_l < 50.0 { "dark".into() } else { "light".into() },
    };

    // convert sRGB pixels to CIELab
    let pixels: Vec<[f64; 3]> = rgb.pixels()
        .map(|p| argb_to_lab(rgb_to_argb(p[0], p[1], p[2])))
        .collect();

    // k-means clustering
    let k = 8usize;
    let max_iter = 20;
    let mut centroids = Vec::with_capacity(k);
    let mut lcg = 42u32;
    for _ in 0..k {
        lcg = lcg.wrapping_mul(1664525).wrapping_add(1013904223);
        centroids.push(pixels[lcg as usize % pixels.len()]);
    }

    // loop to find centres of the colour clusters
    let mut assignments = vec![0usize; pixels.len()];
    for _ in 0..max_iter {
        for (i, px) in pixels.iter().enumerate() {
            let best = (0..k).min_by(|&a, &b| {
                let dist = |c: usize| -> f64 {
                    (0..3).map(|j| (px[j] - centroids[c][j]).powi(2)).sum()
                };
                dist(a).partial_cmp(&dist(b)).unwrap()
            }).unwrap_or(0);
            assignments[i] = best;
        }

        let mut sums = vec![[0.0f64; 3]; k];
        let mut counts = vec![0u32; k];
        for (i, &c) in assignments.iter().enumerate() {
            for j in 0..3 { sums[c][j] += pixels[i][j]; }
            counts[c] += 1;
        }
        for c in 0..k {
            if counts[c] > 0 {
                for j in 0..3 { centroids[c][j] = sums[c][j] / counts[c] as f64; }
            }
        }
    }

    // score cluster and pick the best dcol
    let mut cluster_counts = vec![0u32; k];
    for &a in &assignments { cluster_counts[a] += 1; }
    let total = pixels.len() as f64;
    let best = (0..k)
        .filter(|&c| cluster_counts[c] > 0)
        .max_by(|&a, &b| {
            let score = |c: usize| -> f64 {
                let lab = centroids[c];
                let chroma = (lab[1].powi(2) + lab[2].powi(2)).sqrt();
                let prop = cluster_counts[c] as f64 / total;
                chroma * prop.sqrt()
            };
            score(a).partial_cmp(&score(b)).unwrap()
        })
        .unwrap_or(0);
    generate_palette(lab_to_argb(centroids[best]), &palette);
}


// --------------------------------------------------------------------- / generate palette

fn generate_palette(source_argb: u32, palette: &str) {
    let (src_h, src_c, src_t) = argb_to_hct(source_argb, &VC);

    // material palette specs
    let pri_c = src_c.max(48.0);
    let sec_c = 16.0;
    let ter_h = (src_h + 60.0).rem_euclid(360.0);
    let ter_c = 24.0;
    let neu_c = 4.0;
    let nev_c = 8.0;
    let err_h = 25.0;
    let err_c = 84.0;

    // group, name, hue, chroma, dark_tone, light_tone
    type Role = (&'static str, &'static str, f64, f64, f64, f64);
    let roles: &[Role] = &[
        ("Primary",   "Primary",                src_h, pri_c, 80.0,  40.0),
        ("Primary",   "On Primary",             src_h, pri_c, 20.0, 100.0),
        ("Primary",   "Primary Container",      src_h, pri_c, 30.0,  90.0),
        ("Primary",   "On Primary Container",   src_h, pri_c, 90.0,  10.0),
        ("Secondary", "Secondary",              src_h, sec_c, 80.0,  40.0),
        ("Secondary", "On Secondary",           src_h, sec_c, 20.0, 100.0),
        ("Secondary", "Secondary Container",    src_h, sec_c, 30.0,  90.0),
        ("Secondary", "On Secondary Container", src_h, sec_c, 90.0,  10.0),
        ("Tertiary",  "Tertiary",               ter_h, ter_c, 80.0,  40.0),
        ("Tertiary",  "On Tertiary",            ter_h, ter_c, 20.0, 100.0),
        ("Tertiary",  "Tertiary Container",     ter_h, ter_c, 30.0,  90.0),
        ("Tertiary",  "On Tertiary Container",  ter_h, ter_c, 90.0,  10.0),
        ("Error",     "Error",                  err_h, err_c, 80.0,  40.0),
        ("Error",     "On Error",               err_h, err_c, 20.0, 100.0),
        ("Error",     "Error Container",        err_h, err_c, 30.0,  90.0),
        ("Error",     "On Error Container",     err_h, err_c, 90.0,  10.0),
        ("Surface",   "Background",             src_h, neu_c,  6.0,  98.0),
        ("Surface",   "On Background",          src_h, neu_c, 90.0,  10.0),
        ("Surface",   "Surface",                src_h, neu_c,  6.0,  98.0),
        ("Surface",   "On Surface",             src_h, neu_c, 90.0,  10.0),
        ("Surface",   "Surface Variant",        src_h, nev_c, 30.0,  90.0),
        ("Surface",   "On Surface Variant",     src_h, nev_c, 80.0,  30.0),
        ("Surface",   "Outline",                src_h, nev_c, 60.0,  50.0),
        ("Surface",   "Outline Variant",        src_h, nev_c, 30.0,  80.0),
        ("Surface",   "Shadow",                 src_h, neu_c,  0.0,   0.0),
        ("Surface",   "Inverse Surface",        src_h, neu_c, 90.0,  20.0),
        ("Surface",   "Inverse On Surface",     src_h, neu_c, 20.0,  95.0),
        ("Surface",   "Inverse Primary",        src_h, pri_c, 80.0,  40.0),
    ];

    // display dominant color
    let r = ((source_argb >> 16) & 0xFF) as u8;
    let g = ((source_argb >>  8) & 0xFF) as u8;
    let b = ( source_argb        & 0xFF) as u8;
    print!("\x1b[48;2;{};{};{}m \x1b[0m", r, g, b);
    println!(
        " #{:06X} :: HyDE-{:<5} :: H:{:.1}° C:{:.1} T:{:.1}",
        source_argb & 0xFFFFFF, palette, src_h, src_c, src_t
    );

    // display generated palette
    let mut colors = Vec::with_capacity(roles.len());
    for &(group, name, hue, chroma, dark_tone, light_tone) in roles {
        let tone = if palette == "dark" { dark_tone } else { light_tone };
        colors.push(ColorPalette { group, name, argb: hct_to_argb(hue, chroma, tone, &VC) });
    }
    print_palette(colors);
}


// --------------------------------------------------------------------- / print palette

fn print_palette(colors: Vec<ColorPalette>) {
    for entry in &colors {
        let r = ((entry.argb >> 16) & 0xFF) as u8;
        let g = ((entry.argb >>  8) & 0xFF) as u8;
        let b = ( entry.argb        & 0xFF) as u8;
        print!("\x1b[48;2;{};{};{}m \x1b[0m", r, g, b);
        println!(" #{:06X} :: {:<10} :: {}", entry.argb & 0xFFFFFF, entry.group, entry.name);
    }
    deploy_palette(&colors);
}


// --------------------------------------------------------------------- / deploy palette

fn deploy_palette(colors: &[ColorPalette]) {

    // eval XDG config dir
    let xdg_dir = std::env::var("XDG_CONFIG_HOME").ok().or_else(|| {
        std::env::var("HOME").ok().map(|home| format!("{}/.config", home))
    }).map(|base| format!("{}/wallbash", base)).unwrap_or_default();

    let templates = match std::fs::read_dir(&xdg_dir) {
        Ok(templates) => templates,
        Err(_) => return,
    };

    let mut deployments = Vec::new();
    let mut hashpaths = std::collections::HashSet::new();

    // scan template files (*.t2)
    for entry in templates.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("t2") {
            continue;
        }
        let template = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // parse header
        let (out, cmd, content) = if let Some(newline_pos) = template.find('\n') {
            let header = &template[..newline_pos];
            if header.starts_with("[::HyDE::]") {
                let parts: Vec<&str> = header.split('|').collect();
                let out = parts.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string());
                let cmd = parts.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string());
                (out, cmd, &template[newline_pos + 1..])
            } else {
                continue;
            }
        } else {
            continue;
        };

        // skip duplicate outputs
        if let Some(ref dest) = out {
            if !hashpaths.insert(dest.clone()) {
                continue;
            }
        }

        // inject palette colors
        let mut rendered = content.to_string();
        for color in colors {
            let tag = format!("[::{}::]", color.name);
            let hex = format!("#{:06X}", color.argb & 0xFFFFFF);
            rendered = rendered.replace(&tag, &hex);
        }
        deployments.push((out, cmd, rendered));
    }

    // spawn thread for each template
    let mut handles = Vec::new();
    for (out, cmd, rendered) in deployments {
        handles.push(std::thread::spawn(move || {

            // generate shell script
            let mut script = String::new();
            if let Some(target) = &out {
                script.push_str(&format!("if [ -f \"{}\" ]; then\n", target));
                script.push_str(&format!("cat > \"{}\" << 'EOF'\n", target));
                script.push_str(&rendered);
                script.push_str("\nEOF\n");
                script.push_str(&format!("echo \"[shell] deployed -> {}\"\n", target));
                if let Some(cmd) = &cmd {
                    script.push_str(&format!("{}\n", cmd));
                }
                script.push_str("else\n");
                script.push_str(&format!("echo '[shell] skipped (file not found) -> {}'\n", target));
                script.push_str("fi\n");
            }

            // execute shell script
            if !script.is_empty() {
                let _ = std::process::Command::new("sh").arg("-c").arg(&script).status();
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
}

