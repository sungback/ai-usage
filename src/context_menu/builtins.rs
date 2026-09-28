//! 내장(빌트인) 컨텍스트 메뉴 정의.
//! - `classic_context_menu()`: 전통 방식 메뉴 (Classic v1)

use super::model::*;

/// Classic v1 내장 메뉴 — 모든 기능을 포함한 풀-기능 메뉴.
pub fn classic_context_menu() -> ContextMenuDocument {
    use ContextMenuAction as Action;
    use ContextMenuProvider as Provider;

    // 업데이트 주기 서브메뉴
    let frequency = ContextMenuItem::submenu(
        "update-frequency",
        "Update frequency",
        vec![
            ContextMenuItem::action(
                "frequency-1-minute",
                "Every minute",
                Action::SetUpdateFrequency { seconds: crate::app_settings::POLL_1_MIN_SECONDS },
            ),
            ContextMenuItem::action(
                "frequency-5-minutes",
                "Every 5 minutes",
                Action::SetUpdateFrequency { seconds: crate::app_settings::POLL_5_MIN_SECONDS },
            ),
            ContextMenuItem::action(
                "frequency-15-minutes",
                "Every 15 minutes",
                Action::SetUpdateFrequency { seconds: crate::app_settings::POLL_15_MIN_SECONDS },
            ),
            ContextMenuItem::action(
                "frequency-1-hour",
                "Every hour",
                Action::SetUpdateFrequency { seconds: crate::app_settings::POLL_1_HOUR_SECONDS },
            ),
        ],
    );

    // 표시 순서 서브메뉴
    let mut order_items = Vec::new();
    for provider in [
        Provider::Claude,
        Provider::Codex,
        Provider::Antigravity,
        Provider::OpenCode,
        Provider::Cursor,
    ] {
        let key = provider.descriptor().key;
        let name = provider.descriptor().display_name;
        order_items.push(ContextMenuItem::submenu(
            &format!("order-{key}"),
            name,
            vec![
                ContextMenuItem::action(
                    &format!("order-up-{key}"),
                    "Move Up",
                    Action::MoveProviderUp { provider: key.to_string() },
                ),
                ContextMenuItem::action(
                    &format!("order-down-{key}"),
                    "Move Down",
                    Action::MoveProviderDown { provider: key.to_string() },
                ),
            ],
        ));
    }
    order_items.push(ContextMenuItem::separator("order-separator"));
    order_items.push(ContextMenuItem::action("order-reset", "Reset Order", Action::ResetProviderOrder));
    let display_order = ContextMenuItem::submenu("display-order", "Display Order", order_items);

    // 프로바이더 서브메뉴
    let providers = ContextMenuItem::submenu(
        "providers",
        "Providers",
        vec![
            ContextMenuItem::action("provider-claude", "Claude Code", Action::ToggleProvider { provider: Provider::Claude }),
            ContextMenuItem::action("provider-codex", "Codex", Action::ToggleProvider { provider: Provider::Codex }),
            ContextMenuItem::action("provider-antigravity", "Antigravity", Action::ToggleProvider { provider: Provider::Antigravity }),
            ContextMenuItem::action("provider-opencode", "OpenCode", Action::ToggleProvider { provider: Provider::OpenCode }),
            ContextMenuItem::action("provider-cursor", "Cursor", Action::ToggleProvider { provider: Provider::Cursor }),
            ContextMenuItem::separator("providers-order-separator"),
            display_order,
        ],
    );

    // 설정 서브메뉴
    let settings = ContextMenuItem::submenu(
        "settings",
        "Settings",
        vec![
            ContextMenuItem::action("show-inner-ring", "Show Weekly (7D) Ring", Action::ToggleInnerRing),
            ContextMenuItem::action("usage-direction", "Show Remaining Allowance", Action::ToggleUsageDirection),
            ContextMenuItem::action("taskbar-ring-badge", "Concentric Ring Badge", Action::ToggleTaskbarRingBadge),
            ContextMenuItem::action("start-with-windows", "Start with Windows", Action::ToggleStartup),
        ],
    );

    ContextMenuDocument {
        schema_version: CONTEXT_MENU_SCHEMA_VERSION,
        id: CLASSIC_CONTEXT_MENU_ID.into(),
        name: "Classic v1".into(),
        items: vec![
            ContextMenuItem::action("refresh", "Refresh", Action::Refresh),
            frequency,
            providers,
            settings,
            ContextMenuItem::action("toggle-widget", "Show widget", Action::ToggleWidget),
            ContextMenuItem::separator("root-separator"),
            ContextMenuItem::action("exit", "Exit", Action::Exit),
        ],
    }
}
