(function_declaration name: (identifier) @name) @def.function
(generator_function_declaration name: (identifier) @name) @def.function
(class_declaration name: (identifier) @name) @def.class
(class name: (identifier) @name) @def.class
(method_definition name: (property_identifier) @name) @def.method
(field_definition property: (property_identifier) @name value: [(arrow_function) (function_expression)]) @def.method
; const f = () => {} / function () {}
(variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @def.function
; Top-level constants: literal-like values, or UPPER_CASE names. (Aliases such as
; `const fs = require("fs")` are imports, not constants.)
(program (lexical_declaration (variable_declarator name: (identifier) @name value: [(object) (array) (string) (number) (template_string) (true) (false) (regex)]) @def.constant))
(program (export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name value: [(object) (array) (string) (number) (template_string) (true) (false) (regex)]) @def.constant)))
((program (lexical_declaration (variable_declarator name: (identifier) @name) @def.constant)) (#match? @name "^[A-Z][A-Z0-9_]*$"))
((program (export_statement declaration: (lexical_declaration (variable_declarator name: (identifier) @name) @def.constant))) (#match? @name "^[A-Z][A-Z0-9_]*$"))
