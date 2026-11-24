# Highlight query sources

The `highlights.scm` files here are copied from the tree-sitter grammar crates FarSub
bundles, under each crate's license:

| Crate | Version | License |
| --- | --- | --- |
| dekobon-tree-sitter-groovy | 0.6.0 | Apache-2.0 OR MIT |
| tree-sitter-bash | 0.25.1 | MIT |
| tree-sitter-batch | 0.11.1 | MIT |
| tree-sitter-c | 0.24.2 | MIT |
| tree-sitter-c-sharp | 0.23.5 | MIT |
| tree-sitter-cmake | 0.7.6 | MIT |
| tree-sitter-cpp | 0.23.4 | MIT |
| tree-sitter-css | 0.25.0 | MIT |
| tree-sitter-cuda | 0.21.2 | MIT |
| tree-sitter-dart | 0.3.0 | MIT |
| tree-sitter-elixir | 0.3.5 | Apache-2.0 |
| tree-sitter-elm | 5.9.4 | MIT |
| tree-sitter-erlang | 0.21.0 | MIT |
| tree-sitter-fish | 3.6.0 | MIT |
| tree-sitter-fsharp | 0.3.12 | MIT |
| tree-sitter-gleam | 1.0.0 | Apache-2.0 |
| tree-sitter-glsl | 0.2.0 | MIT |
| tree-sitter-go | 0.25.0 | MIT |
| tree-sitter-haskell | 0.24.1 | MIT |
| tree-sitter-html | 0.23.2 | MIT |
| tree-sitter-ini | 1.4.0 | Apache-2.0 |
| tree-sitter-java | 0.23.5 | MIT |
| tree-sitter-javascript | 0.25.0 | MIT |
| tree-sitter-json | 0.24.8 | MIT |
| tree-sitter-julia | 0.23.1 | MIT |
| tree-sitter-kdl | 2.0.0 | MIT |
| tree-sitter-kotlin | 0.3.8 | MIT (query based on nvim-treesitter, Apache-2.0) |
| tree-sitter-less | 1.0.0 | MIT |
| tree-sitter-lua | 0.5.0 | MIT |
| tree-sitter-make | 1.1.1 | MIT |
| tree-sitter-md | 0.5.4 | MIT |
| tree-sitter-nginx | 1.0.1 | MIT |
| tree-sitter-nix | 0.3.0 | MIT |
| tree-sitter-ocaml | 0.26.0 | MIT |
| tree-sitter-odin | 1.3.0 | MIT |
| tree-sitter-php | 0.25.1 | MIT |
| tree-sitter-powershell | 0.26.6 | MIT |
| tree-sitter-prisma | 0.2.2 | MIT |
| tree-sitter-proto | 0.6.0 | MIT |
| tree-sitter-python | 0.25.0 | MIT |
| tree-sitter-r | 1.3.0 | MIT |
| tree-sitter-ruby | 0.23.1 | MIT |
| tree-sitter-rust | 0.24.2 | MIT |
| tree-sitter-scala | 0.26.13 | MIT |
| tree-sitter-scss | 1.0.0 | MIT |
| tree-sitter-sequel | 0.3.13 | MIT |
| tree-sitter-solidity | 1.2.13 | MIT |
| tree-sitter-starlark | 1.3.0 | MIT |
| tree-sitter-swift | 0.7.4 | MIT |
| tree-sitter-toml-ng | 0.7.0 | MIT |
| tree-sitter-typescript | 0.23.2 | MIT |
| tree-sitter-vim | 0.4.0 | MIT |
| tree-sitter-xml | 0.7.0 | MIT |
| tree-sitter-yaml | 0.7.2 | MIT |
| tree-sitter-zig | 1.1.2 | MIT |
| tree-sitter-dockerfile | 0.2.0 | MIT |
| tree-sitter-svelte | 0.10.2 | MIT |
| tree-sitter-vue | 0.0.3 | MIT |

Changes: `typescript` and `tsx` leave out tree-sitter-javascript's `highlights-params.scm`
(TypeScript wraps parameters differently and has its own parameter patterns); one
`solidity` pattern is rewritten for tree-sitter 0.27 (marked in the file).
