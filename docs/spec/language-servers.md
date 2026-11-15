# Language servers

- nvim-lspconfig-style server files (`servers/*.toml`) and languages listing servers in
  preference order; the first installed (by farsub, or on PATH) is used.
- Mason-like manager (Tools → Language Servers): install via npm / pip venv / cargo / go /
  GitHub release / script into the data folder; status, logs, update, uninstall. Placeholders
  `${install_dir}`, `${root}`, `${home}` in server args/env/settings/initialization options.
- Built-in servers (~45). Installable from the manager on macOS, Linux and Windows:
  - npm (needs Node.js): pyright, basedpyright, ts_ls (`typescript@5`, since TypeScript
    7 has no `tsserver`), bashls, yamlls, jsonls / cssls / html_ls
    (vscode-langservers-extracted), taplo, dockerls, intelephense, svelte, elmls, vimls,
    prismals.
  - pip (needs Python): pylsp, cmake_ls. go (needs Go): gopls.
  - GitHub release binaries: rust_analyzer, clangd, lua_ls, marksman, zls, texlab,
    tinymist, clojure_lsp.
  - On PATH only (installed with the language's own toolchain; the description says how):
    ruby_lsp, solargraph, jdtls, kotlin_language_server, sourcekit_lsp, csharp_ls, ols,
    nimlangserver, hls, ocamllsp, elixir_ls, erlang_ls, metals, dartls, gleam, nil_ls,
    nixd, terraformls, r_language_server.
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
