//! 테마 파일 보관함 — 처음 보시는 분을 위한 안내.
//!
//! - 테마는 설정 폴더 아래 `themes`에, 그림 파일은 `themes/assets`에 둡니다.
//! - `assets/` 밖을 가리키는 그림 경로는 테스트에서 걸러냅니다.

use super::*;

pub fn themes_directory() -> PathBuf {
    crate::app_settings::app_data_directory().join("themes")
}


#[cfg(test)]
pub(super) fn managed_asset_file_name(path: &str) -> Option<&str> {
    let normalized = path.strip_prefix("assets/")?;
    (!normalized.is_empty()
        && !normalized.contains('/')
        && !normalized.contains('\\')
        && Path::new(normalized)
            .file_name()
            .and_then(|name| name.to_str())
            == Some(normalized))
    .then_some(normalized)
}
#[cfg(test)]
pub fn remove_asset_references(theme: &mut ThemeDocument, path: &str) -> usize {
    let mut removed = 0;
    for surface in &mut theme.surfaces {
        if matches!(
            &surface.background,
            LayerBackground::Image { path: image, .. } if image == path
        ) {
            surface.background = LayerBackground::None;
            removed += 1;
        }
        for object in &mut surface.children {
            if matches!(
                &object.background,
                LayerBackground::Image { path: image, .. } if image == path
            ) {
                object.background = LayerBackground::None;
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
pub fn theme_asset_usage(theme: &ThemeDocument, path: &str) -> usize {
    theme
        .surfaces
        .iter()
        .map(|surface| {
            usize::from(matches!(
                &surface.background,
                LayerBackground::Image { path: image, .. } if image == path
            )) + surface
                .children
                .iter()
                .filter(|object| {
                    matches!(
                        &object.background,
                        LayerBackground::Image { path: image, .. } if image == path
                    )
                })
                .count()
        })
        .sum()
}

pub fn save_theme(theme: &ThemeDocument) -> Result<PathBuf, String> {
    let mut theme = theme.clone();
    theme.prepare_runtime();
    if theme.is_builtin() {
        return Err(format!(
            "{} is read-only; duplicate it to make changes",
            theme.name
        ));
    }
    let errors = theme.validate();
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    let directory = themes_directory();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{}.json", safe_file_stem(&theme.id)));
    crate::app_settings::write_json_atomic(&path, &theme)?;
    Ok(path)
}

#[cfg(test)]
pub fn is_managed_theme_path(path: &Path) -> bool {
    let directory = themes_directory();
    path.parent().is_some_and(|parent| {
        parent == directory
            || std::fs::canonicalize(parent)
                .ok()
                .zip(std::fs::canonicalize(&directory).ok())
                .is_some_and(|(parent, directory)| parent == directory)
    })
}

#[cfg(test)]
pub fn delete_theme(path: &Path) -> Result<(), String> {
    if !is_managed_theme_path(path) {
        return Err("Only themes saved by the monitor can be deleted".into());
    }
    let directory = std::fs::canonicalize(themes_directory()).map_err(|error| error.to_string())?;
    let path = std::fs::canonicalize(path).map_err(|error| error.to_string())?;
    if path.parent() != Some(directory.as_path()) {
        return Err("Theme path is outside the managed themes folder".into());
    }
    let theme = load_theme(&path)?;
    if theme.is_builtin() {
        return Err(format!("{} is built-in and cannot be deleted", theme.name));
    }
    std::fs::remove_file(path).map_err(|error| error.to_string())
}

pub fn load_theme(path: &Path) -> Result<ThemeDocument, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut theme: ThemeDocument = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    theme.prepare_runtime();
    let errors = theme.validate();
    if errors.is_empty() {
        Ok(theme)
    } else {
        Err(errors.join("\n"))
    }
}

pub fn ensure_starter_theme() -> Result<PathBuf, String> {
    let directory = themes_directory();
    for (expected_id, source) in BUILTIN_THEME_SOURCES {
        let mut theme: ThemeDocument = serde_json::from_str(source)
            .map_err(|error| format!("Built-in theme '{expected_id}' is invalid JSON: {error}"))?;
        if theme.id != *expected_id {
            return Err(format!(
                "Built-in theme id '{}' does not match '{expected_id}'",
                theme.id
            ));
        }
        theme.prepare_runtime();
        let errors = theme.validate();
        if !errors.is_empty() {
            return Err(format!(
                "Built-in theme '{}' is invalid:\n{}",
                theme.name,
                errors.join("\n")
            ));
        }
        let path = directory.join(format!("{expected_id}.json"));
        let canonical = serde_json::to_vec_pretty(&theme).map_err(|error| error.to_string())?;
        let current = std::fs::read(&path).ok();
        if current.as_deref() != Some(canonical.as_slice()) {
            crate::app_settings::write_json_atomic(&path, &theme)?;
        }
    }
    Ok(directory.join(format!("{CLASSIC_THEME_ID}.json")))
}
