((method_declaration name: (name) @entry) (#match? @entry "^test"))
; Laravel-style controller actions referenced from routes: Route::get('/x', 'handler').
((scoped_call_expression scope: (name) @_r name: (name) @_m arguments: (arguments (argument (string)) (argument (string (string_content) @entry.name))))
 (#eq? @_r "Route") (#match? @_m "^(get|post|put|patch|delete|any)$"))
