# Licenses and attribution

**Help › Licenses and Attribution** (also in the command palette) opens a read-only
Markdown tab listing everything third-party compiled into the running build, and the
license texts. Running it again goes to the tab already open.

- The page is generated at build time (`build/licenses.rs`) from `cargo metadata` for the
  target being built, so it follows dependency changes by itself and lists each
  platform's crates (a Windows build lists the Windows crates).
- Crates listed: FarSub's normal dependencies, transitively. Proc-macros and what only
  they use run at compile time and aren't listed; nor are dev-dependencies. A
  build-dependency whose data ends up in the binary is listed (two-face).
- License texts come from each crate's LICENSE*/LICENCE*/COPYING*/NOTICE*/UNLICENSE*/
  COPYRIGHT* files, plus named extras (egui's bundled font licenses). Each distinct text
  appears once, numbered and grouped by license, with what it covers; the crate table
  cites those numbers. Full Apache-2.0 copies differing only in their header or appendix
  count as one. A crate shipping no license file is marked "—" (its declared license's
  text appears below).
- For "A OR B" licenses FarSub uses the permissive option: GPL texts from dual-licensed
  crates are left out. `.spdx` metadata files are skipped.
- Bundled data: bat's syntaxes and color schemes (via two-face's acknowledgements; most
  bundled themes were converted from those), and every `runtime/themes/*.toml` whose
  leading comment holds a copyright notice (themes taken from elsewhere keep their source
  and license notice there).
- Bundled fonts: each `runtime/fonts/<name>.ttf` comes with `<name>-LICENSE.txt` (its
  source in the first lines, then the license), listed under that first line's name
  (Codicons, CC BY 4.0).
- Language servers and grammars FarSub downloads at runtime aren't part of the build and
  aren't listed.
