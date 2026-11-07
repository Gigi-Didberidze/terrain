; Definitions only (with bodies), not header prototypes: calls land on implementations.
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @def.function
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @def.function
(function_definition declarator: (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name)))) @def.function
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class
(union_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @def.type
(type_definition declarator: (type_identifier) @name) @def.type
; Function-like macros act as functions. Object-like defines are skipped: large C trees have
; millions of them (DECISIONS.md D25).
(preproc_function_def name: (identifier) @name) @def.function
