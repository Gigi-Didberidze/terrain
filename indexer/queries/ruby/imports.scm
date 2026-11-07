((call method: (identifier) @_r arguments: (argument_list (string (string_content) @import))) (#match? @_r "^(require|load|autoload)$"))
((call method: (identifier) @_r arguments: (argument_list (string (string_content) @import.rel))) (#eq? @_r "require_relative"))
