//! 컨텍스트 메뉴의 핵심 데이터 모델 (구조체, 열거형, 직렬화).

use serde::{Deserialize, Deserializer, Serialize};

use crate::providers::ProviderId;
use crate::theme_engine::{self, Expression};

pub const CONTEXT_MENU_SCHEMA_VERSION: u32 = 1;
pub const CLASSIC_CONTEXT_MENU_ID: &str = "classic-v1";
pub const LEGACY_CLASSIC_CONTEXT_MENU_ID: &str = "classic-v1-4-9";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMenuDocument {
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<ContextMenuItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMenuItem {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default = "theme_engine::default_render")]
    pub render: Expression,
    #[serde(flatten)]
    pub kind: ContextMenuItemKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextMenuItemKind {
    Action { action: ContextMenuAction },
    /// 클릭 불가 정보 행. 메뉴가 열릴 때마다 label을 템플릿으로 평가.
    Text,
    Separator,
    Submenu { items: Vec<ContextMenuItem> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextMenuAction {
    Refresh,
    SetUpdateFrequency {
        #[serde(
            alias = "milliseconds",
            deserialize_with = "deserialize_update_frequency_seconds"
        )]
        seconds: u32,
    },
    ToggleProvider { provider: ContextMenuProvider },
    ToggleInnerRing,
    ToggleUsageDirection,
    ToggleTaskbarRingBadge,
    MoveProviderUp { provider: String },
    MoveProviderDown { provider: String },
    ResetProviderOrder,
    ToggleStartup,
    ToggleWidget,
    /// 구버전 저장 메뉴 로드·정리용. 신규 메뉴에서는 생성/실행 불가.
    #[serde(rename = "reset_position")]
    LegacyResetPosition,
    /// v2.12.47 이하가 기록한 언어 항목. 로드만 허용하고 곧바로 제거한다.
    #[serde(rename = "set_language")]
    LegacySetLanguage { language: String },
    CheckForUpdates,
    ToggleLayerRender { target: String },
    LayerActions { actions: String },
    OpenUrl { url: String },
    Exit,
}

fn deserialize_update_frequency_seconds<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    Ok(match value {
        crate::app_settings::POLL_1_MIN
        | crate::app_settings::POLL_5_MIN
        | crate::app_settings::POLL_15_MIN
        | crate::app_settings::POLL_1_HOUR => value / 1_000,
        _ => value,
    })
}

pub type ContextMenuProvider = ProviderId;

#[derive(Clone, Debug)]
pub struct ContextMenuDescriptor {
    pub path: std::path::PathBuf,
    pub id: String,
    pub name: String,
    pub built_in: bool,
}

pub fn schema_version() -> u32 {
    CONTEXT_MENU_SCHEMA_VERSION
}

impl ContextMenuItem {
    /// render 조건이 false/invalid이면 항목 숨김.
    pub fn should_render(&self, context: &crate::theme_engine::DataContext) -> bool {
        theme_engine::evaluate(&self.render.0, context)
            .is_ok_and(|value| value.is_finite() && value != 0.0)
    }

    pub fn action(id: &str, label: &str, action: ContextMenuAction) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Action { action },
        }
    }

    pub fn separator(id: &str) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Separator,
        }
    }

    #[cfg(test)]
    pub fn text(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Text,
        }
    }

    pub fn submenu(id: &str, label: &str, items: Vec<Self>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            render: theme_engine::default_render(),
            kind: ContextMenuItemKind::Submenu { items },
        }
    }
}

impl ContextMenuDocument {
    pub fn is_builtin(&self) -> bool {
        self.id.eq_ignore_ascii_case(CLASSIC_CONTEXT_MENU_ID)
            || self.id.eq_ignore_ascii_case(LEGACY_CLASSIC_CONTEXT_MENU_ID)
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.schema_version != CONTEXT_MENU_SCHEMA_VERSION {
            errors.push(format!(
                "Context menu schema {} is not supported; expected {}",
                self.schema_version, CONTEXT_MENU_SCHEMA_VERSION
            ));
        }
        if self.id.trim().is_empty() {
            errors.push("Context menu id cannot be empty".into());
        }
        if self.name.trim().is_empty() {
            errors.push("Context menu name cannot be empty".into());
        }
        if self.items.is_empty() {
            errors.push("A context menu needs at least one item".into());
        }
        let mut ids = std::collections::HashSet::new();
        validate_items(&self.items, 0, &mut ids, &mut errors);
        errors
    }
}

// ── 내부 유효성 검사 ──────────────────────────────────────────────────────

fn validate_items(
    items: &[ContextMenuItem],
    depth: usize,
    ids: &mut std::collections::HashSet<String>,
    errors: &mut Vec<String>,
) {
    if depth > 5 {
        errors.push("Context menus support at most six nested levels".into());
        return;
    }
    let context = crate::theme_engine::DataContext::from_usage(
        None,
        &crate::theme_engine::Canvas::default(),
    );
    for item in items {
        if item.id.trim().is_empty() || !ids.insert(item.id.to_ascii_lowercase()) {
            errors.push(format!("Menu item '{}' needs a unique id", item.label));
        }
        match crate::theme_engine::evaluate(&item.render.0, &context) {
            Ok(value) if value.is_finite() => {}
            Ok(_) => errors.push(format!("{}.render did not produce a finite value", item.id)),
            Err(error) => errors.push(format!("{}.render: {error}", item.id)),
        }
        match &item.kind {
            ContextMenuItemKind::Separator => {}
            ContextMenuItemKind::Text => validate_item_label(item, "Menu text", errors),
            ContextMenuItemKind::Submenu { items } => {
                validate_item_label(item, "Submenu", errors);
                if items.is_empty() {
                    errors.push(format!("Submenu '{}' needs at least one item", item.label));
                }
                validate_items(items, depth + 1, ids, errors);
            }
            ContextMenuItemKind::Action { action } => {
                validate_item_label(item, "Menu action", errors);
                match action {
                    ContextMenuAction::SetUpdateFrequency { seconds }
                        if !matches!(
                            *seconds,
                            crate::app_settings::POLL_1_MIN_SECONDS
                                | crate::app_settings::POLL_5_MIN_SECONDS
                                | crate::app_settings::POLL_15_MIN_SECONDS
                                | crate::app_settings::POLL_1_HOUR_SECONDS
                        ) =>
                    {
                        errors.push(format!(
                            "{}.seconds must be a supported update frequency",
                            item.id
                        ));
                    }
                    ContextMenuAction::ToggleLayerRender { target } if target.trim().is_empty() => {
                        errors.push(format!("{}.target cannot be empty", item.id));
                    }
                    ContextMenuAction::LayerActions { actions } => {
                        if actions.trim().is_empty() {
                            errors.push(format!("{}.actions cannot be empty", item.id));
                        } else if let Err(error) =
                            crate::theme_engine::parse_mouse_actions(actions)
                        {
                            errors.push(format!("{}.actions: {error}", item.id));
                        }
                    }
                    ContextMenuAction::OpenUrl { url } if !crate::theme_engine::supported_url(url) => {
                        errors.push(format!(
                            "{}.url must start with http:// or https://",
                            item.id
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
}

fn validate_item_label(item: &ContextMenuItem, kind: &str, errors: &mut Vec<String>) {
    if item.label.trim().is_empty() {
        errors.push(format!("{kind} '{}' needs a label", item.id));
        return;
    }
    let context = crate::theme_engine::DataContext::from_usage(
        None,
        &crate::theme_engine::Canvas::default(),
    );
    for error in crate::theme_engine::validate_template(&item.label, &context) {
        errors.push(format!("{}.label: {error}", item.id));
    }
}

/// 레거시 LegacyResetPosition 액션 재귀 제거.
pub fn remove_legacy_context_menu_actions(items: &mut Vec<ContextMenuItem>) {
    for item in items.iter_mut() {
        if let ContextMenuItemKind::Submenu { items } = &mut item.kind {
            remove_legacy_context_menu_actions(items);
        }
    }
    items.retain(|item| match &item.kind {
        ContextMenuItemKind::Action {
            action: ContextMenuAction::LegacyResetPosition,
        }
        | ContextMenuItemKind::Action {
            action: ContextMenuAction::LegacySetLanguage { .. },
        } => false,
        ContextMenuItemKind::Submenu { items } => !items.is_empty(),
        _ => true,
    });
}
