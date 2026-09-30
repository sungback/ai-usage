//! Cross-platform concentric ring badge rendering engine for macOS and Windows.
//!
//! Provides ring rendering, brand color palettes, font loading,
//! and optical number centering for system tray / menu bar items.

use fontdue::{Font, FontSettings};
use image::{Rgba, RgbaImage};

use crate::app_settings::SettingsFile;
use crate::models::{AppUsageData, UsageData};
use crate::providers::ProviderId;

/// 단일 링 배지의 여섯 값 묶음 (세션·주간 채움률, 바깥·안쪽 색, 중앙 숫자, 숫자 색).
type RingDatum = (f64, f64, Rgba<u8>, Rgba<u8>, Option<String>, Rgba<u8>);

/// 단일 링 쌍 렌더링 파라미터 묶음 (clippy too_many_arguments 회피).
pub struct RingPairParams<'a> {
    pub size: u32,
    pub session_fill: f64,
    pub weekly_fill: f64,
    pub outer_color: Rgba<u8>,
    pub inner_color: Rgba<u8>,
    pub center_text: Option<&'a str>,
    pub text_color: Rgba<u8>,
    pub font: Option<&'a Font>,
    pub show_inner_ring: bool,
}

/// 중앙 숫자 렌더링 파라미터 묶음 (clippy too_many_arguments 회피).
pub struct CenteredNumberParams<'a> {
    pub text: &'a str,
    pub cx: f32,
    pub cy: f32,
    pub color: Rgba<u8>,
    pub large: bool,
    pub scale_factor: f32,
}

/// Brand theme color palettes for concentric ring pairs (outer 5H, inner 7D) per provider.
pub fn provider_ring_palette(provider: ProviderId) -> (Rgba<u8>, Rgba<u8>) {
    match provider {
        // Claude Code: Anthropic warm coral orange / golden amber
        ProviderId::Claude => (
            Rgba([249, 115, 22, 255]),  // #F97316 bright warm coral orange
            Rgba([251, 191, 36, 255]),  // #FBBF24 golden amber
        ),
        // Codex: OpenAI signature emerald teal / bright lime mint
        ProviderId::Codex => (
            Rgba([16, 163, 127, 255]),  // #10A37F OpenAI teal
            Rgba([74, 222, 128, 255]),  // #4ADE80 bright lime mint
        ),
        // Google Antigravity: Gemini sky blue / electric violet
        ProviderId::Antigravity => (
            Rgba([0, 191, 255, 255]),   // #00BFFF deep sky blue
            Rgba([168, 85, 247, 255]),  // #A855F7 Gemini violet
        ),
        // Cursor: futuristic electric cyan / neon indigo
        ProviderId::Cursor => (
            Rgba([6, 182, 212, 255]),   // #06B6D4 electric cyan
            Rgba([129, 140, 248, 255]), // #818CF8 neon indigo
        ),
        // OpenCode: creative violet / soft rose pink
        ProviderId::OpenCode => (
            Rgba([139, 92, 246, 255]),  // #8B5CF6 violet
            Rgba([244, 114, 182, 255]), // #F472B6 rose pink
        ),
    }
}

/// Vibrant text color tailored for maximum contrast and readability inside the ring hole for each model.
pub fn provider_text_color(provider: ProviderId) -> Rgba<u8> {
    match provider {
        // Claude Code: Anthropic warm, vibrant coral orange (#FF8A3D)
        ProviderId::Claude => Rgba([255, 138, 61, 255]),
        // Codex: Signature OpenAI vibrant emerald mint (#34D399)
        ProviderId::Codex => Rgba([52, 211, 153, 255]),
        // Google Antigravity: Vibrant Gemini sky blue for high contrast and readability on dark
        // macOS menu bars and Windows taskbars (#38BDF8)
        ProviderId::Antigravity => Rgba([56, 189, 248, 255]),
        // Cursor: Futuristic bright electric cyan (#22D3EE)
        ProviderId::Cursor => Rgba([34, 211, 238, 255]),
        // OpenCode: Creative luminous soft violet (#C084FC)
        ProviderId::OpenCode => Rgba([192, 132, 252, 255]),
    }
}

/// Parse a "#RRGGBB" hex color string into an Rgba.
#[allow(dead_code)]
pub fn parse_hex_color(hex: &str) -> Option<Rgba<u8>> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgba([r, g, b, 255]))
}

/// Load a system font suitable for drawing clean numbers at small sizes.
/// Searches macOS, Windows, and Linux font paths.
pub fn load_system_font() -> Option<Font> {
    let mut candidate_paths: Vec<String> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        candidate_paths.push("/System/Library/Fonts/Supplemental/Arial Bold.ttf".into());
        candidate_paths.push("/System/Library/Fonts/SFNS.ttf".into());
        candidate_paths.push("/System/Library/Fonts/HelveticaNeue.ttc".into());
        candidate_paths.push("/System/Library/Fonts/Supplemental/Arial.ttf".into());
    }

    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        candidate_paths.push(format!("{}\\Fonts\\segoeuib.ttf", windir));
        candidate_paths.push(format!("{}\\Fonts\\arialbd.ttf", windir));
        candidate_paths.push(format!("{}\\Fonts\\segoeui.ttf", windir));
        candidate_paths.push(format!("{}\\Fonts\\arial.ttf", windir));
    }

    // Fallbacks for any OS or cross-testing
    candidate_paths.push("/System/Library/Fonts/Supplemental/Arial Bold.ttf".into());
    candidate_paths.push("C:\\Windows\\Fonts\\segoeuib.ttf".into());
    candidate_paths.push("C:\\Windows\\Fonts\\arialbd.ttf".into());
    candidate_paths.push("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf".into());
    candidate_paths.push("/usr/share/fonts/TTF/DejaVuSans-Bold.ttf".into());

    for path in candidate_paths {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(font);
            }
        }
    }
    None
}

/// Alpha blend a single pixel onto an RgbaImage.
pub fn blend_pixel(img: &mut RgbaImage, x: u32, y: u32, color: Rgba<u8>, glyph_alpha: u8) {
    if glyph_alpha == 0 || x >= img.width() || y >= img.height() {
        return;
    }
    let pixel = img.get_pixel_mut(x, y);
    let alpha_f = (glyph_alpha as f32 / 255.0) * (color[3] as f32 / 255.0);
    if alpha_f <= 0.0 {
        return;
    }
    let inv_alpha = 1.0 - alpha_f;
    let dst_a = pixel[3] as f32 / 255.0;
    let out_a = alpha_f + dst_a * inv_alpha;
    if out_a <= 0.0 {
        return;
    }
    let r = ((color[0] as f32 * alpha_f + pixel[0] as f32 * dst_a * inv_alpha) / out_a).round();
    let g = ((color[1] as f32 * alpha_f + pixel[1] as f32 * dst_a * inv_alpha) / out_a).round();
    let b = ((color[2] as f32 * alpha_f + pixel[2] as f32 * dst_a * inv_alpha) / out_a).round();

    pixel[0] = r.clamp(0.0, 255.0) as u8;
    pixel[1] = g.clamp(0.0, 255.0) as u8;
    pixel[2] = b.clamp(0.0, 255.0) as u8;
    pixel[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Draw a number string centered precisely at (cx, cy) inside the ring hole.
pub fn draw_centered_number(
    img: &mut RgbaImage,
    font: &Font,
    params: &CenteredNumberParams,
) {
    let text = params.text;
    if text.is_empty() {
        return;
    }
    let base_size = if params.large {
        match text.len() {
            1 => 21.0,
            2 => 18.5,
            _ => 15.0,
        }
    } else {
        match text.len() {
            1 => 14.5,
            2 => 12.5,
            _ => 10.0,
        }
    };
    let font_size = base_size * params.scale_factor;

    struct GlyphPixel {
        px_rel: f32,
        py_rel: f32,
        alpha: u8,
    }

    let mut pixels = Vec::new();
    let mut cursor_x = 0.0f32;

    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, font_size);
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let alpha = bitmap[row * metrics.width + col];
                if alpha > 0 {
                    let px_rel = cursor_x + metrics.xmin as f32 + col as f32;
                    let py_rel = -(metrics.height as f32) - (metrics.ymin as f32) + row as f32;
                    pixels.push(GlyphPixel {
                        px_rel,
                        py_rel,
                        alpha,
                    });
                }
            }
        }
        cursor_x += metrics.advance_width;
    }

    if pixels.is_empty() {
        return;
    }

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for p in &pixels {
        if p.px_rel < min_x { min_x = p.px_rel; }
        if p.px_rel > max_x { max_x = p.px_rel; }
        if p.py_rel < min_y { min_y = p.py_rel; }
        if p.py_rel > max_y { max_y = p.py_rel; }
    }

    let mid_x = (min_x + max_x) / 2.0;
    let mid_y = (min_y + max_y) / 2.0;

    let offset_x = params.cx - mid_x;
    let offset_y = params.cy - mid_y;

    for p in pixels {
        let px = (offset_x + p.px_rel).round() as i32;
        let py = (offset_y + p.py_rel).round() as i32;
        if px >= 0 && px < img.width() as i32 && py >= 0 && py < img.height() as i32 {
            blend_pixel(img, px as u32, py as u32, params.color, p.alpha);
        }
    }
}

/// Anti-aliasing factor for a distance within [r_inner, r_outer].
/// Returns 0.0 outside, 1.0 inside, smooth at edges.
pub fn ring_antialias(dist: f64, r_inner: f64, r_outer: f64) -> f64 {
    if dist < r_inner - 0.5 || dist > r_outer + 0.5 {
        return 0.0;
    }
    let inner_edge = (dist - (r_inner - 0.5)).clamp(0.0, 1.0);
    let outer_edge = ((r_outer + 0.5) - dist).clamp(0.0, 1.0);
    inner_edge.min(outer_edge)
}

/// Scale the alpha channel of an RGBA color by a factor.
fn scale_alpha(color: Rgba<u8>, factor: f64) -> Rgba<u8> {
    let a = (color[3] as f64 * factor).round().clamp(0.0, 255.0) as u8;
    Rgba([color[0], color[1], color[2], a])
}

/// Render a single ring pair (outer + inner concentric arcs) into a square image of `size` x `size`.
///
/// `session_fill` and `weekly_fill` are 0.0..=1.0 fractions.
pub fn render_single_ring_pair(params: &RingPairParams) -> RgbaImage {
    let size = params.size;
    let session_fill = params.session_fill;
    let weekly_fill = params.weekly_fill;
    let outer_color = params.outer_color;
    let inner_color = params.inner_color;
    let center_text = params.center_text;
    let text_color = params.text_color;
    let font = params.font;
    let show_inner_ring = params.show_inner_ring;
    let mut img = RgbaImage::new(size, size);
    let center = size as f64 / 2.0;
    let scale = size as f64 / 44.0;

    // Ring geometry (proportional to 44px base size)
    let stroke_width = (4.0 * scale).max(2.0);
    let ring_gap = (2.0 * scale).max(1.0);

    let outer_r_outer = center - (1.0 * scale).max(0.5);
    let outer_r_inner = outer_r_outer - stroke_width;
    let inner_r_outer = outer_r_inner - ring_gap;
    let inner_r_inner = inner_r_outer - stroke_width;

    // Track color: dim version of ring color at low alpha
    let track_alpha = 60u8;
    let outer_track = Rgba([outer_color[0], outer_color[1], outer_color[2], track_alpha]);
    let inner_track = Rgba([inner_color[0], inner_color[1], inner_color[2], track_alpha]);

    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - center;
            let dy = y as f64 + 0.5 - center;
            let dist = (dx * dx + dy * dy).sqrt();

            // Angle from 12 o'clock, clockwise, 0..1
            let angle = dx.atan2(-dy);
            let norm_angle = if angle < 0.0 {
                (angle + std::f64::consts::TAU) / std::f64::consts::TAU
            } else {
                angle / std::f64::consts::TAU
            };

            // Outer ring
            if dist >= outer_r_inner - 0.5 && dist <= outer_r_outer + 0.5 {
                let aa = ring_antialias(dist, outer_r_inner, outer_r_outer);
                if aa > 0.0 {
                    let t = scale_alpha(outer_track, aa);
                    blend_pixel(&mut img, x, y, t, 255);
                    if norm_angle <= session_fill {
                        let f = scale_alpha(outer_color, aa);
                        blend_pixel(&mut img, x, y, f, 255);
                    }
                }
            }

            // Inner ring (only drawn if show_inner_ring is true)
            if show_inner_ring && dist >= inner_r_inner - 0.5 && dist <= inner_r_outer + 0.5 {
                let aa = ring_antialias(dist, inner_r_inner, inner_r_outer);
                if aa > 0.0 {
                    let t = scale_alpha(inner_track, aa);
                    blend_pixel(&mut img, x, y, t, 255);
                    if norm_angle <= weekly_fill {
                        let f = scale_alpha(inner_color, aa);
                        blend_pixel(&mut img, x, y, f, 255);
                    }
                }
            }
        }
    }

    // Draw centered 5-hour number in the hollow center
    if let (Some(text), Some(f)) = (center_text, font) {
        draw_centered_number(
            &mut img,
            f,
            &CenteredNumberParams {
                text,
                cx: center as f32,
                cy: center as f32,
                color: text_color,
                large: !show_inner_ring,
                scale_factor: scale as f32,
            },
        );
    }

    img
}

/// Render a single provider's ring as a standalone square icon (ideal for Windows notification tray slots).
#[allow(dead_code)]
pub fn render_single_provider_ring(
    provider_id: ProviderId,
    usage: Option<&UsageData>,
    settings: &SettingsFile,
    size: u32,
) -> RgbaImage {
    let countdown = settings.usage_countdown;
    let show_inner_ring = settings.show_inner_ring;
    let default_usage = UsageData::default();
    let usage = usage.unwrap_or(&default_usage);
    let font = load_system_font();

    let s_fill = UsageData::fill(usage.session.percentage, countdown);
    let w_fill = UsageData::fill(usage.weekly.percentage, countdown);

    let center_num = if usage.session.available {
        let s_val = UsageData::shown(usage.session.percentage, countdown);
        Some(format!("{:.0}", s_val))
    } else {
        None
    };

    let (outer_c, inner_c) = provider_ring_palette(provider_id);
    let text_color = settings
        .ring_outer_color
        .as_deref()
        .and_then(parse_hex_color)
        .unwrap_or_else(|| provider_text_color(provider_id));

    render_single_ring_pair(&RingPairParams {
        size,
        session_fill: s_fill,
        weekly_fill: w_fill,
        outer_color: outer_c,
        inner_color: inner_c,
        center_text: center_num.as_deref(),
        text_color,
        font: font.as_ref(),
        show_inner_ring,
    })
}

/// Render the full ring badge image with customizable ring size and gap.
pub fn render_ring_badge_image_at_size(
    data: &AppUsageData,
    settings: &SettingsFile,
    ring_size: u32,
    gap: u32,
) -> Option<RgbaImage> {

    let countdown = settings.usage_countdown;
    let show_inner_ring = settings.show_inner_ring;
    let default_usage = UsageData::default();
    let font = load_system_font();

    let ordered_providers = settings.ordered_providers();

    let mut ring_data: Vec<RingDatum> = Vec::new();

    for provider_id in ordered_providers {
        if !settings.provider_enabled(provider_id) {
            continue;
        }
        let usage = data.get(provider_id).unwrap_or(&default_usage);

        let s_fill = UsageData::fill(usage.session.percentage, countdown);
        let w_fill = UsageData::fill(usage.weekly.percentage, countdown);

        let center_num = if usage.session.available {
            let s_val = UsageData::shown(usage.session.percentage, countdown);
            Some(format!("{:.0}", s_val))
        } else {
            None
        };

        let (default_outer, default_inner) = provider_ring_palette(provider_id);

        let outer_c = if ring_data.is_empty() {
            settings.ring_outer_color.as_deref()
                .and_then(parse_hex_color)
                .unwrap_or(default_outer)
        } else {
            default_outer
        };
        let inner_c = if ring_data.is_empty() {
            settings.ring_inner_color.as_deref()
                .and_then(parse_hex_color)
                .unwrap_or(default_inner)
        } else {
            default_inner
        };

        let text_color = if ring_data.is_empty() {
            settings
                .ring_outer_color
                .as_deref()
                .and_then(parse_hex_color)
                .unwrap_or_else(|| provider_text_color(provider_id))
        } else {
            provider_text_color(provider_id)
        };

        ring_data.push((s_fill, w_fill, outer_c, inner_c, center_num, text_color));
    }

    if ring_data.is_empty() {
        return None;
    }

    let count = ring_data.len() as u32;
    let total_width = ring_size * count + gap * count.saturating_sub(1);

    let mut combined = RgbaImage::new(total_width, ring_size);

    for (i, (s_fill, w_fill, outer_c, inner_c, center_num, text_color)) in
        ring_data.iter().enumerate()
    {
        let ring_img = render_single_ring_pair(&RingPairParams {
            size: ring_size,
            session_fill: *s_fill,
            weekly_fill: *w_fill,
            outer_color: *outer_c,
            inner_color: *inner_c,
            center_text: center_num.as_deref(),
            text_color: *text_color,
            font: font.as_ref(),
            show_inner_ring,
        });
        let x_offset = (i as u32) * (ring_size + gap);
        for ry in 0..ring_size {
            for rx in 0..ring_size {
                let src = ring_img.get_pixel(rx, ry);
                if src[3] > 0 {
                    let dx = x_offset + rx;
                    combined.put_pixel(dx, ry, *src);
                }
            }
        }
    }

    Some(combined)
}

/// Convert an RgbaImage to top-down, premultiplied BGRA 0xAARRGGBB pixels for Windows DIB / HICON.
#[allow(dead_code)]
pub fn rgba_to_bgra_premultiplied(img: &RgbaImage) -> Vec<u32> {
    let mut pixels = Vec::with_capacity((img.width() * img.height()) as usize);
    for pixel in img.pixels() {
        let r = pixel[0] as u32;
        let g = pixel[1] as u32;
        let b = pixel[2] as u32;
        let a = pixel[3] as u32;
        if a == 0 {
            pixels.push(0);
        } else {
            let pr = (r * a + 127) / 255;
            let pg = (g * a + 127) / 255;
            let pb = (b * a + 127) / 255;
            let val = (a << 24) | (pr << 16) | (pg << 8) | pb;
            pixels.push(val);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_ring_palettes() {
        let (claude_out, claude_in) = provider_ring_palette(ProviderId::Claude);
        assert_eq!(claude_out, Rgba([249, 115, 22, 255]));
        assert_eq!(claude_in, Rgba([251, 191, 36, 255]));

        let (codex_out, codex_in) = provider_ring_palette(ProviderId::Codex);
        assert_eq!(codex_out, Rgba([16, 163, 127, 255]));
        assert_eq!(codex_in, Rgba([74, 222, 128, 255]));

        let (anti_out, anti_in) = provider_ring_palette(ProviderId::Antigravity);
        assert_eq!(anti_out, Rgba([0, 191, 255, 255]));
        assert_eq!(anti_in, Rgba([168, 85, 247, 255]));
    }

    #[test]
    fn test_render_single_ring_pair_modes() {
        let font = load_system_font();

        // 1. Dual-ring mode
        let img_dual = render_single_ring_pair(&RingPairParams {
            size: 44,
            session_fill: 0.85,
            weekly_fill: 0.50,
            outer_color: Rgba([249, 115, 22, 255]),
            inner_color: Rgba([251, 191, 36, 255]),
            center_text: Some("85"),
            text_color: Rgba([245, 245, 245, 255]),
            font: font.as_ref(),
            show_inner_ring: true,
        });
        assert_eq!(img_dual.width(), 44);
        assert_eq!(img_dual.height(), 44);

        // 2. Single-ring mode (inner ring disabled)
        let img_single = render_single_ring_pair(&RingPairParams {
            size: 44,
            session_fill: 0.85,
            weekly_fill: 0.50,
            outer_color: Rgba([249, 115, 22, 255]),
            inner_color: Rgba([251, 191, 36, 255]),
            center_text: Some("85"),
            text_color: Rgba([245, 245, 245, 255]),
            font: font.as_ref(),
            show_inner_ring: false,
        });
        assert_eq!(img_single.width(), 44);
        assert_eq!(img_single.height(), 44);

        // 3. 32px Windows tray icon size (dual ring)
        let img_32_dual = render_single_ring_pair(&RingPairParams {
            size: 32,
            session_fill: 0.85,
            weekly_fill: 0.45,
            outer_color: Rgba([249, 115, 22, 255]),
            inner_color: Rgba([251, 191, 36, 255]),
            center_text: Some("85"),
            text_color: Rgba([245, 245, 245, 255]),
            font: font.as_ref(),
            show_inner_ring: true,
        });
        assert_eq!(img_32_dual.width(), 32);
        assert_eq!(img_32_dual.height(), 32);

        // 4. 32px Windows tray icon size (single ring enlarged number)
        let img_32_single = render_single_ring_pair(&RingPairParams {
            size: 32,
            session_fill: 0.85,
            weekly_fill: 0.45,
            outer_color: Rgba([249, 115, 22, 255]),
            inner_color: Rgba([251, 191, 36, 255]),
            center_text: Some("85"),
            text_color: Rgba([245, 245, 245, 255]),
            font: font.as_ref(),
            show_inner_ring: false,
        });
        assert_eq!(img_32_single.width(), 32);
        assert_eq!(img_32_single.height(), 32);

        // Convert to premultiplied BGRA
        let bgra = rgba_to_bgra_premultiplied(&img_32_dual);
        assert_eq!(bgra.len(), 32 * 32);
        assert!(bgra.iter().any(|&p| (p >> 24) > 0));

        let _ = std::fs::create_dir_all("target");
        let _ = img_32_dual.save("target/test_win_tray_dual_32.png");
        let _ = img_32_single.save("target/test_win_tray_single_32.png");

        // Save 4x preview side by side for visual artifact check
        let mut preview = RgbaImage::new(32 * 2 + 16, 32);
        for y in 0..32 {
            for x in 0..32 {
                preview.put_pixel(x, y, *img_32_dual.get_pixel(x, y));
                preview.put_pixel(32 + 16 + x, y, *img_32_single.get_pixel(x, y));
            }
        }
        let preview_4x = image::imageops::resize(
            &preview,
            preview.width() * 4,
            preview.height() * 4,
            image::imageops::FilterType::Nearest,
        );
        let _ = preview_4x.save("/Users/back/.gemini/antigravity-ide/brain/fc31fd98-d1ff-4842-ac1a-c7e9eb50122c/test_win_tray_preview_4x.png");
    }

    #[test]
    fn test_provider_text_colors() {
        assert_eq!(provider_text_color(ProviderId::Claude), Rgba([255, 138, 61, 255]));
        assert_eq!(provider_text_color(ProviderId::Codex), Rgba([52, 211, 153, 255]));
        assert_eq!(provider_text_color(ProviderId::Antigravity), Rgba([56, 189, 248, 255]));
        assert_eq!(provider_text_color(ProviderId::Cursor), Rgba([34, 211, 238, 255]));
        assert_eq!(provider_text_color(ProviderId::OpenCode), Rgba([192, 132, 252, 255]));
    }

    #[test]
    fn test_ring_badge_track_alpha_preservation() {
        // Render a 44px ring with 0% fill so only the track (alpha 60) is rendered.
        let img = render_single_ring_pair(&RingPairParams {
            size: 44,
            session_fill: 0.0,
            weekly_fill: 0.0,
            outer_color: Rgba([249, 115, 22, 255]),
            inner_color: Rgba([251, 191, 36, 255]),
            center_text: None,
            text_color: Rgba([255, 255, 255, 255]),
            font: None,
            show_inner_ring: true,
        });

        // Find pixels in the track region (outer ring stroke).
        // center is 22.0, outer ring is near radius ~19..20.
        // Let's sample a point along the horizontal axis: (x=2, y=22) or (x=3, y=22)
        // distance from (22, 22): for (3.5, 22.5) -> dist = 18.5
        let center = 22.0f64;
        let mut max_track_alpha = 0u8;
        for y in 0..44 {
            for x in 0..44 {
                let dx = x as f64 + 0.5 - center;
                let dy = y as f64 + 0.5 - center;
                let dist = (dx * dx + dy * dy).sqrt();
                // Outer ring track radius
                if (17.5..=20.0).contains(&dist) {
                    let pixel = img.get_pixel(x, y);
                    if pixel[3] > max_track_alpha {
                        max_track_alpha = pixel[3];
                    }
                }
            }
        }

        // Track alpha is defined as 60. With anti-aliasing = 1.0, maximum pixel alpha should be 60.
        // In the buggy implementation, it was (60/255)^2 * 255 = 14!
        assert_eq!(
            max_track_alpha, 60,
            "Track alpha should be 60 at full coverage, but got {}",
            max_track_alpha
        );

        // Also test render_ring_badge_image_at_size with a settings file
        let mut settings = SettingsFile::default();
        settings.set_provider_enabled(ProviderId::Claude, true);
        let data = AppUsageData::default();
        let combined = render_ring_badge_image_at_size(&data, &settings, 44, 4)
            .expect("combined image should be generated");

        let mut max_combined_track_alpha = 0u8;
        for y in 0..44 {
            for x in 0..44 {
                let dx = x as f64 + 0.5 - center;
                let dy = y as f64 + 0.5 - center;
                let dist = (dx * dx + dy * dy).sqrt();
                if (17.5..=20.0).contains(&dist) {
                    let pixel = combined.get_pixel(x, y);
                    if pixel[3] > max_combined_track_alpha {
                        max_combined_track_alpha = pixel[3];
                    }
                }
            }
        }

        assert_eq!(
            max_combined_track_alpha, 60,
            "Combined image track alpha should remain 60, but got {}",
            max_combined_track_alpha
        );
    }
}
