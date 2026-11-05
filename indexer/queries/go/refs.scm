(call_expression function: (identifier) @ref.call)
(call_expression function: (selector_expression field: (field_identifier) @ref.call))
(composite_literal type: (type_identifier) @ref.call)
(composite_literal type: (qualified_type name: (type_identifier) @ref.call))
(type_identifier) @ref.type
(qualified_type name: (type_identifier) @ref.type)
; Embedded interfaces and structs act as inheritance.
(interface_type (type_elem (type_identifier) @ref.inherits))
(field_declaration_list (field_declaration . type: (type_identifier) @ref.inherits .))
