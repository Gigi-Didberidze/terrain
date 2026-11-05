((function_declaration name: (identifier) @entry) (#match? @entry "^(main|init)$"))
((function_declaration name: (identifier) @entry) (#match? @entry "^(Test|Benchmark|Fuzz|Example)"))
((package_clause (package_identifier) @_p) (#eq? @_p "main")) @entry.file
; net/http handlers: http.HandleFunc("/x", handler)
((call_expression
   function: (selector_expression field: (field_identifier) @_f)
   arguments: (argument_list (interpreted_string_literal) (identifier) @entry.name))
 (#match? @_f "^(HandleFunc|Handle|GET|POST|PUT|DELETE|PATCH)$"))
