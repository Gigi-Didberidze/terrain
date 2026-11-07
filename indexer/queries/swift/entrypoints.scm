((function_declaration name: (simple_identifier) @entry) (#match? @entry "^test"))
; @main types and SwiftUI apps.
((class_declaration (modifiers (attribute (user_type (type_identifier) @_a))) name: (type_identifier) @entry) (#eq? @_a "main"))
