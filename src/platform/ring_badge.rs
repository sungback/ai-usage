//! macOS 및 Windows용 크로스 플랫폼 동심원 링 배지 렌더링 엔진입니다.
//!
//! 시스템 트레이 및 메뉴 바 아이템을 위한 링 렌더링, 브랜드 색상 팔레트,
//! 폰트 로딩, 시각적 숫자 중앙 정렬을 제공합니다.

use fontdue::{Font, FontSettings};
use image::{Rgba, RgbaImage};

use crate::app_settings::SettingsFile;
use crate::models::{AppUsageData, UsageData};
use crate::providers::ProviderId;

/// 단일 링 배지의 여섯 값 묶음 (세션·주간 채움률, 바깥·안쪽 색, 중앙 숫자, 숫자 색).
type RingDatum = (f64, f64, Rgba<u8>, Rgba<u8>, Option<String>, Rgba<u8>);

/// 단일 링 쌍 렌더링에 필요한 파라미터 묶음.
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

/// 중앙 숫자 렌더링에 필요한 파라미터 묶음.
pub struct CenteredNumberParams<'a> {
    pub text: &'a str,
    pub cx: f32,
    pub cy: f32,
    pub color: Rgba<u8>,
    pub large: bool,
    pub scale_factor: f32,
}

/// 제공자별 동심원 링 쌍(바깥쪽 5H, 안쪽 7D)의 브랜드 테마 색상 팔레트입니다.
pub fn provider_ring_palette(provider: ProviderId) -> (Rgba<u8>, Rgba<u8>) {
    match provider {
        // Claude Code: Anthropic의 따뜻한 코랄 오렌지 / 골든 앰버
        ProviderId::Claude => (
            Rgba([249, 115, 22, 255]),  // #F97316 bright warm coral orange
            Rgba([251, 191, 36, 255]),  // #FBBF24 golden amber
        ),
        // Codex: OpenAI 시그니처 에메랄드 틸 / 밝은 라임 민트
        ProviderId::Codex => (
            Rgba([16, 163, 127, 255]),  // #10A37F OpenAI teal
            Rgba([74, 222, 128, 255]),  // #4ADE80 bright lime mint
        ),
        // Google Antigravity: Gemini 스카이 블루 / 일렉트릭 바이올렛
        ProviderId::Antigravity => (
            Rgba([0, 191, 255, 255]),   // #00BFFF deep sky blue
            Rgba([168, 85, 247, 255]),  // #A855F7 Gemini violet
        ),
        // Cursor: 미래지향적 일렉트릭 시안 / 네온 인디고
        ProviderId::Cursor => (
            Rgba([6, 182, 212, 255]),   // #06B6D4 electric cyan
            Rgba([129, 140, 248, 255]), // #818CF8 neon indigo
        ),
        // OpenCode: 창의적인 바이올렛 / 소프트 로즈 핑크
        ProviderId::OpenCode => (
            Rgba([139, 92, 246, 255]),  // #8B5CF6 violet
            Rgba([244, 114, 182, 255]), // #F472B6 rose pink
        ),
    }
}

/// 각 모델의 링 내부 구멍에서 최대 대비와 가독성을 얻도록 맞춤 설계된 생생한 텍스트 색상입니다.
pub fn provider_text_color(provider: ProviderId) -> Rgba<u8> {
    match provider {
        // Claude Code: Anthropic의 따뜻하고 선명한 코랄 오렌지 (#FF8A3D)
        ProviderId::Claude => Rgba([255, 138, 61, 255]),
        // Codex: OpenAI 시그니처 선명한 에메랄드 민트 (#34D399)
        ProviderId::Codex => Rgba([52, 211, 153, 255]),
        // Google Antigravity: 어두운 macOS 메뉴 바 및 Windows 작업 표시줄에서 높은 대비와 가독성을 제공하는 생생한 Gemini 스카이 블루 (#38BDF8)
        ProviderId::Antigravity => Rgba([56, 189, 248, 255]),
        // Cursor: 미래지향적 밝은 일렉트릭 시안 (#22D3EE)
        ProviderId::Cursor => Rgba([34, 211, 238, 255]),
        // OpenCode: 창의적인 발광 소프트 바이올렛 (#C084FC)
        ProviderId::OpenCode => Rgba([192, 132, 252, 255]),
    }
}

/// 실패해서 직전 값을 보여주는 중(stale)인 공급자의 회색 링 색상.
/// 소진(빨강)과 달리 "모르는 상태"임을 한눈에 구분하기 위한 것이다.
pub fn stale_ring_colors() -> (Rgba<u8>, Rgba<u8>, Rgba<u8>) {
    (
        Rgba([130, 130, 130, 255]),
        Rgba([165, 165, 165, 255]),
        Rgba([175, 175, 175, 255]),
    )
}

/// 링 바깥·안쪽·숫자 색상. stale이면 브랜드색 대신 회색을 돌려준다.
pub fn ring_colors_for(provider_id: ProviderId, usage: &UsageData) -> (Rgba<u8>, Rgba<u8>, Rgba<u8>) {
    if usage.stale {
        stale_ring_colors()
    } else {
        let (outer, inner) = provider_ring_palette(provider_id);
        (outer, inner, provider_text_color(provider_id))
    }
}

/// "#RRGGBB" 16진수 색상 문자열을 Rgba로 파싱합니다.
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

/// 작은 크기에서도 숫자를 깔끔하게 렌더링하기에 적합한 시스템 폰트를 로드합니다.
/// macOS, Windows, Linux 폰트 경로를 순차적으로 탐색합니다.
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

    // 모든 OS 또는 크로스 테스트를 위한 대체(fallback) 폰트
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

/// 단일 픽셀을 RgbaImage 위에 알파 블렌딩합니다.
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

/// 링 구멍 내부의 (cx, cy) 위치에 정확하게 중앙 정렬하여 숫자 문자열을 그립니다.
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

/// [r_inner, r_outer] 범위 내의 거리에 대한 안티앨리어싱 계수입니다.
/// 외부에서는 0.0, 내부에서는 1.0을 반환하며 가장자리는 부드럽게 보간됩니다.
pub fn ring_antialias(dist: f64, r_inner: f64, r_outer: f64) -> f64 {
    if dist < r_inner - 0.5 || dist > r_outer + 0.5 {
        return 0.0;
    }
    let inner_edge = (dist - (r_inner - 0.5)).clamp(0.0, 1.0);
    let outer_edge = ((r_outer + 0.5) - dist).clamp(0.0, 1.0);
    inner_edge.min(outer_edge)
}

/// RGBA 색상의 알파 채널에 계수를 곱하여 조절합니다.
fn scale_alpha(color: Rgba<u8>, factor: f64) -> Rgba<u8> {
    let a = (color[3] as f64 * factor).round().clamp(0.0, 255.0) as u8;
    Rgba([color[0], color[1], color[2], a])
}

/// 단일 링 쌍(바깥쪽 + 안쪽 동심원 호)을 `size` x `size` 크기의 정사각형 이미지로 렌더링합니다.
///
/// `session_fill` 및 `weekly_fill`은 0.0..=1.0 범위의 비율입니다.
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

    // 링 기하 구조 (기본 크기 44px에 비례)
    let stroke_width = (4.0 * scale).max(2.0);
    let ring_gap = (2.0 * scale).max(1.0);

    let outer_r_outer = center - (1.0 * scale).max(0.5);
    let outer_r_inner = outer_r_outer - stroke_width;
    let inner_r_outer = outer_r_inner - ring_gap;
    let inner_r_inner = inner_r_outer - stroke_width;

    // 트랙 색상: 낮은 알파값의 어두운 링 색상
    let track_alpha = 60u8;
    let outer_track = Rgba([outer_color[0], outer_color[1], outer_color[2], track_alpha]);
    let inner_track = Rgba([inner_color[0], inner_color[1], inner_color[2], track_alpha]);

    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - center;
            let dy = y as f64 + 0.5 - center;
            let dist = (dx * dx + dy * dy).sqrt();

            // 12시 방향 기준 시계 방향 각도, 0..1
            let angle = dx.atan2(-dy);
            let norm_angle = if angle < 0.0 {
                (angle + std::f64::consts::TAU) / std::f64::consts::TAU
            } else {
                angle / std::f64::consts::TAU
            };

            // 바깥쪽 링
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

            // 안쪽 링 (show_inner_ring이 참일 때만 렌더링)
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

    // 빈 중앙 영역에 5시간 세션 숫자 중앙 정렬하여 그리기
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

/// 단일 제공자의 링을 독립형 정사각형 아이콘으로 렌더링합니다(Windows 알림 트레이 슬롯에 적합).
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

    let (outer_c, inner_c, text_color) = ring_colors_for(provider_id, usage);
    let (outer_c, text_color) = if usage.stale {
        (outer_c, text_color)
    } else {
        match settings.ring_outer_color.as_deref().and_then(parse_hex_color) {
            Some(custom) => (custom, custom),
            None => (outer_c, text_color),
        }
    };

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

/// 사용자 지정 링 크기와 간격으로 전체 링 배지 이미지를 렌더링합니다.
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

        let (mut outer_c, mut inner_c, mut text_color) = ring_colors_for(provider_id, usage);
        if !usage.stale && ring_data.is_empty() {
            if let Some(custom) = settings.ring_outer_color.as_deref().and_then(parse_hex_color) {
                outer_c = custom;
                text_color = custom;
            }
            if let Some(custom) = settings.ring_inner_color.as_deref().and_then(parse_hex_color) {
                inner_c = custom;
            }
        }

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

/// Windows DIB / HICON용으로 RgbaImage를 상단 우선(top-down), 사전 곱셈(premultiplied) BGRA 0xAARRGGBB 픽셀로 변환합니다.
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

        // 1. 듀얼 링 모드
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

        // 2. 싱글 링 모드 (안쪽 링 비활성화)
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

        // 3. 32px Windows 트레이 아이콘 크기 (듀얼 링)
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

        // 4. 32px Windows 트레이 아이콘 크기 (싱글 링 확대 숫자)
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

        // 사전 곱셈 BGRA로 변환
        let bgra = rgba_to_bgra_premultiplied(&img_32_dual);
        assert_eq!(bgra.len(), 32 * 32);
        assert!(bgra.iter().any(|&p| (p >> 24) > 0));

        let _ = std::fs::create_dir_all("target");
        let _ = img_32_dual.save("target/test_win_tray_dual_32.png");
        let _ = img_32_single.save("target/test_win_tray_single_32.png");

        // 시각적 아티팩트 검사를 위해 4배 확대 미리보기를 나란히 저장
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
    fn test_provider_text_colors() {        assert_eq!(provider_text_color(ProviderId::Claude), Rgba([255, 138, 61, 255]));
        assert_eq!(provider_text_color(ProviderId::Codex), Rgba([52, 211, 153, 255]));
        assert_eq!(provider_text_color(ProviderId::Antigravity), Rgba([56, 189, 248, 255]));
        assert_eq!(provider_text_color(ProviderId::Cursor), Rgba([34, 211, 238, 255]));
        assert_eq!(provider_text_color(ProviderId::OpenCode), Rgba([192, 132, 252, 255]));
    }

    #[test]
    fn test_stale_usage_renders_gray_instead_of_brand_colors() {
        let fresh = UsageData {
            session: crate::models::UsageSection {
                available: true,
                percentage: 95.0,
                resets_at: None,
            },
            ..Default::default()
        };
        let mut stale = fresh.clone();
        stale.stale = true;

        let (outer, inner, text) = ring_colors_for(ProviderId::Claude, &fresh);
        assert_eq!((outer, inner), provider_ring_palette(ProviderId::Claude));
        assert_eq!(text, provider_text_color(ProviderId::Claude));

        let gray = stale_ring_colors();
        assert_eq!(ring_colors_for(ProviderId::Claude, &stale), gray);
        // 빨강(소진)과 회색(실패)이 같은 색이 아니어야 한다.
        assert_ne!(gray.0, Rgba([255, 0, 0, 255]));
        assert!(gray.0[0] == gray.0[1] && gray.0[1] == gray.0[2]);
    }

    #[test]
    fn test_ring_badge_track_alpha_preservation() {
        // 0% 채움률로 44px 링을 렌더링하여 트랙(알파 60)만 렌더링되도록 함.
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

        // 트랙 영역(바깥쪽 링 스트로크)의 픽셀 찾기.
        // 중심은 22.0이며, 바깥쪽 링 반경은 약 19..20 부근.
        // 수평축을 따라 샘플링: (x=2, y=22) 또는 (x=3, y=22)
        // (22, 22)로부터의 거리: (3.5, 22.5)의 경우 -> dist = 18.5
        let center = 22.0f64;
        let mut max_track_alpha = 0u8;
        for y in 0..44 {
            for x in 0..44 {
                let dx = x as f64 + 0.5 - center;
                let dy = y as f64 + 0.5 - center;
                let dist = (dx * dx + dy * dy).sqrt();
                // 바깥쪽 링 트랙 반경
                if (17.5..=20.0).contains(&dist) {
                    let pixel = img.get_pixel(x, y);
                    if pixel[3] > max_track_alpha {
                        max_track_alpha = pixel[3];
                    }
                }
            }
        }

        // 트랙 알파는 60으로 정의됨. 안티앨리어싱 = 1.0일 때 최대 픽셀 알파는 60이어야 함.
        // 기존 버그가 있던 구현에서는 (60/255)^2 * 255 = 14였음!
        assert_eq!(
            max_track_alpha, 60,
            "Track alpha should be 60 at full coverage, but got {}",
            max_track_alpha
        );

        // 설정 파일을 사용하는 render_ring_badge_image_at_size도 테스트
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
