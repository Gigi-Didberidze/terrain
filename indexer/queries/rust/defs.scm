(function_item name: (identifier) @name) @def.function
(function_signature_item name: (identifier) @name) @def.function
(struct_item name: (type_identifier) @name) @def.class
(enum_item name: (type_identifier) @name) @def.class
(union_item name: (type_identifier) @name) @def.class
(trait_item name: (type_identifier) @name) @def.type
(type_item name: (type_identifier) @name) @def.type
; impl blocks group their methods under the type name.
(impl_item type: (type_identifier) @name) @def.class
(impl_item type: (generic_type type: (type_identifier) @name)) @def.class
(impl_item type: (scoped_type_identifier name: (type_identifier) @name)) @def.class
(mod_item name: (identifier) @name body: (declaration_list)) @def.module
(const_item name: (identifier) @name) @def.constant
(static_item name: (identifier) @name) @def.constant
(macro_definition name: (identifier) @name) @def.function
