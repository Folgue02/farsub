# WakaTime

Built-in time tracking with [WakaTime](https://wakatime.com) (decided 2026-11-22). It is
one self-contained module (`src/wakatime.rs`) so a plugin can replace it later. It follows
WakaTime's plugin guide (wakatime.com/help/creating-plugin) and uses only what the guide
sanctions: the official `wakatime-cli` sends every heartbeat. farsub never calls
WakaTime's API itself.

## Turning it on

- Off by default. `wakatime = true` in Preferences: Settings turns it on (a project's
  `[settings]` can turn it on or off for that project).
- With it off, the WakaTime commands aren't in the Command Palette and nothing runs.

## The API key

- The key lives in `~/.wakatime.cfg`, under `[settings]` as `api_key = …`. Every WakaTime
  plugin uses this same file, so a key already set there is used with no prompt.
  wakatime-cli reads it there itself. `$WAKATIME_HOME`, when set, replaces the home folder
  (as in wakatime-cli). A `api_key_vault_cmd` there counts as a key.
- **WakaTime: Set API Key…** (Command Palette) asks for the key in a password field (see
  [Text input boxes](inputs.md)). It checks the key's format (a UUID, optionally starting
  with `waka_`, as the official plugins accept) and writes it to `~/.wakatime.cfg`. Only
  `api_key` changes; the rest of the file stays as it was. A file farsub creates is
  readable only by its owner.
- The key never appears on a command line (wakatime-cli is never given `--key`), in the
  log or console, in status messages or on screen.
- With no key, the first heartbeat attempt shows a notice naming the command. farsub does
  not open the prompt by itself.

## Heartbeats

- Sent on the guide's three events: the focused file changes (another tab, another
  group, a file opened), the focused file is edited, and the file is saved.
- A save always sends (with `--write`), and so does a file other than the last one sent.
  Otherwise the same file sends again only once more than 2 minutes have passed since its
  last heartbeat.
- Only files with a path count. Untitled buffers never send.
- wakatime-cli runs in the background: the UI never waits on it. Exit code 104 (the
  server rejected the key) shows a notice. Offline or backing-off codes (102, 112) are
  only logged at debug level: wakatime-cli queues those heartbeats and sends them later.
  Any other failure is logged as a warning.
- Arguments: `--entity <absolute path>`, `--plugin "farsub/<version> farsub-wakatime/<version>"`,
  `--lineno`, `--cursorpos` (1-based column), `--lines-in-file`, `--write` on save, and
  `--is-unsaved-entity` for a new file that isn't on disk yet.
- **Language:** the file's current language in farsub is reported (a language picked in
  the status bar included). It is passed as `--language` (used when WakaTime knows the
  name) and as `--alternate-language` (used when WakaTime can't detect one itself). A few
  titles map to WakaTime's names: Plain Text → Text, JavaScript (JSX) → JSX,
  TypeScript (TSX) → TSX, JSON with Comments → JSON, Protocol Buffers → Protocol Buffer,
  Vim script → VimL. This table lives in the module, not in the language files, so
  removing the module removes everything WakaTime-specific.
- **Project:** `--alternate-project <folder name>` and `--project-folder <folder>` for the
  innermost project folder that holds the file. The guide says never to use `--project`,
  so wakatime-cli's own detection (git and so on) wins.

## wakatime-cli

- Found where the official plugins install it, `~/.wakatime/wakatime-cli-<os>-<arch>` (or
  `~/.wakatime/wakatime-cli`), else on PATH. `<os>` is darwin, linux, windows…; `<arch>`
  is amd64, arm64, 386, arm… (`.exe` on Windows).
- Missing: the first heartbeat attempt shows a notice. Following the
  [principles](principles.md), large programs install only when asked:
  **WakaTime: Install wakatime-cli** downloads the official release
  (`github.com/wakatime/wakatime-cli/releases/latest/download/wakatime-cli-<os>-<arch>.zip`)
  and unzips it into `~/.wakatime/`. Running it again updates to the latest release.
  There's no automatic update check. Like the language server installer, it relies on
  HTTPS and doesn't verify the release's checksums.
- wakatime-cli writes its own log to `~/.wakatime/wakatime.log`.
