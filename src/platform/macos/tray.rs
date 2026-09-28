//! macOS 트레이 메뉴 빌드 및 메뉴바 타이틀/툴팁 계산.

use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    Icon,
};

use super::startup::is_startup_enabled;

// ── 내부 포맷 헬퍼 ─────────────────────────────────────────────────────────

pub(super) fn format_percentage(pct: f64, countdown: bool) -> String {
    let display_pct = if countdown {
        (100.0 - pct).clamp(0.0, 100.0)
    } else {
        pct
    };
    format!("{:.0}%", display_pct)
}

fn format_compact_reset(resets_at: Option<std::time::SystemTime>) -> Option<String> {
    let resets_at = resets_at?;
    let now = std::time::SystemTime::now();
    if resets_at <= now {
        return None;
    }
    let diff = resets_at.duration_since(now).ok()?;
    let seconds = diff.as_secs();
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let mins = (seconds % 3_600) / 60;
    if days > 0 {
        Some(format!("{days}d"))
    } else if hours > 0 {
        Some(format!("{hours}h"))
    } else if mins > 0 {
        Some(format!("{mins}m"))
    } else {
        None
    }
}

pub(super) fn format_reset_time(resets_at: Option<std::time::SystemTime>) -> Option<String> {
    let resets_at = resets_at?;
    let now = std::time::SystemTime::now();
    if resets_at > now {
        let diff = resets_at.duration_since(now).ok()?;
        let total_mins = diff.as_secs() / 60;
        let hours = total_mins / 60;
        let mins = total_mins % 60;
        if hours > 0 {
            Some(format!("{hours}h {mins}m"))
        } else {
            Some(format!("{mins}m"))
        }
    } else {
        Some("Now".to_string())
    }
}

fn format_window_badge(
    section: &crate::models::UsageSection,
    countdown: bool,
    show_reset: bool,
) -> String {
    let pct_str = format_percentage(section.percentage, countdown);
    if show_reset {
        if let Some(reset_str) = format_compact_reset(section.resets_at) {
            return format!("{pct_str}({reset_str})");
        }
    }
    pct_str
}

// ── 메뉴바 타이틀 / 툴팁 ──────────────────────────────────────────────────

pub fn compute_menu_bar_title(
    data: &Option<crate::models::AppUsageData>,
    settings: &crate::app_settings::SettingsFile,
    lang: crate::localization::LanguageId,
) -> String {
    let Some(data) = data else {
        return "⚡ --%".to_string();
    };

    let active_providers: Vec<(crate::providers::ProviderId, &crate::models::UsageData)> =
        data.iter().collect();

    if active_providers.is_empty() {
        return "⚡ --%".to_string();
    }

    let countdown = settings.usage_countdown;

    if active_providers.len() == 1 {
        let (_id, usage) = active_providers[0];
        let session_str = format_window_badge(&usage.session, countdown, true);
        let weekly_str = format_window_badge(&usage.weekly, countdown, true);
        let session_label = if lang.code() == "ko" { "5시간" } else { "5h" };
        let weekly_label = usage.weekly_label.as_deref().unwrap_or(if lang.code() == "ko" {
            "7일"
        } else {
            "7d"
        });
        format!("{session_label} {session_str} · {weekly_label} {weekly_str}")
    } else {
        let mut parts = Vec::new();
        for (provider_id, usage) in active_providers {
            let short_name = match provider_id {
                crate::providers::ProviderId::Claude => "Claude",
                crate::providers::ProviderId::Codex => "Codex",
                crate::providers::ProviderId::Antigravity => "Anti",
                crate::providers::ProviderId::OpenCode => "Open",
                crate::providers::ProviderId::Cursor => "Cursor",
            };
            let s_badge = format_window_badge(&usage.session, countdown, true);
            let w_badge = format_window_badge(&usage.weekly, countdown, true);
            parts.push(format!("{short_name}: {s_badge} {w_badge}"));
        }
        parts.join(" | ")
    }
}

pub fn compute_tooltip(
    data: &Option<crate::models::AppUsageData>,
    countdown: bool,
    lang: crate::localization::LanguageId,
) -> String {
    let strings = lang.strings();
    let Some(data) = data else {
        return format!("{}\n{}", strings.window_title, lang.text("Loading..."));
    };

    let mut lines = vec![strings.window_title.to_string()];

    for (provider, usage) in data.iter() {
        let desc = provider.descriptor();
        let provider_name = lang.text(desc.display_name);
        let (session_pct, weekly_pct) = if countdown {
            (
                (100.0 - usage.session.percentage).clamp(0.0, 100.0),
                (100.0 - usage.weekly.percentage).clamp(0.0, 100.0),
            )
        } else {
            (usage.session.percentage, usage.weekly.percentage)
        };
        lines.push(format!(
            "{} - {}: {:.0}% | {}: {:.0}%",
            provider_name,
            strings.session_window,
            session_pct,
            usage.weekly_label.as_deref().unwrap_or(strings.weekly_window),
            weekly_pct
        ));
        if let Some(t) = format_reset_time(usage.session.resets_at) {
            lines.push(format!("  {}: {t}", lang.text("Session Reset")));
        }
        if let Some(t) = format_reset_time(usage.weekly.resets_at) {
            lines.push(format!("  {}: {t}", lang.text("Weekly Reset")));
        }
    }

    lines.join("\n")
}

// ── 컨텍스트 메뉴 빌드 ────────────────────────────────────────────────────

pub fn build_context_menu(
    data: &Option<crate::models::AppUsageData>,
    available_update: &Option<crate::updater::ReleaseDescriptor>,
    settings: &crate::app_settings::SettingsFile,
    lang: crate::localization::LanguageId,
) -> Menu {
    let menu = Menu::new();
    let strings = lang.strings();

    // 1. 사용량 요약 헤더
    if let Some(data) = data {
        for (provider, usage) in data.iter() {
            let desc = provider.descriptor();
            let provider_name = lang.text(desc.display_name);
            let (session_pct, weekly_pct) = if settings.usage_countdown {
                (
                    (100.0 - usage.session.percentage).clamp(0.0, 100.0),
                    (100.0 - usage.weekly.percentage).clamp(0.0, 100.0),
                )
            } else {
                (usage.session.percentage, usage.weekly.percentage)
            };
            let header_text = format!(
                "{} - {}: {:.0}% | {}: {:.0}%",
                provider_name,
                strings.session_window,
                session_pct,
                usage.weekly_label.as_deref().unwrap_or(strings.weekly_window),
                weekly_pct
            );
            let _ = menu.append(&MenuItem::new(header_text, false, None));
            if let Some(reset_str) = format_reset_time(usage.session.resets_at) {
                let _ = menu.append(&MenuItem::new(
                    format!("  {} {reset_str}", lang.text("Resets in:")),
                    false,
                    None,
                ));
            }
        }
    } else {
        let _ = menu.append(&MenuItem::new(lang.text("Loading usage..."), false, None));
    }

    let _ = menu.append(&PredefinedMenuItem::separator());

    // 2. 즉시 새로 고침
    let _ = menu.append(&MenuItem::with_id(
        "refresh",
        format!("🔄 {}", lang.text("Refresh now")),
        true,
        None,
    ));
    let _ = menu.append(&PredefinedMenuItem::separator());

    // 3. 퀵 설정
    let countdown_title = if lang.code() == "ko" {
        "남은 양으로 표시 (Remaining)"
    } else {
        "Show Remaining Allowance"
    };
    let _ = menu.append(&CheckMenuItem::with_id(
        "countdown",
        countdown_title,
        true,
        settings.usage_countdown,
        None,
    ));

    let inner_ring_title = if lang.code() == "ko" {
        "주간(7일) 안쪽 링 표시"
    } else {
        "Show Weekly (7D) Ring"
    };
    let _ = menu.append(&CheckMenuItem::with_id(
        "inner_ring",
        inner_ring_title,
        true,
        settings.show_inner_ring,
        None,
    ));

    let _ = menu.append(&CheckMenuItem::with_id(
        "startup",
        lang.text("Launch at Login"),
        true,
        is_startup_enabled(),
        None,
    ));

    // 4. 프로바이더(모델) 서브메뉴
    let models_menu = Submenu::new(lang.strings().models, true);
    let providers = [
        ("model:claude", lang.strings().claude_code_model, crate::providers::ProviderId::Claude),
        ("model:codex", lang.strings().codex_model, crate::providers::ProviderId::Codex),
        ("model:antigravity", lang.strings().antigravity_model, crate::providers::ProviderId::Antigravity),
        ("model:opencode", lang.strings().opencode_model, crate::providers::ProviderId::OpenCode),
        ("model:cursor", lang.strings().cursor_model, crate::providers::ProviderId::Cursor),
    ];
    for (id, label, provider) in providers {
        let _ = models_menu.append(&CheckMenuItem::with_id(
            id,
            label,
            true,
            settings.provider_enabled(provider),
            None,
        ));
    }
    append_order_submenu(&models_menu, settings, lang);
    let _ = menu.append(&models_menu);

    // 5. 업데이트
    if let Some(update) = available_update {
        let _ = menu.append(&MenuItem::with_id(
            "update",
            format!("{} ({})", lang.text("Install Update"), update.latest_version),
            true,
            None,
        ));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());

    // 6. 종료
    let _ = menu.append(&MenuItem::with_id(
        "quit",
        lang.text("Quit Usage Monitor"),
        true,
        None,
    ));

    menu
}

fn append_order_submenu(
    models_menu: &tray_icon::menu::Submenu,
    settings: &crate::app_settings::SettingsFile,
    lang: crate::localization::LanguageId,
) {
    let enabled_order = settings.enabled_ordered_providers();
    if enabled_order.len() < 2 {
        return;
    }

    let _ = models_menu.append(&PredefinedMenuItem::separator());
    let is_korean = lang.code() == "ko";
    let order_menu = Submenu::new(
        if is_korean { "표시 순서 변경" } else { "Display Order" },
        true,
    );

    for (idx, &provider) in enabled_order.iter().enumerate() {
        let name = match provider {
            crate::providers::ProviderId::Codex => "Codex",
            crate::providers::ProviderId::Claude => "Claude Code",
            crate::providers::ProviderId::Antigravity => "Antigravity",
            crate::providers::ProviderId::OpenCode => "OpenCode",
            crate::providers::ProviderId::Cursor => "Cursor",
        };
        let provider_submenu = Submenu::new(format!("{}. {name}", idx + 1), true);
        let key = provider.descriptor().key;

        if idx > 0 {
            let _ = provider_submenu.append(&MenuItem::with_id(
                format!("order:up:{key}"),
                if is_korean { "▲ 한 칸 앞으로 이동" } else { "▲ Move Up" },
                true,
                None,
            ));
        }
        if idx + 1 < enabled_order.len() {
            let _ = provider_submenu.append(&MenuItem::with_id(
                format!("order:down:{key}"),
                if is_korean { "▼ 한 칸 뒤로 이동" } else { "▼ Move Down" },
                true,
                None,
            ));
        }
        let _ = order_menu.append(&provider_submenu);
    }

    let _ = order_menu.append(&PredefinedMenuItem::separator());
    let _ = order_menu.append(&MenuItem::with_id(
        "order:reset",
        if is_korean { "↺ 기본 순서로 초기화" } else { "↺ Reset to Default" },
        true,
        None,
    ));
    let _ = models_menu.append(&order_menu);
}

// ── 뱃지 아이콘 생성 ──────────────────────────────────────────────────────

pub fn render_compact_menu_badge(
    data: &Option<crate::models::AppUsageData>,
    settings: &crate::app_settings::SettingsFile,
) -> Option<Icon> {
    let default_data = crate::models::AppUsageData::default();
    let data = data.as_ref().unwrap_or(&default_data);
    let img = crate::platform::ring_badge::render_ring_badge_image(data, settings)?;
    let (width, height) = (img.width(), img.height());
    Icon::from_rgba(img.into_raw(), width, height).ok()
}

pub fn load_app_icon() -> Option<Icon> {
    let bytes = include_bytes!("../../icons/32x32.png");
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}
