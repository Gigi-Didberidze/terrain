; Test files: describe/it/test at any level.
((call_expression function: (identifier) @_f) (#match? @_f "^(describe|suite|it|test)$")) @entry.file
; HTTP route handlers: app.get("/x", handler)
((call_expression
   function: (member_expression property: (property_identifier) @_m)
   arguments: (arguments (string) (identifier) @entry.name))
 (#match? @_m "^(get|post|put|patch|delete|all|use|route)$"))
; VS Code extensions and CLIs
((function_declaration name: (identifier) @entry) (#match? @entry "^(activate|deactivate|main)$"))
((variable_declarator name: (identifier) @entry) (#match? @entry "^(activate|deactivate|main)$"))
