; Definitions only (with bodies), not header prototypes: calls land on implementations.
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @def.function
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @def.function
(function_definition declarator: (reference_declarator (function_declarator declarator: (identifier) @name))) @def.function
(class_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class
(union_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @def.type
(type_definition declarator: (type_identifier) @name) @def.type
(alias_declaration name: (type_identifier) @name) @def.type
(namespace_definition name: (namespace_identifier) @name) @def.module
; Methods defined in the class body.
(field_declaration_list (function_definition declarator: (function_declarator declarator: (field_identifier) @name)) @def.method)
; Out-of-class definitions: Foo::bar() joins Foo when the file defines it.
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier scope: (namespace_identifier) @owner name: (identifier) @name))) @def.method
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier scope: (namespace_identifier) @owner name: (destructor_name) @name))) @def.method
; ns::Foo::bar(): the owner is the innermost scope.
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (qualified_identifier scope: (namespace_identifier) @owner name: (identifier) @name)))) @def.method
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (qualified_identifier
        name: (qualified_identifier scope: (namespace_identifier) @owner name: (identifier) @name))))) @def.method
; Function-like macros act as functions; object-like defines are skipped (D25).
(preproc_function_def name: (identifier) @name) @def.function
