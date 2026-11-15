# Language servers

- nvim-lspconfig-style server files (`servers/*.toml`) and languages listing servers in
  preference order; the first installed (by farsub, or on PATH) is used.
- Mason-like manager (Tools → Language Servers): install via npm / pip venv / cargo / go /
  GitHub release / download URL / install commands / script into the data folder; status,
  logs, update, uninstall. Placeholders `${install_dir}`, `${root}`, `${root_id}` (a short
  per-project id), `${home}` in server args/env/settings/initialization options.
- **Goal: as few manual installs as possible** (decided 2026-11-15). Built-in servers
  (~46), installable from the manager (and automatically with `auto_install`) on macOS,
  Linux and Windows:
  - npm (needs Node.js): pyright, basedpyright, ts_ls (`typescript@5`, since TypeScript
    7 has no `tsserver`), bashls, yamlls, jsonls / cssls / html_ls
    (vscode-langservers-extracted), taplo, dockerls, intelephense, svelte, elmls, vimls,
    prismals.
  - pip (needs Python): pylsp, cmake_ls. go (needs Go): gopls.
  - GitHub release binaries: rust_analyzer, clangd, lua_ls, marksman, zls, texlab,
    tinymist, clojure_lsp, gleam, ols, nimlangserver (needs Nim), elixir_ls (needs
    Elixir), kotlin_language_server (needs Java; fwcd's, which fails to start on Java 22+).
  - Direct downloads: jdtls (eclipse.org's latest build, needs Java 21+), terraformls
    (releases.hashicorp.com), kotlin_lsp (JetBrains' Kotlin server, from its VS Code
    package; brings its own Java runtime, ~370 MB; Kotlin's first choice).
  - Install commands: metals (Coursier, downloaded first; needs Java), ruby_lsp and
    solargraph (`gem`, into farsub's folder; need Ruby), csharp_ls (`dotnet tool`; needs
    the .NET SDK), r_language_server (the R package into farsub's folder; needs R).
  - On PATH only, because they're tied to a toolchain you already have: sourcekit_lsp
    (Xcode / Swift), dartls (Dart / Flutter SDK), hls (ghcup, matches your GHC), ocamllsp
    (opam, matches your switch), erlang_ls (builds per Erlang/OTP version), nil_ls and
    nixd (Nix). Their descriptions say how to get them.
- Recipe parts (in `[install]`), besides the sources: `needs = ["java"]` + `needs_hint`
  (checked before downloading: "`java` not found — install a JDK 21 or newer…"),
  `bin_windows` (when the executable isn't `<bin>.exe`, e.g. a `.bat`), `[install.url]
  assets` (download URLs keyed like the GitHub assets; `all` fits every platform; `.zip`,
  `.vsix`, `.tar.gz`, `.tar.xz`, `.gz`), `[[install.run]]` commands run without a shell (so
  they work the same on Windows) with `{dir}` / `{dir/}` for the install folder, and
  `[install.wrapper]`: farsub writes a launcher (`launch-<bin>`, a `.cmd` on Windows) that
  runs a program with fixed arguments and environment (`{dir}`, `{os}` = mac/linux/win,
  `{arm}`, `*` globs resolved at install time, `{root_id}` filled in per project when the
  server starts). Java servers and jdtls start through one, so no Python is needed.
- Finding programs on Windows: commands are looked up with the executable extensions
  (`PATHEXT`: `.exe`, `.cmd`, `.bat`…), so `npm`, `java` or a server installed by hand
  are found; also in `%APPDATA%\npm`, Scoop's shims, `~/.dotnet/tools`, and (everywhere)
  mise / asdf shims.
- Root markers may be globs (`*.csproj`, `*.cabal`): a directory matches when any of its
  entries matches.
- Any server with a source that works on this OS/CPU can be installed from the manager. A
  recipe with nothing for this platform shows as "not installed" (no install action), and
  installing it explains why.
- GitHub release source (`[install.github]`, like Mason's): `repo`, optional `tag` (latest
  by default), and `assets` globs keyed `"<os>-<arch>"` or `"<os>"` (os: macos, linux,
  windows; arch: x86_64, aarch64). The download, `.zip` / `.tar.gz` / `.tar.xz` / `.gz` extraction and
  executable lookup all happen inside farsub, so they don't depend on `curl`, `tar` or `sh`
  and work the same on Windows. `GITHUB_TOKEN` is used if set (API rate limit).
- LSP Info (Cmd+Alt+I / status bar): this file's language/syntax, configured servers and
  their status, attached server details (command, root, pid, sync, capabilities, stderr),
  all running servers; `i` install, `r` restart, `s` manager.
- A server that fails `initialize` is shown as failed with its reason (never stuck on
  "starting").
