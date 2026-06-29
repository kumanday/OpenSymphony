; Captures Rust definition nodes and their names for symbol summaries.
; function_signature_item covers trait method declarations without bodies.

(function_item) @definition.function

(function_signature_item) @definition.method

(struct_item) @definition.struct

(enum_item) @definition.enum

(trait_item) @definition.trait
