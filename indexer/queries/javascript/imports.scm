(import_statement source: (string (string_fragment) @import))
(import_statement
  (import_clause (named_imports (import_specifier name: (identifier) @import.name)))
  source: (string (string_fragment) @import))
(export_statement source: (string (string_fragment) @import))
((call_expression
   function: (identifier) @_req
   arguments: (arguments . (string (string_fragment) @import)))
 (#eq? @_req "require"))
(call_expression function: (import) arguments: (arguments . (string (string_fragment) @import)))
