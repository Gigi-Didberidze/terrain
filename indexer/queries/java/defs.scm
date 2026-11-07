(class_declaration name: (identifier) @name) @def.class
(record_declaration name: (identifier) @name) @def.class
(enum_declaration name: (identifier) @name) @def.class
(interface_declaration name: (identifier) @name) @def.type
(annotation_type_declaration name: (identifier) @name) @def.type
(method_declaration name: (identifier) @name) @def.method
(constructor_declaration name: (identifier) @name) @def.method
(package_declaration (scoped_identifier) @namespace)
(package_declaration (identifier) @namespace)
