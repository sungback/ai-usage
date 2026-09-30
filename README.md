# AI Usage Monitor

Claude Code, OpenAI Codex, Google Antigravity, OpenCode Go, Cursor의 사용량 한도 및 리셋 주기를 한눈에 모니터링할 수 있는 크로스 플랫폼(macOS 메뉴바 / Windows 작업표시줄) 애플리케이션입니다.

> 일상적인 사용법과 상세 설정에 대한 내용은 [사용자 가이드(USER_GUIDE.md)](USER_GUIDE.md)를 참고하세요.

---

## 주요 기능

- **네이티브 멀티 플랫폼 인터페이스**:
  - **macOS**: 활성화된 공급자마다 원형 링 배지(중앙 세션 숫자, 브랜드 색상, Retina 대응).
  - **Windows**: 작업표시줄 위젯 및 시스템 트레이 빠른 제어, 브랜드별 전용 색상 동심원 링 배지 및 사용량 숫자 지원.
- **실시간 설정 동기화**:
  - 메뉴바/트레이 아이콘을 클릭해 공급자 토글, 사용량 방향을 변경하면 앱 재시작 없이 즉시 반영됩니다.
- **다양한 AI 코딩 어시스턴트 공급자 지원**:
  - **Claude Code (Anthropic)**: 세션(5시간) 및 주간 한도 추적. CLI 및 데스크톱, WSL 자격 증명 자동 감지.
  - **Codex (OpenAI)**: 5시간 쿼터 및 주간 한도 추적.
  - **Google Antigravity (Gemini Code Assist)**: Gemini 쿼터 및 서드파티(Claude/GPT) 쿼터 분리 표시, macOS 키체인 및 Windows 자격 증명 관리자(`gemini:antigravity`) 연동, OAuth2 토큰 자동 갱신.
  - **OpenCode Go**: 워크스페이스 세션 기반 사용량 추적.
  - **Cursor**: 로컬 세션 자동 감지.
- **유연한 사용량 표시 방향**:
  - 남은 사용량 카운트다운(Remaining %) 또는 소진된 사용량(Used %) 모드를 지원하며, 임계치에 따라 색상이 자동 전환됩니다(초록/주황/빨강).
- **자동 업데이트**:
  - GitHub Releases를 확인해 새 버전을 감지하고, 다운로드 binaries의 SHA-256 체크섬을 검증한 뒤에만 설치합니다. 체크섬을 게시하지 않는 릴리스는 **검증 불가로 간주하여 설치를 거부**합니다.
- **프라이버시 최우선**:
  - 외부 분석 도구나 텔레메트리, 중계 서버를 전혀 사용하지 않으며, 로컬에 저장된 자격 증명을 통해 각 공급자의 공식 API와 직접 통신합니다.

---

## 요구 사항

- **macOS**: Apple Silicon (M1/M2/M3/M4 등 ARM64), macOS 11 Big Sur 이상
- **Windows**: Windows 10 또는 Windows 11 (64-bit)
- 지원되는 공급자 중 하나 이상의 도구가 설치되어 있고 로그인되어 있어야 합니다.

> Intel(x86_64) macOS와 범용(universal) 바이너리는 더 이상 제공하지 않습니다.

---

## 설치 방법

[GitHub Releases](https://github.com/sungback/ai-usage/releases)에서 최신 빌드를 다운로드할 수 있습니다:

- **macOS (Apple Silicon)**:
  - `ai-usage-macos-arm64.dmg` 다운로드 후 `Applications` 폴더로 드래그 앤 드롭하여 설치합니다.
  - (대안) `ai-usage-macos-arm64.zip`을 풀어 `AI Usage Monitor.app`를 직접 설치합니다.
- **Windows**:
  - `ai-usage.exe`를 다운로드하여 바로 실행합니다.
  - (대안) `ai-usage.exe.sha256`로 체크섬을 먼저 검증한 뒤 실행합니다.

릴리스 자산에는 무결성 검증을 위한 `.sha256` 체크섬 파일이 함께 게시됩니다.

> **macOS 첫 실행 시 열기가 거부될 수 있습니다 — 한 번만 승인하면 됩니다.**
>
> 이 앱은 **Apple Developer ID로 서명·공증(notarization)되지 않은 빌드**입니다. macOS 11
> (Big Sur) 이상은 App Store 밖에서 내려받은 앱에 Gatekeeper 검사를 적용하므로, `dmg`/`zip`
> 을 내려받아 처음 열면 "개발자를 확인할 수 없습니다" 계열의 경고와 함께 실행이 막힙니다.
> 설치 자체가 실패한 것이 아니므로 아래 중 한 경로로 **한 번만** 승인하십시오. 승인 후에는
> 이후 실행마다 경고가 나오지 않습니다.
>
> 1. **Finder 우클릭 → 열기**: `AI Usage Monitor.app`를 마우스 오른쪽 버튼으로 눌러
>    **열기**를 선택하고, 경고 대화상자에서 **열기**를 다시 누릅니다.
> 2. **시스템 설정 경유**: 위 방법이 동작하지 않으면 **시스템 설정 → 개인정보 보호 및 보안**
>    화면 하단에 승인 버튼("열기" / "그래도 열기" / "Open Anyway" — macOS 언어 설정에 따라
>    문구가 다릅니다)이 나타납니다. 이를 누른 뒤 앱을 다시 실행하십시오.
> 3. **터미널로 격리 해제**(선택):
>    ```bash
>    xattr -dr com.apple.quarantine "/Applications/AI Usage Monitor.app"
>    ```
>
> 또한 **시스템 설정 → 개인정보 보호 및 보안 → Gatekeeper 보호 강화를 잠시 끄는 방식은
> 권장하지 않습니다.** 전체 다운로드 경로의 보안이 함께 약해집니다. 위 1~2번으로
> 해결하십시오.

---

## 사용 방법

### 앱 실행
- **macOS**: `AI Usage Monitor.app`을 실행하면 메뉴바 상단에 위젯이 표시됩니다.
- **Windows**: `ai-usage.exe`를 실행하면 시스템 트레이 및 작업표시줄에 배지가 표시됩니다.

### 설정 변경하기
별도 설정 창 없이 **메뉴바/트레이 아이콘을 클릭해 나타나는 메뉴**에서 모두 조정합니다.
변경 사항은 앱을 재시작하지 않고 즉시 반영됩니다.

- **macOS**: 메뉴바 아이콘을 **클릭**하면 메뉴가 열립니다.
- **Windows**: 트레이/작업표시줄 아이콘을 **왼쪽 클릭 또는 우클릭**하면 컨텍스트 메뉴가 열립니다.

메뉴에서 조정할 수 있는 항목:
- **사용량 새로고침**: 즉시 최신 사용량을 다시 조회합니다.
- **Update frequency** (Windows): 1분 / 5분 / 15분 / 1시간.
- **Providers**: 각 AI 공급자 활성화/비활성화, 표시 순서.
- **Settings**: 사용량 표시 방향(남은 양 / 사용량), 주간(7일) 안쪽 링 표시, 링 배지 표시(Windows), 시작 시 자동 실행.
- **업데이트 확인**: 새 버전을 확인하고 설치합니다 (업데이트가 있으면 버전과 함께 안내).

메뉴 맨 아래에는 실행 중인 버전(`AI Usage Monitor vX.Y.Z`)이 표시됩니다.

> 표시 언어는 한국어 단독입니다. 메뉴에 언어 선택 항목이 없습니다.

---

## 공급자별 설정 안내

| 공급자 | 설정 방법 |
| --- | --- |
| **Claude Code** | Claude Code CLI 또는 데스크톱 앱에서 로그인합니다. Windows, macOS 및 WSL 자격 증명이 자동으로 감지됩니다. |
| **Codex** | Codex CLI를 설치하고 로그인한 뒤, 메뉴의 **Providers** 항목에서 활성화합니다. |
| **Google Antigravity** | Antigravity에 로그인합니다. macOS 키체인 및 Windows 자격 증명 관리자(`gemini:antigravity`)의 토큰을 자동으로 감지하며 OAuth2 갱신을 지원합니다. |
| **OpenCode Go** | OpenCode Go 계정을 연결하고 아래의 인증 쿠키를 설정한 후 활성화합니다. |
| **Cursor** | Cursor에 로그인하면 로컬 세션이 자동으로 감지됩니다. 필요 시 `CURSOR_SESSION_TOKEN` 환경 변수로 재지정할 수 있습니다. |

### OpenCode Go 상세 설정
`OPENCODE_GO_WORKSPACE_ID` 및 `OPENCODE_GO_AUTH_COOKIE` 환경 변수를 설정하거나, 설정 디렉터리에 `config.json`을 생성합니다:

```json
{
  "workspaceId": "wrk_01...",
  "authCookie": "__Host-console_session=your-session-cookie-value"
}
```

---

## 소스 코드에서 빌드하기

### 사전 준비
- [Rust](https://www.rust-lang.org/tools/install) **1.95 이상** (`Cargo.toml`의 `rust-version` 기준)

**별도 설정 없이 빌드됩니다.** Antigravity 폴러가 토큰 갱신에 쓰는 Google OAuth 클라이언트는
Antigravity 앱 자신의 공개 "installed application" 클라이언트이며, 실행 시 Antigravity가 이미
저장해 둔 `id_token`에서 클라이언트 ID를 읽어 결정합니다. 기본값이 코드에 포함되어 있어
별도 환경변수 설정이 필요 없습니다.

### macOS (Apple Silicon ARM64) 빌드
```bash
./scripts/build-macos.sh
```
빌드가 완료되면 `target/AI Usage Monitor.app`, `target/ai-usage-macos-arm64.zip`, `target/ai-usage-macos-arm64.dmg`가 생성됩니다.

### Windows 빌드
```powershell
cargo build --release
```
빌드된 실행 파일은 `target/release/ai-usage.exe`에 생성됩니다.

### 테스트
```bash
cargo test
```

---

## 프로젝트 구조

```
src/
├── main.rs                   엔트리포인트, 패닉 훅, CLI 인자 처리
│
├── platform/                 플랫폼 추상화 (macOS / Windows / generic unix)
│   ├── mod.rs                  플랫폼별 run() 진입점 디스패치
│   ├── macos/                  macOS 메뉴바 구현
│   ├── ring_badge.rs           원형 링 배지 래스터라이저 (Retina 대응)
│   └── unix.rs                 유닉스 공통 스텁
│
├── window.rs                 Windows 네이티브 윈도우 / 메시지 루프 / 작업표시줄 위젯
│
├── poller/                   공급자별 사용량 폴러
│   ├── accounts.rs             계정·자격 증명 공통 로직
│   ├── claude.rs               Claude Code (CLI / WSL) — 5시간 세션 + 주간 한도
│   ├── claude_desktop.rs       Claude Desktop 자격 증명 감지
│   ├── antigravity.rs          Google Antigravity — OAuth2 자동 갱신, Gemini·서드파티 쿼터 분리
│   ├── codex.rs                OpenAI Codex — 5시간 쿼터 + 주간 한도
│   ├── cursor.rs               Cursor — 로컬 세션 자동 감지
│   ├── opencode.rs             OpenCode Go — 워크스페이스 세션 기반
│   └── tests.rs                폴러 통합 테스트
│
├── theme_engine/             테마 시스템
│   ├── theme_rendering.rs      렌더링 엔진 (사용량 % → 색상·크기 계산)
│   ├── theme_expression.rs     JSON 테마 표현식 평가기
│   ├── theme_datetime.rs       날짜·시간 처리
│   ├── theme_storage.rs        테마 파일 로드·저장
│   └── tests.rs                테마 엔진 테스트
│
├── context_menu/             컨텍스트 메뉴 (Windows 전용)
│   ├── mod.rs                  메뉴 빌더·이벤트 처리
│   ├── model.rs                메뉴 데이터 모델
│   ├── builtins.rs             내장 메뉴 항목 (공급자 토글, 설정 등)
│   └── io.rs                   메뉴 상태 영속화
│
├── localization/             다국어 지원
│   ├── mod.rs                  로케일 로더
│   └── locales/ko.toml         한국어 문자열 리소스
│
├── native_interop/           네이티브 FFI
│   ├── win32.rs                Win32 API 래퍼
│   └── tests.rs                FFI 바인딩 테스트
│
├── themes/                   내장 JSON 테마 파일
│   ├── classic-usage-widget.json
│   └── compact-fluent-quad.json
│
├── app_settings.rs           설정 저장·불러오기 (settings.json)
├── models.rs                 핵심 데이터 모델 (공급자 상태, 사용량 등)
├── providers.rs              공급자 목록·메타데이터 정의
├── updater.rs                GitHub Releases 자동 업데이트 (SHA-256 체크섬 검증)
├── tray_icon.rs              Windows 시스템 트레이 아이콘
├── desktop_compositor.rs     Windows Desktop Composition (DirectComposition)
├── theme.rs                  테마 전환 로직
├── winsqlite.rs              Windows 내장 SQLite3 바인딩
└── app.manifest              Windows 애플리케이션 매니페스트 (DPI 인식 등)
```

---

## 문제 해결

- **설정 파일 위치**:
  - macOS: `~/Library/Application Support/ai-usage/settings.json`
  - Windows: `%APPDATA%\ai-usage\settings.json`
- **설정 파일 복구**: 설정 파일이 손상되어 파싱에 실패하면 원본은 `settings.json.corrupt-<타임스탬프>`로 보존되고, 앱은 기본값으로 시작합니다.

---

## 라이선스

[MIT License](LICENSE)

누구나 자유롭게 사용, 복제, 수정, 병합, 게시, 배포, 판매할 수 있으며
상업적 이용도 가능합니다. 전체 조건은 [`LICENSE`](LICENSE) 파일을 참고하십시오.
