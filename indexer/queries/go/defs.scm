(function_declaration name: (identifier) @name) @def.function
(method_declaration name: (field_identifier) @name) @def.method
(type_spec name: (type_identifier) @name type: (struct_type)) @def.class
(type_spec name: (type_identifier) @name type: (interface_type)) @def.type
(type_spec name: (type_identifier) @name) @def.type
(type_alias name: (type_identifier) @name) @def.type
(source_file (const_declaration (const_spec name: (identifier) @name) @def.constant))
(source_file (var_declaration (var_spec name: (identifier) @name) @def.constant))
(package_clause (package_identifier) @namespace)
