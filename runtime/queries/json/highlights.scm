;; From tree-sitter-json 0.24.8: queries/highlights.scm
(string) @string

;; FarSub: after `(string)`, so it wins (the last matching pattern wins).
(pair
  key: (_) @string.special.key)

(number) @number

[
  (null)
  (true)
  (false)
] @constant.builtin

(escape_sequence) @escape

(comment) @comment
