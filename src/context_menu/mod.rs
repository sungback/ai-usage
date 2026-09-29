#![cfg_attr(not(windows), allow(dead_code))]

//! 재사용 가능한 버전 관리 컨텍스트 메뉴 문서.
//!
//! # 모듈 구조
//! - `model`    : 데이터 구조체/열거형, 유효성 검사
//! - `builtins` : 내장 메뉴 템플릿 (Classic v1)
//! - `io`       : 파일 로드/저장/목록/삭제

pub mod model;
pub mod builtins;
pub mod io;

pub use model::{ContextMenuAction, ContextMenuItem, ContextMenuItemKind};
pub use builtins::classic_context_menu;
#[cfg(windows)]
pub use io::resolve_context_menu;

// ── 렌더링 헬퍼 ──────────────────────────────────────────────────────────

/// 메뉴 항목 레이블을 현재 언어로 번역 후 템플릿 변수 치환.
pub fn rendered_label(
    language: crate::localization::LanguageId,
    label: &str,
    context: &crate::theme_engine::DataContext,
) -> String {
    let translated = match label {
        "Settings" => language.text("Settings"),
        "Update frequency" => language.text("Update frequency"),
        "Start with Windows" => language.text("Start with Windows"),
        "Refresh" => language.text("Refresh"),
        "Exit" => language.text("Exit"),
        "Claude Code" => language.text("Claude Code"),
        "Codex" => language.text("Codex"),
        "Antigravity" => language.text("Antigravity"),
        "OpenCode" => language.text("OpenCode"),
        "Cursor" => language.text("Cursor"),
        "Every minute" => language.text("Every minute"),
        "Every 5 minutes" => language.text("Every 5 minutes"),
        "Every 15 minutes" => language.text("Every 15 minutes"),
        "Every hour" => language.text("Every hour"),
        "Providers" => language.text("Providers"),
        "Show widget" => language.text("Show widget"),
        "Show Weekly (7D) Ring" => {
            if language.code().starts_with("ko") { "주간(7일) 안쪽 링 표시" } else { "Show Weekly (7D) Ring" }
        }
        "Show Remaining Allowance" => {
            if language.code().starts_with("ko") { "남은 양 표시" } else { "Show Remaining Allowance" }
        }
        "Concentric Ring Badge" => {
            if language.code().starts_with("ko") { "동심원 링 배지" } else { "Concentric Ring Badge" }
        }
        "Display Order" => {
            if language.code().starts_with("ko") { "표시 순서" } else { "Display Order" }
        }
        "Move Up" => {
            if language.code().starts_with("ko") { "한 칸 앞으로 이동" } else { "Move Up" }
        }
        "Move Down" => {
            if language.code().starts_with("ko") { "한 칸 뒤로 이동" } else { "Move Down" }
        }
        "Reset Order" => {
            if language.code().starts_with("ko") { "순서 초기화" } else { "Reset Order" }
        }
        "Check for updates" => language.text("Check for updates"),
        _ => label,
    };
    crate::theme_engine::format_template(translated, context)
}

// ── 테스트 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::model::{
        ContextMenuDocument, CLASSIC_CONTEXT_MENU_ID, LEGACY_CLASSIC_CONTEXT_MENU_ID,
    };
    use crate::theme_engine::DataContext;

    /// v2.12.47 and earlier wrote `set_language` rows into the on-disk menu file.
    /// A user upgrading straight from those versions still has that file, so the
    /// document has to load even though the action can no longer be built.
    #[test]
    fn removed_legacy_actions_are_rejected() {
        let legacy = r#"{
            "schema_version": 1, "id": "classic-v1", "name": "Classic v1",
            "items": [
                {"id":"refresh","type":"action","label":"Refresh","action":{"type":"refresh"}},
                {"id":"language-system","type":"action","label":"System default",
                 "action":{"type":"set_language","language":"system"}}
            ]
        }"#;
        // set_language는 제거된 옛 형식이라 파싱이 실패해야 한다. 로드 실패 시
        // 호출자는 내장 Classic 메뉴로 떨어지므로 메뉴가 비는 일은 없다.
        let parsed = serde_json::from_str::<ContextMenuDocument>(legacy);
        assert!(parsed.is_err(), "removed legacy actions must not parse");
    }

    #[test]
    fn classic_menu_is_valid_and_contains_the_v1_controls() {
        let menu = classic_context_menu();
        assert_eq!(menu.id, CLASSIC_CONTEXT_MENU_ID);
        assert_eq!(menu.name, "Classic v1");
        assert!(!serde_json::to_string(&menu).unwrap().contains("description"));
        assert!(menu.validate().is_empty());
        assert!(menu.items.iter().any(|item| item.id == "refresh"));
        assert!(menu.items.iter().any(|item| item.id == "update-frequency"));
        assert!(serde_json::to_string(&menu).unwrap().contains("provider-opencode"));
        assert!(serde_json::to_string(&menu).unwrap().contains("provider-cursor"));
        assert!(menu.items.iter().any(|item| {
            item.id == "toggle-widget"
                && matches!(
                    &item.kind,
                    ContextMenuItemKind::Action { action: ContextMenuAction::ToggleWidget }
                )
        }));
        assert!(menu.items.iter().any(|item| {
            item.id == "check-for-updates"
                && matches!(
                    &item.kind,
                    ContextMenuItemKind::Action { action: ContextMenuAction::CheckForUpdates }
                )
        }));
        assert!(!serde_json::to_string(&menu).unwrap().contains("reset_position"));
        let mut legacy = menu.clone();
        legacy.id = LEGACY_CLASSIC_CONTEXT_MENU_ID.into();
        assert!(legacy.is_builtin());
    }

    #[test]
    fn test_classic_menu_contains_order_and_inner_ring() {
        let menu = classic_context_menu();
        assert!(menu.validate().is_empty(), "{:?}", menu.validate());

        let settings = menu.items.iter().find(|i| i.id == "settings").expect("settings submenu");
        if let ContextMenuItemKind::Submenu { items } = &settings.kind {
            assert!(items.iter().any(|i| {
                i.id == "show-inner-ring"
                    && matches!(&i.kind, ContextMenuItemKind::Action { action: ContextMenuAction::ToggleInnerRing })
            }));
            assert!(items.iter().any(|i| {
                i.id == "usage-direction"
                    && matches!(&i.kind, ContextMenuItemKind::Action { action: ContextMenuAction::ToggleUsageDirection })
            }));
            assert!(items.iter().any(|i| {
                i.id == "taskbar-ring-badge"
                    && matches!(&i.kind, ContextMenuItemKind::Action { action: ContextMenuAction::ToggleTaskbarRingBadge })
            }));
        } else {
            panic!("settings is not a submenu");
        }

        let providers = menu.items.iter().find(|i| i.id == "providers").expect("providers submenu");
        if let ContextMenuItemKind::Submenu { items } = &providers.kind {
            let order = items.iter().find(|i| i.id == "display-order").expect("display-order");
            if let ContextMenuItemKind::Submenu { items: order_items } = &order.kind {
                assert!(order_items.iter().any(|i| i.id == "order-claude"));
                assert!(order_items.iter().any(|i| {
                    i.id == "order-reset"
                        && matches!(&i.kind, ContextMenuItemKind::Action { action: ContextMenuAction::ResetProviderOrder })
                }));
            } else {
                panic!("display-order is not a submenu");
            }
        } else {
            panic!("providers is not a submenu");
        }

        let ko = crate::localization::LanguageId::Korean;
        let ctx = DataContext::default();
        assert_eq!(rendered_label(ko, "Show Weekly (7D) Ring", &ctx), "주간(7일) 안쪽 링 표시");
        assert_eq!(rendered_label(ko, "Display Order", &ctx), "표시 순서");
        assert_eq!(rendered_label(ko, "Move Up", &ctx), "한 칸 앞으로 이동");
        assert_eq!(rendered_label(ko, "Move Down", &ctx), "한 칸 뒤로 이동");
        assert_eq!(rendered_label(ko, "Reset Order", &ctx), "순서 초기화");
        assert_eq!(rendered_label(ko, "Check for updates", &ctx), "업데이트 확인");
    }

    #[test]
    fn legacy_frequency_milliseconds_are_normalized_to_seconds() {
        let action: ContextMenuAction =
            serde_json::from_str(r#"{"type":"set_update_frequency","milliseconds":300000}"#).unwrap();
        assert_eq!(action, ContextMenuAction::SetUpdateFrequency { seconds: 300 });
        let serialized = serde_json::to_string(&action).unwrap();
        assert!(serialized.contains(r#""seconds":300"#));
        assert!(!serialized.contains("milliseconds"));
    }

    #[test]
    fn duplicate_item_ids_are_rejected_across_submenus() {
        let mut menu = classic_context_menu();
        menu.items.push(ContextMenuItem::submenu(
            "submenu",
            "Submenu",
            vec![ContextMenuItem::action("exit", "Duplicate", ContextMenuAction::Exit)],
        ));
        assert!(menu.validate().iter().any(|e| e.contains("unique id")));
    }

    #[test]
    fn informational_text_accepts_usage_and_version_templates() {
        let mut menu = classic_context_menu();
        menu.items.insert(0, ContextMenuItem::text("usage", "v{app.version} - {claude.session:usage_line}"));
        assert!(menu.validate().is_empty());
    }

    #[test]
    fn monthly_menu_labels_validate_without_live_usage() {
        use crate::providers::ProviderId;
        let mut menu = classic_context_menu();
        for provider in ProviderId::ALL {
            let name = provider.descriptor().key;
            let mut item = ContextMenuItem::text(
                &format!("{name}-monthly"),
                &format!("{{{name}.monthly.label}}: {{{name}.monthly.remaining:0}}% left"),
            );
            item.render = crate::theme_engine::Expression(format!(
                "providers.{name}.enabled && {name}.monthly.available"
            ));
            menu.items.push(item);
        }
        assert!(menu.validate().is_empty(), "{:?}", menu.validate());
    }

    #[test]
    fn legacy_items_default_to_visible_and_conditions_round_trip() {
        use crate::theme_engine::{Canvas, Expression};
        let mut item: ContextMenuItem =
            serde_json::from_str(r#"{"id":"usage","type":"text","label":"Usage"}"#).unwrap();
        let mut context = DataContext::from_usage(None, &Canvas::default());
        assert!(item.should_render(&context));
        assert_eq!(item.render.0, "1");
        item.render = Expression("providers.codex.enabled && codex.weekly.available".into());
        let encoded = serde_json::to_string(&item).unwrap();
        assert_eq!(serde_json::from_str::<ContextMenuItem>(&encoded).unwrap(), item);
        context.insert("providers.codex.enabled", 1.0);
        assert!(!item.should_render(&context));
        context.insert("codex.weekly.available", 1.0);
        assert!(item.should_render(&context));
        context.insert("providers.codex.enabled", 0.0);
        assert!(!item.should_render(&context));
    }

    #[test]
    fn menu_render_rejects_invalid_values_and_hides_falsy_rows() {
        use crate::theme_engine::{Canvas, Expression};
        let context = DataContext::from_usage(None, &Canvas::default());
        let mut menu = classic_context_menu();
        for source in ["missing.variable", "1 +", "sqrt(-1)", "\"text\""] {
            menu.items[0].render = Expression(source.into());
            assert!(!menu.items[0].should_render(&context));
            assert!(menu.validate().iter().any(|e| e.contains(".render")));
        }
        let item = &mut menu.items[0];
        for (render, visible) in [("0", false), ("false", false), ("-1", true), ("1", true), ("true", true)] {
            item.render = Expression(render.into());
            assert_eq!(item.should_render(&context), visible);
        }
    }

    #[test]
    fn the_native_menu_offers_no_dashboard_entry() {
        let menu = classic_context_menu();
        let rendered = serde_json::to_string(&menu).unwrap();
        assert!(!rendered.contains("dashboard"), "{rendered}");
        assert!(!rendered.contains("open-dashboard"), "{rendered}");
    }
}
