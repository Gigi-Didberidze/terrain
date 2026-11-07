((function_definition declarator: (function_declarator declarator: (identifier) @entry)) (#eq? @entry "main"))
; GoogleTest and Catch2 test macros make the file a test root.
((call_expression function: (identifier) @_t) (#match? @_t "^(TEST|TEST_F|TEST_P|TYPED_TEST|TEST_CASE|SCENARIO)$")) @entry.file
((function_definition declarator: (function_declarator declarator: (identifier) @_t)) (#match? @_t "^(TEST|TEST_F|TEST_P|TYPED_TEST|TEST_CASE)$")) @entry.file
