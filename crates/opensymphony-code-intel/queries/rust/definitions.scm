; Captures Rust definition nodes and their names for symbol summaries.
; function_signature_item covers trait method declarations without bodies.

(function_item
  name: (identifier) @name) @definition

(function_signature_item
  name: (identifier) @name) @definition

(struct_item
  name: (type_identifier) @name) @definition

(enum_item
  name: (type_identifier) @name) @definition

(trait_item
  name: (type_identifier) @name) @definition
