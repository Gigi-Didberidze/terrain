(class_declaration name: (identifier) @name) @def.class
(struct_declaration name: (identifier) @name) @def.class
(record_declaration name: (identifier) @name) @def.class
(interface_declaration name: (identifier) @name) @def.type
(enum_declaration name: (identifier) @name) @def.type
(method_declaration name: (identifier) @name) @def.method
(constructor_declaration name: (identifier) @name) @def.method
(namespace_declaration name: (_) @name @namespace) @def.module
(file_scoped_namespace_declaration name: (_) @namespace)
