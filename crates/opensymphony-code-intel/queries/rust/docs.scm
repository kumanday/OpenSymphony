((line_comment) @doc.comment
  (#match? @doc.comment "^///|^//!"))

((block_comment) @doc.comment
  (#match? @doc.comment "^/\\*\\*|^/\\*!"))
