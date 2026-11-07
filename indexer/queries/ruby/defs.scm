(method name: (_) @name) @def.method
(singleton_method name: (_) @name) @def.method
(class name: (constant) @name) @def.class
(class name: (scope_resolution name: (constant) @name)) @def.class
(module name: (constant) @name) @def.module
(module name: (scope_resolution name: (constant) @name)) @def.module
(assignment left: (constant) @name) @def.constant
