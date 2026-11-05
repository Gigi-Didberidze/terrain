(use_declaration argument: (_) @import)
; `mod foo;` pulls in foo.rs or foo/mod.rs.
(mod_item name: (identifier) @import.mod !body)
(extern_crate_declaration name: (identifier) @import)
