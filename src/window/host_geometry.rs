//! 모니터·작업표시줄 지리 정보 — 처음 보시는 분을 위한 안내.
//!
//! - 시작할 때와 화면 배치가 바뀔 때 모니터·작업표시줄 자리를 미리 재 둡니다.
//! - 값만 들고 있고 네이티브 핸들은 안 들고 있어 읽는 쪽은 Win32 호출이 없습니다.

use super::*;

#[derive(Clone, Copy)]
struct ThemeHostGeometry {
    monitor: RECT,
    taskbar: Option<RECT>,
    scale: f64,
}

// 읽는 쪽에서 Win32를 호출할 필요가 없도록 네이티브 핸들 대신 값을 저장합니다.
// 읽는 쪽은 STATE를 보유할 수 있으며, 쓰는 쪽은 이 락을 보유한 상태에서 STATE를 획득해서는 안 됩니다.
static THEME_HOST_GEOMETRY: Mutex<Vec<ThemeHostGeometry>> = Mutex::new(Vec::new());
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// 시작할 때 및 셸/디스플레이 레이아웃이 변경될 때 STATE가 잠기지 않은 상태에서 호출됩니다.
pub(super) fn refresh_theme_host_geometry() {
    refresh_with(|| {
        let displays = native_interop::find_monitors();
        let taskbars = native_interop::find_taskbars();
        displays
            .into_iter()
            .map(|display| ThemeHostGeometry {
                monitor: display.rect,
                taskbar: taskbars
                    .iter()
                    .find(|taskbar| unsafe {
                        MonitorFromWindow(taskbar.hwnd, MONITOR_DEFAULTTOPRIMARY) == display.handle
                    })
                    .map(|taskbar| taskbar.rect),
                scale: monitor_scale(display),
            })
            .collect()
    });
}

fn refresh_with(query: impl FnOnce() -> Vec<ThemeHostGeometry>) {
    // 레이아웃 쿼리는 반환하기 전에 다른 레이아웃 알림을 전달(디스패치)할 수 있습니다.
    // 재진입한 읽는 쪽은 마지막으로 완료된 스냅샷을 계속 사용하며, 여기서 대기하지 않습니다.
    if REFRESHING.swap(true, Ordering::Acquire) {
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REFRESHING.store(false, Ordering::Release);
        }
    }
    let _reset = Reset;
    let geometry = query();
    *THEME_HOST_GEOMETRY
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = geometry;
}

/// STATE 잠금 상태에서도 안전합니다: 크기 조정, 히트 테스트 및 메뉴 평가는 캐시된 값만 사용합니다.
pub(super) fn theme_runtime_for_surface(
    theme: &ThemeDocument,
    surface_index: usize,
    runtime: ThemeRuntime,
) -> ThemeRuntime {
    let geometry = THEME_HOST_GEOMETRY
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    runtime_with_geometry(theme, surface_index, runtime, &geometry)
}

pub(super) fn taskbar_is_horizontal(display_index: usize) -> bool {
    THEME_HOST_GEOMETRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(display_index)
        .and_then(|host| host.taskbar)
        .is_none_or(native_interop::is_taskbar_horizontal)
}

fn runtime_with_geometry(
    theme: &ThemeDocument,
    surface_index: usize,
    runtime: ThemeRuntime,
    geometry: &[ThemeHostGeometry],
) -> ThemeRuntime {
    let Some(surface) = theme.surfaces.get(surface_index) else {
        return runtime;
    };
    let nest = surface
        .placement
        .nest
        .resolve(surface.placement.reference.region);
    let runtime = runtime.with_nest(nest);
    let Some(host) = geometry
        .get(surface.placement.reference.display)
        .or_else(|| geometry.first())
    else {
        return runtime;
    };
    let rect = if matches!(nest, SurfaceNest::Taskbar | SurfaceNest::TrayIcon) {
        host.taskbar.unwrap_or(host.monitor)
    } else {
        host.monitor
    };
    runtime.with_host_dimensions(
        logical_host_dimension(rect.right - rect.left, host.scale),
        logical_host_dimension(rect.bottom - rect.top, host.scale),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display() -> ThemeHostGeometry {
        ThemeHostGeometry {
            monitor: RECT {
                left: 0,
                top: 0,
                right: 3840,
                bottom: 2160,
            },
            taskbar: Some(RECT {
                left: 0,
                top: 2068,
                right: 3840,
                bottom: 2160,
            }),
            scale: 2.0,
        }
    }

    #[test]
    fn cached_hosts_preserve_nesting_and_dpi() {
        let mut theme = ThemeDocument::starter();
        let runtime = ThemeRuntime::default().with_poll_state(false, true);
        for (nest, height) in [
            (SurfaceNest::Taskbar, 46),
            (SurfaceNest::TrayIcon, 46),
            (SurfaceNest::Floating, 1080),
            (SurfaceNest::Desktop, 1080),
        ] {
            theme.surfaces[0].placement.nest = nest;
            assert_eq!(
                runtime_with_geometry(&theme, 0, runtime, &[display()]),
                runtime.with_nest(nest).with_host_dimensions(1920, height)
            );
        }
    }

    #[test]
    fn selected_monitor_uses_its_own_scale_and_vertical_taskbar() {
        let mut theme = ThemeDocument::starter();
        theme.surfaces[0].placement.nest = SurfaceNest::TrayIcon;
        theme.surfaces[0].placement.reference.display = 1;
        let mut secondary = display();
        secondary.scale = 1.0;
        secondary.taskbar = Some(RECT {
            left: -1920,
            top: 0,
            right: -1860,
            bottom: 1080,
        });
        let runtime = ThemeRuntime::default();
        assert_eq!(
            runtime_with_geometry(&theme, 0, runtime, &[display(), secondary]),
            runtime
                .with_nest(SurfaceNest::TrayIcon)
                .with_host_dimensions(60, 1080)
        );
    }

    #[test]
    fn missing_display_and_taskbar_keep_existing_fallbacks() {
        let mut theme = ThemeDocument::starter();
        theme.surfaces[0].placement.nest = SurfaceNest::TrayIcon;
        theme.surfaces[0].placement.reference.display = 99;
        let runtime = ThemeRuntime::default();
        let mut host = display();
        host.taskbar = None;
        assert_eq!(
            runtime_with_geometry(&theme, 0, runtime, &[host]),
            runtime
                .with_nest(SurfaceNest::TrayIcon)
                .with_host_dimensions(1920, 1080)
        );
        assert_eq!(
            runtime_with_geometry(&theme, 0, runtime, &[]),
            runtime.with_nest(SurfaceNest::TrayIcon)
        );
        assert_eq!(runtime_with_geometry(&theme, 99, runtime, &[host]), runtime);
    }

    #[test]
    fn shell_query_can_reenter_state_and_read_previous_geometry() {
        let theme = ThemeDocument::starter();
        let runtime = ThemeRuntime::default();
        refresh_with(|| vec![display()]);
        let previous = theme_runtime_for_surface(&theme, 0, runtime);
        refresh_with(|| {
            // 셸 왕복 통신 중에 전달된 전송 메시지를 시뮬레이션합니다.
            let _state = lock_state();
            assert_eq!(theme_runtime_for_surface(&theme, 0, runtime), previous);
            refresh_with(|| panic!("a reentrant layout message must not query the shell"));
            let mut host = display();
            host.scale = 1.0;
            vec![host]
        });
        assert_eq!(
            theme_runtime_for_surface(&theme, 0, runtime),
            runtime.with_host_dimensions(3840, 92)
        );
    }
}
