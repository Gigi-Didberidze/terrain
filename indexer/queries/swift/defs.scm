(class_declaration name: (type_identifier) @name) @def.class
; extension Foo { ... } joins Foo when the file defines it (like a Rust impl block).
(class_declaration name: (user_type (type_identifier) @name)) @def.impl
(protocol_declaration name: (type_identifier) @name) @def.type
(typealias_declaration name: (type_identifier) @name) @def.type
(function_declaration name: (simple_identifier) @name) @def.function
(protocol_function_declaration name: (simple_identifier) @name) @def.method
(init_declaration "init" @name) @def.method
