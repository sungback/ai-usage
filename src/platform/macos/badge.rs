//! macOS 메뉴바 뱃지 이미지 렌더링 (텍스트 기반 레거시 버전).
//! 현재 실제 사용은 ring_badge 모듈로 대체되었으나, 참고용으로 유지.

use fontdue::{Font, FontSettings};
use image::{imageops, Rgba, RgbaImage};

#[allow(dead_code)]
pub(super) static CLAUDE_ICON_BYTES: &[u8] = include_bytes!("../../icons/claude_menu.png");
#[allow(dead_code)]
pub(super) static CODEX_ICON_BYTES: &[u8] = include_bytes!("../../icons/codex_menu.png");
#[allow(dead_code)]
pub(super) static ANTIGRAVITY_ICON_BYTES: &[u8] =
    include_bytes!("../../icons/antigravity_menu.png");

#[allow(dead_code)]
pub(super) fn load_sf_font() -> Option<Font> {
    for path in [
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
        "/System/Library/Fonts/SFNS.ttf",
        "/System/Library/Fonts/HelveticaNeue.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(font) = Font::from_bytes(bytes, FontSettings::default()) {
                return Some(font);
            }
        }
    }
    None
}

#[allow(dead_code)]
pub(super) fn blend_pixel(img: &mut RgbaImage, x: u32, y: u32, color: Rgba<u8>, glyph_alpha: u8) {
    if glyph_alpha == 0 || x >= img.width() || y >= img.height() {
        return;
    }
    let pixel = img.get_pixel_mut(x, y);
    let alpha_f = (glyph_alpha as f32 / 255.0) * (color[3] as f32 / 255.0);
    if alpha_f <= 0.0 {
        return;
    }
    let src_r = color[0] as f32;
    let src_g = color[1] as f32;
    let src_b = color[2] as f32;

    let dst_a = pixel[3] as f32 / 255.0;
    let out_a = alpha_f + dst_a * (1.0 - alpha_f);
    if out_a > 0.0 {
        let dst_r = pixel[0] as f32;
        let dst_g = pixel[1] as f32;
        let dst_b = pixel[2] as f32;
        let out_r = (src_r * alpha_f + dst_r * dst_a * (1.0 - alpha_f)) / out_a;
        let out_g = (src_g * alpha_f + dst_g * dst_a * (1.0 - alpha_f)) / out_a;
        let out_b = (src_b * alpha_f + dst_b * dst_a * (1.0 - alpha_f)) / out_a;
        *pixel = Rgba([
            out_r as u8,
            out_g as u8,
            out_b as u8,
            (out_a * 255.0).round() as u8,
        ]);
    }
}

#[allow(dead_code)]
pub(super) fn draw_text(
    img: &mut RgbaImage,
    font: &Font,
    text: &str,
    mut x: i32,
    baseline_y: i32,
    size: f32,
    color: Rgba<u8>,
) -> i32 {
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, size);
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let alpha = bitmap[row * metrics.width + col];
                if alpha > 0 {
                    let px = x + metrics.xmin + col as i32;
                    let py = baseline_y - metrics.height as i32 - metrics.ymin + row as i32;
                    if px >= 0
                        && px < img.width() as i32
                        && py >= 0
                        && py < img.height() as i32
                    {
                        blend_pixel(img, px as u32, py as u32, color, alpha);
                    }
                }
            }
        }
        x += metrics.advance_width.round() as i32;
    }
    x
}

#[allow(dead_code)]
pub(super) fn measure_text(font: &Font, text: &str, size: f32) -> i32 {
    let mut width = 0.0;
    for ch in text.chars() {
        let metrics = font.metrics(ch, size);
        width += metrics.advance_width;
    }
    width.round() as i32
}

#[allow(dead_code)]
pub(super) fn draw_icon(
    img: &mut RgbaImage,
    icon: &RgbaImage,
    target_x: i32,
    target_y: i32,
    target_w: u32,
    target_h: u32,
) {
    let resized = imageops::resize(icon, target_w, target_h, imageops::FilterType::Lanczos3);
    for row in 0..target_h {
        for col in 0..target_w {
            let px = target_x + col as i32;
            let py = target_y + row as i32;
            if px >= 0
                && px < img.width() as i32
                && py >= 0
                && py < img.height() as i32
            {
                let src = resized.get_pixel(col, row);
                blend_pixel(img, px as u32, py as u32, *src, src[3]);
            }
        }
    }
}

#[allow(dead_code)]
pub(super) fn percent_color(pct: f64) -> Rgba<u8> {
    if pct >= 50.0 {
        Rgba([74, 222, 128, 255])   // lime green #4ADE80
    } else if pct >= 20.0 {
        Rgba([251, 191, 36, 255])   // amber #FBBF24
    } else {
        Rgba([248, 113, 113, 255])  // coral red #F87171
    }
}

#[allow(dead_code)]
struct BadgeColumn {
    icon: Option<RgbaImage>,
    row1_label: String,
    row1_pct: String,
    row1_color: Rgba<u8>,
    row2_label: String,
    row2_pct: String,
    row2_color: Rgba<u8>,
}

#[allow(dead_code)]
pub fn render_compact_badge_image(
    data: &crate::models::AppUsageData,
    settings: &crate::app_settings::SettingsFile,
) -> Option<RgbaImage> {
    let font = load_sf_font()?;

    let claude_icon = image::load_from_memory(CLAUDE_ICON_BYTES).ok()?.into_rgba8();
    let codex_icon = image::load_from_memory(CODEX_ICON_BYTES).ok()?.into_rgba8();
    let antigravity_icon = image::load_from_memory(ANTIGRAVITY_ICON_BYTES).ok()?.into_rgba8();

    let countdown = settings.usage_countdown;
    let mut columns = Vec::new();
    let ordered_providers = settings.ordered_providers();
    let default_usage = crate::models::UsageData::default();
    let muted_gray = Rgba([160, 160, 160, 255]);

    for provider_id in ordered_providers {
        if !settings.provider_enabled(provider_id) {
            continue;
        }
        let usage = data.get(provider_id).unwrap_or(&default_usage);

        let s_pct = if countdown {
            (100.0 - usage.session.percentage).clamp(0.0, 100.0)
        } else {
            usage.session.percentage
        };
        let w_pct = if countdown {
            (100.0 - usage.weekly.percentage).clamp(0.0, 100.0)
        } else {
            usage.weekly.percentage
        };

        let (icon, row1_label, row2_label) = match provider_id {
            crate::providers::ProviderId::Codex => (
                Some(codex_icon.clone()),
                "GPT(5H) ".to_string(),
                "GPT(1W) ".to_string(),
            ),
            crate::providers::ProviderId::Claude => (
                Some(claude_icon.clone()),
                "CC(5H) ".to_string(),
                "CC(1W) ".to_string(),
            ),
            crate::providers::ProviderId::Antigravity => (
                Some(antigravity_icon.clone()),
                "AG(Gem) ".to_string(),
                "AG(3P)  ".to_string(),
            ),
            crate::providers::ProviderId::OpenCode => (
                None,
                "OC(5H) ".to_string(),
                "OC(1W) ".to_string(),
            ),
            crate::providers::ProviderId::Cursor => (
                None,
                "Cur(5H) ".to_string(),
                "Cur(1W) ".to_string(),
            ),
        };

        let (row1_pct, row1_color) = if usage.session.available {
            (format!("{:.0}%", s_pct), percent_color(s_pct))
        } else {
            ("--%".to_string(), muted_gray)
        };

        let (row2_pct, row2_color) = if usage.weekly.available {
            (format!("{:.0}%", w_pct), percent_color(w_pct))
        } else {
            ("--%".to_string(), muted_gray)
        };

        columns.push(BadgeColumn {
            icon,
            row1_label,
            row1_pct,
            row1_color,
            row2_label,
            row2_pct,
            row2_color,
        });
    }

    if columns.is_empty() {
        return None;
    }

    let font_size = 24.0;
    let icon_size = 32u32;
    let icon_gap = 6i32;
    let col_gap = 20i32;
    let side_padding = 8i32;
    let height = 48u32;
    let row1_base_y = 20i32;
    let row2_base_y = 44i32;
    let row1_icon_y = 3;
    let row2_icon_y = 27;

    let mut col_widths = Vec::new();
    for col in &columns {
        let icon_w = if col.icon.is_some() { icon_size as i32 + icon_gap } else { 0 };
        let w1 = measure_text(&font, &col.row1_label, font_size)
            + measure_text(&font, &col.row1_pct, font_size);
        let w2 = measure_text(&font, &col.row2_label, font_size)
            + measure_text(&font, &col.row2_pct, font_size);
        col_widths.push(icon_w + w1.max(w2));
    }

    let total_width = (side_padding * 2
        + col_widths.iter().sum::<i32>()
        + col_gap * (columns.len() as i32 - 1))
    .max(1) as u32;

    let white = Rgba([255, 255, 255, 255]);
    let mut img = RgbaImage::new(total_width, height);
    let mut cur_x = side_padding;

    for (i, col) in columns.iter().enumerate() {
        let icon_offset = if col.icon.is_some() { icon_size as i32 + icon_gap } else { 0 };

        if let Some(ref icon) = col.icon {
            draw_icon(&mut img, icon, cur_x, row1_icon_y, icon_size, icon_size);
            draw_icon(&mut img, icon, cur_x, row2_icon_y, icon_size, icon_size);
        }

        let text_x = cur_x + icon_offset;
        let after1 = draw_text(&mut img, &font, &col.row1_label, text_x, row1_base_y, font_size, white);
        draw_text(&mut img, &font, &col.row1_pct, after1, row1_base_y, font_size, col.row1_color);
        let after2 = draw_text(&mut img, &font, &col.row2_label, text_x, row2_base_y, font_size, white);
        draw_text(&mut img, &font, &col.row2_pct, after2, row2_base_y, font_size, col.row2_color);

        cur_x += col_widths[i] + col_gap;
    }

    Some(img)
}
