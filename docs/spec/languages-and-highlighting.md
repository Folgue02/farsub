# Languages and highlighting

- Language: chosen from the languages config (extensions, file names, first-line
  patterns); the status-bar picker (languages by title, plus Plain Text) overrides it per
  file, and highlighting, comment token, indentation defaults and LSP server follow.
- **Highlighting (tree-sitter):** 21 common grammars are compiled in (Bash, C, C++, CSS,
  Go, HTML, INI, Java, JavaScript, JSON, Lua, Make, Markdown, PHP, Python, Ruby, Rust,
  TOML, TSX, TypeScript, XML, YAML); the others are installed at runtime (below). Each buffer keeps a parser and
  syntax tree; every edit is reported to the tree (byte offsets + points) so re-parsing is
  incremental, and a long parse is spread over frames (the UI never waits more than the
  frame budget; highlighting catches up). Only the visible lines are highlighted, by
  running the grammar's highlight query over that byte range. Inner nodes win over outer
  ones; for the same node the earliest query pattern wins.
- Highlight queries are data: `runtime/queries/<grammar>/highlights.scm` (copied from the
  grammar crates, licenses in `runtime/queries/LICENSES.md`), overridable per grammar
  from a package or `~/.config/farsub/queries/<grammar>/highlights.scm`.
- Themes are TOML (`themes/*.toml`): `[editor]` colors (background, foreground,
  selection, line_highlight, gutter, guide, active_guide, caret, find_highlight, and the
  [timer widget](timers.md) colors) and `[syntax]` styles
  per capture (`"#RRGGBB"` or `{ fg, bold, italic, underline }`), looked up from specific to
  general (`function.builtin` → `function`). Markup a theme doesn't style borrows a color
  every theme has (decided 2026-11-18): `markup.raw` (code) uses `string`, `markup.link`
  uses `constant`, `markup.heading` uses `keyword`, so Markdown shows its structure in
  every theme. Highlight queries use the `markup.*` names themes style, not
  nvim-treesitter's old `@text.*` ones (a test checks). Tree-sitter grammars don't
  get language injections yet (Markdown's inline grammar, code in HTML/Svelte/Vue), which
  is why Markdown uses the regex highlighter by default (see below). 29 of the bundled themes were converted once
  from the color schemes farsub used before. "Tsoding" was converted from the MIT-licensed
  [Tsoding Color Theme](https://github.com/JoaoAJMatos/Tsoding-Color-Theme) for VS Code;
  a theme taken from elsewhere carries its source and license notice in a comment at the
  top of its file.
- A language names its grammar with `grammar = "…"` (default: its name); `grammar = ""`
  means no tree-sitter grammar (the regex fallback below still applies). Languages with no
  tree-sitter grammar at all yet: AWK, BibTeX, Clojure, Crystal,
  .env, GraphQL, Just, LaTeX, Meson, Nim, Perl, Terraform, Typst, WGSL (a test keeps this
  list current, checks every recipe has a bundled query, and, for recipes whose source is
  on the machine (cargo registry or `FARSUB_GRAMMAR_SOURCES`), that the query compiles
  against the grammar).
- **Installable grammars** (decided 2026-11-14 to keep the binary small: ~33 MB instead
  of ~100 MB): a recipe `grammars/<name>.toml` (runtime/, the config folder, or a
  package's `grammars/`) names a grammar's source: `crate` + `version` on crates.io, or a
  local `source` folder; optional `path` (where `src/` is) and `symbol` (default
  `tree_sitter_<name>`). "Grammars: Install Grammar…" (Tools menu) or `--install-grammar`
  downloads the crate's source tarball and compiles `src/parser.c` (+ `scanner.c` /
  `scanner.cc`) into a shared library with the system C compiler, like Helix, into
  `<data>/grammars/<name>/`; it's checked to load and be accepted by the parser, then
  open files pick it up without a restart. "Grammars: Remove Grammar…" /
  `--uninstall-grammar` remove one; `--list-grammars` lists all with status.
- **Automatic:** with `auto_install` (default on), opening a file whose grammar is
  installable but missing installs it in the background (once per session per grammar;
  a failure is logged with its reason and the file keeps the regex fallback). Language servers
  the manager can install are installed the same way when a file needs one. With it off,
  farsub says what's missing and the Tools menu installs it.
- **From this computer:** "Grammars: Install Grammar from File…/Folder…" (system file
  picker) or `--install-grammar PATH` compiles a grammar source folder or archive
  (`.zip`, `.tar.gz`, `.tgz`, `.tar`, `.crate`) holding `src/parser.c` (also in a
  subfolder). Its name comes from `tree-sitter.json`, else from the language function in
  `parser.c`; a language whose name (or `grammar`) matches uses it right away. Each install
  writes `<data>/grammars/<name>/grammar.toml` (name, symbol, library, source), so
  installed grammars load with or without a recipe.
- Needs a C compiler (Xcode Command Line Tools, gcc/clang, or Visual Studio Build Tools);
  without one the install fails with that advice. Libraries stay loaded for the session
  (an update loads as a new file, versioned by name).
- An installed grammar uses farsub's bundled query (`runtime/queries/<name>/`), else the
  one from its source. Opening a file whose grammar is installable but missing shows it
  with the regex fallback and says once (status bar + console) how to install it.
- **Regex fallback** (decided 2026-11-14): whenever tree-sitter can't highlight a file (the
  language has no tree-sitter grammar, its grammar isn't installed yet or failed to
  compile, e.g. no C compiler, or its query doesn't load), it's highlighted with a
  Sublime-style regex grammar (syntect) instead of shown as plain text. Once the
  tree-sitter grammar is installed the file switches to it. The fallback grammars are
  two-face's set (bat's, ~220) plus `runtime/syntaxes/*.sublime-syntax` (Gleam, Just, KDL,
  Meson, Prisma), precompiled at build time and loaded only when a file first needs one
  (the release binary grew from 34 to 36 MB). A language's fallback is found by title,
  name, then file extension / file name; `fallback_syntax = "…"` (a grammar name) pins it,
  `""` turns it off. TextMate scopes become theme captures through `scopes.toml` (longest
  scope prefix wins; the innermost scope the theme styles wins; `""` falls through to the
  enclosing scope), overridable from a package or the config folder, so themes need
  nothing extra. Parsing goes top-down with state kept every 64 lines: an edit redoes from
  the edited line, work is capped per frame like tree-sitter's, and lines over 4,000 bytes
  aren't highlighted. A test checks every language without a built-in tree-sitter grammar
  has a fallback.
- **Choosing the highlighter** (decided 2026-11-18): a language file's `highlighter` is
  `"tree-sitter"` (the default: its tree-sitter grammar, the regex one while there's
  none) or `"regex"` (always the regex grammar, even with a tree-sitter one). The
  settings' `highlighter` map overrides it per language without copying the language
  file: `highlighter = { markdown = "tree-sitter" }`; open files switch when it's saved.
  Markdown defaults to `"regex"`: bat's Markdown grammar colors bold, italic, inline
  code, links and fenced code by its language, which tree-sitter would need injections
  for. Nothing else depends on the tree-sitter parse, so regex mode loses nothing but
  its precision. No per-file switch: the setting applies live.
- 37 recipes ship: Batch, C#, CMake, CUDA, Dart, Dockerfile, Elixir, Elm, Erlang, Fish,
  F#, Gleam, GLSL, Groovy, Haskell, Julia, KDL, Kotlin, Less, nginx, Nix, OCaml, Odin, PowerShell, Prisma,
  Protocol Buffers, R, Scala, SCSS, Solidity, SQL, Starlark, Svelte, Swift, Vim script,
  Vue, Zig. Compiling from source also covers grammars whose crates are too old to link
  in (Dockerfile, Svelte, Vue).
- Gradle: `.gradle` files are Groovy and `.gradle.kts` files are Kotlin. Groovy uses the
  `dekobon-tree-sitter-groovy` grammar (decided 2026-11-14: unlike the older
  `tree-sitter-groovy` crate it parses Groovy-specific syntax such as `?.`, `?:`, closures
  and command calls, and ships a highlight query); Kotlin uses `tree-sitter-kotlin` (fwcd,
  the one Helix and Neovim use; `tree-sitter-kotlin-ng` has no highlight query).
- Large files: typing re-parses incrementally (about 7 ms per keystroke in a realistic
  200k-line file); one giant function/block parses slower, capped by the frame budget.
- Built-in languages (`runtime/languages/`, ~75): each gives file extensions / file names
  / shebang detection, line-comment token, indentation defaults, grammar, and servers.
  Invariants (enforced by tests): no two built-in languages claim the same extension or
  file name, every named grammar exists, and every listed
  server exists. Languages whose only comments are block comments (HTML, CSS, XML,
  Markdown, OCaml, Svelte, Vue) have no comment token yet, so Toggle Comment does nothing
  there. Block-comment support is still to do.
