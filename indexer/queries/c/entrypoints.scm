((function_definition declarator: (function_declarator declarator: (identifier) @entry)) (#eq? @entry "main"))
; Kernel-style registration: module_init(fn), late_initcall(fn), EXPORT_SYMBOL(fn), ...
((call_expression
   function: (identifier) @_m
   arguments: (argument_list (identifier) @entry.name))
 (#match? @_m "^(module_init|module_exit|[a-z_]*initcall|module_[a-z_]*_driver|EXPORT_SYMBOL|EXPORT_SYMBOL_GPL)$"))
