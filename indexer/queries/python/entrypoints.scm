; if __name__ == "__main__":
((if_statement
   condition: (comparison_operator (identifier) @_name (string)))
 (#eq? @_name "__name__")) @entry.file
; pytest / unittest conventions
((function_definition name: (identifier) @entry) (#match? @entry "^test_"))
((class_definition name: (identifier) @entry) (#match? @entry "^Test"))
; Django management commands and CLI entry points
((function_definition name: (identifier) @entry) (#eq? @entry "main"))
