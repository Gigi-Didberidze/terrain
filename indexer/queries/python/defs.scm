; Definitions. @def.<kind> on the node, @name on its name.
(function_definition name: (identifier) @name) @def.function
(class_definition name: (identifier) @name) @def.class
; Module-level UPPER_CASE assignments are constants.
(module
  (expression_statement
    (assignment left: (identifier) @name) @def.constant)
  (#match? @name "^[A-Z][A-Z0-9_]*$"))
