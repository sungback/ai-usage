# Localization

The app ships in Korean only. `locales/ko.toml` is the single source of truth;
the build script reads it and generates the Rust locale registry, locale
matching, and optional Windows font metadata.

## Editing UI text

Add or change keys in `[strings]` and `[translations]` in `locales/ko.toml`, then
run `cargo build`. No Rust changes are needed — `build.rs` generates the
registry, and `LanguageId::Korean` is derived from the file's `code`.

`ko.toml` must stay at `order = 0` with `code = "ko"`: `build.rs` requires the
Korean locale to exist and requires orders to be unique and contiguous from
zero.

The build fails with a focused error when keys are missing or unknown, values
are empty, font metadata is incomplete, or placeholders such as `{name}` and
`{version}` are inconsistent.

## Adding a language back

This is deliberately not supported. The menus expose no language selector, and
`settings.language` is read only to keep pre-existing `settings.json` files
loadable — a stored code other than `ko` falls back to Korean via
`LanguageId::from_code` returning `None`. Reintroducing a second language means
restoring the selector in both native menus (`src/platform/macos/tray.rs` and
`src/context_menu/builtins.rs` via `ContextMenuAction::SetLanguage`), the
`LanguageId::ALL` list, and the per-language menu command mapping in
`src/window.rs`.
