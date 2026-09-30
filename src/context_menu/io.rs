//! 컨텍스트 메뉴 파일 I/O: 로드, 저장, 목록 조회, 삭제, ID 생성.

use std::path::{Path, PathBuf};

use super::model::*;
use super::builtins::classic_context_menu;

pub fn context_menus_directory() -> PathBuf {
    crate::app_settings::app_data_directory().join("context-menus")
}

/// 내장 메뉴를 디렉토리에 기록하고 Classic 메뉴 경로를 반환.
pub fn ensure_builtin_context_menus() -> Result<PathBuf, String> {
    let directory = context_menus_directory();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;

    let mut classic_path = None;
    let document = classic_context_menu();
    let errors = document.validate();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let path = directory.join(format!("{}.json", document.id));
    let canonical = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
    if std::fs::read(&path).ok().as_deref() != Some(canonical.as_slice()) {
        crate::app_settings::write_json_atomic(&path, &document)?;
    }
    if document.id == CLASSIC_CONTEXT_MENU_ID {
        classic_path = Some(path);
    }

    // 레거시 파일 삭제
    let legacy_path = directory.join(format!("{LEGACY_CLASSIC_CONTEXT_MENU_ID}.json"));
    match std::fs::remove_file(legacy_path) {
        Ok(()) | Err(_) => {}
    }

    classic_path.ok_or_else(|| "The Classic context menu could not be created".into())
}

pub fn load_context_menu(path: &Path) -> Result<ContextMenuDocument, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let document: ContextMenuDocument =
        serde_json::from_str(&source).map_err(|e| e.to_string())?;
    let errors = document.validate();
    if errors.is_empty() {
        Ok(document)
    } else {
        Err(errors.join("\n"))
    }
}

pub fn list_context_menus() -> Result<Vec<ContextMenuDescriptor>, String> {
    let built_in_path = ensure_builtin_context_menus()?;
    let mut menus = std::fs::read_dir(context_menus_directory())
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        })
        .filter_map(|path| {
            let menu = load_context_menu(&path).ok()?;
            Some(ContextMenuDescriptor {
                built_in: path == built_in_path || menu.is_builtin(),
                path,
                id: menu.id,
                name: menu.name,
            })
        })
        .collect::<Vec<_>>();
    menus.sort_by_key(|m| (!m.built_in, m.name.to_ascii_lowercase()));
    Ok(menus)
}

pub fn resolve_context_menu(reference: Option<&str>) -> Result<ContextMenuDocument, String> {
    let reference = canonical_context_menu_reference(reference.unwrap_or(CLASSIC_CONTEXT_MENU_ID).trim());
    let menus = list_context_menus()?;
    let mut by_id = menus.iter().filter(|m| m.id.eq_ignore_ascii_case(reference));
    if let Some(menu) = by_id.next() {
        return load_context_menu(&menu.path);
    }
    let by_name: Vec<_> = menus
        .iter()
        .filter(|m| m.name.eq_ignore_ascii_case(reference))
        .collect();
    match by_name.as_slice() {
        [menu] => load_context_menu(&menu.path),
        [] => Err(format!("Context menu '{reference}' was not found")),
        _ => Err(format!("Context menu name '{reference}' is ambiguous; use its id")),
    }
}

fn canonical_context_menu_reference(reference: &str) -> &str {
    if reference.eq_ignore_ascii_case(LEGACY_CLASSIC_CONTEXT_MENU_ID) {
        CLASSIC_CONTEXT_MENU_ID
    } else {
        reference
    }
}
