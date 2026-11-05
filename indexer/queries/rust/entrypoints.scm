((function_item name: (identifier) @entry) (#eq? @entry "main"))
((attribute_item (attribute (identifier) @_t)) . (function_item name: (identifier) @entry) (#match? @_t "^(test|bench)$"))
((attribute_item (attribute (scoped_identifier name: (identifier) @_t))) . (function_item name: (identifier) @entry) (#eq? @_t "test"))
