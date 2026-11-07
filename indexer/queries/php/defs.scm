(class_declaration name: (name) @name) @def.class
(interface_declaration name: (name) @name) @def.type
(trait_declaration name: (name) @name) @def.type
(enum_declaration name: (name) @name) @def.type
(function_definition name: (name) @name) @def.function
(method_declaration name: (name) @name) @def.method
(const_declaration (const_element (name) @name)) @def.constant
(namespace_definition name: (namespace_name) @namespace)
