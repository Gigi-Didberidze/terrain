(call_expression function: (identifier) @ref.call)
(call_expression function: (field_expression field: (field_identifier) @ref.call))
(call_expression function: (qualified_identifier name: (identifier) @ref.call))
(call_expression function: (template_function name: (identifier) @ref.call))
(call_expression function: (qualified_identifier name: (template_function name: (identifier) @ref.call)))
(new_expression type: (type_identifier) @ref.call)
(base_class_clause (type_identifier) @ref.inherits)
(base_class_clause (qualified_identifier name: (type_identifier) @ref.inherits))
(type_identifier) @ref.type
