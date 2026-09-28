# AI Usage Monitor

Claude Code, OpenAI Codex, Google Antigravity, OpenCode Go, Cursor의 사용량 한도 및 리셋 주기를 한눈에 모니터링할 수 있는 크로스 플랫폼(macOS 메뉴바 / Windows 작업표시줄) 애플리케이션입니다.

> 일상적인 사용법과 상세 설정에 대한 내용은 [사용자 가이드(USER_GUIDE.md)](USER_GUIDE.md)를 참고하세요.

---

## 주요 기능

- **네이티브 멀티 플랫폼 인터페이스**:
  - **macOS**: 고해상도(Retina) 2줄 메뉴바 배지, 브랜드별 공식 화이트 엠블럼, 24px 대형 폰트, 실시간 다중 모델 컬럼 지원.
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
- **Refresh now**: 즉시 최신 사용량을 다시 조회합니다.
- **Update frequency** (Windows): 1분 / 5분 / 15분 / 1시간.
- **Providers**: 각 AI 공급자 활성화/비활성화, 표시 순서.
- **Settings**: 사용량 표시 방향(남은 양 / 사용량), 주간(7일) 안쪽 링 표시, 링 배지 표시(Windows), 시작 시 자동 실행, 업데이트 확인.

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
저장해 둔 `id_token`에서 클라이언트 ID를 읽어 결정합니다. 기본값도 코드에 포함되어 있어
`.env` 파일이나 환경변수가 필요 없습니다.

기계별로 다른 값을 쓰고 싶을 때만 `.env`를 생성해 두면 우선 적용됩니다 (`.gitignore`에 등록됨):

```env
ANTIGRAVITY_CLIENT_ID=your_client_id
ANTIGRAVITY_CLIENT_SECRET=your_client_secret
```

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
├── main.rs              엔트리포인트 및 CLI 인자 처리
├── window.rs            Windows 네이티브 윈도우/메시 루프/트레이
├── platform/            플랫폼 추상화 (macOS / Windows / generic unix)
├── theme_engine/        테마 모델, 표현식 평가기, 렌더링
├── poller/              공급자별 사용량 폴러 (Claude/Codex/Antigravity/...)
├── context_menu/        컨텍스트 메뉴 모델·빌더·영속화
├── localization/        한국어 로케일 (TOML)
├── native_interop/      Win32 FFI, SQLite 바인딩
└── updater.rs           GitHub Releases 업데이트 (체크섬 검증)
```

---

## 문제 해결 및 진단 (Diagnostics)

문제가 발생할 경우 진단 모드로 실행하여 로그를 확인할 수 있습니다:

```bash
# macOS
./ai-usage --diagnose

# Windows
.\ai-usage.exe --diagnose
```

- **로그 파일 위치**: OS 임시 디렉터리의 `ai-usage.log`
  - macOS: `$TMPDIR/ai-usage.log` (기본 `~/Library/Caches/TemporaryItems/`)
  - Windows: `%TEMP%\ai-usage.log`
- **설정 파일 위치**:
  - macOS: `~/Library/Application Support/ai-usage/settings.json`
  - Windows: `%APPDATA%\ai-usage\settings.json`
- **설정 파일 복구**: 설정 파일이 손상되어 파싱에 실패하면 원본은 `settings.json.corrupt-<타임스탬프>`로 보존되고, 앱은 기본값으로 시작합니다. 이 경우 로그에 경고가 남습니다.
- **설정 디렉터리 이전**: 이전 판의 폴더 이름 `MyAIMonitor`를 쓰고 있었다면 `ai-usage`로 옮긴 뒤 재시작하십시오. 설정과 계정 정보는 자동 이관되지 않습니다. macOS 자동 실행(LaunchAgent) 라벨도 `com.sungback.aiusage`로 바뀌므로 메뉴에서 다시 켜 주십시오.

---

## 라이선스

[MIT License](LICENSE)

누구나 자유롭게 사용, 복제, 수정, 병합, 게시, 배포, 판매할 수 있으며
상업적 이용도 가능합니다. 전체 조건은 [`LICENSE`](LICENSE) 파일을 참고하십시오.
