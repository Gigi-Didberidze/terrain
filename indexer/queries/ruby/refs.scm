; Method calls, except Ruby's own keywords-as-methods (require, include, attr_*, visibility).
((call method: (identifier) @ref.call)
 (#not-match? @ref.call "^(require|require_relative|load|autoload|include|extend|prepend|attr_accessor|attr_reader|attr_writer|private|protected|public|module_function|raise)$"))
(call receiver: (constant) @ref.call method: (identifier) @_new (#eq? @_new "new"))
(superclass (constant) @ref.inherits)
(superclass (scope_resolution name: (constant) @ref.inherits))
; Mixins behave like inheritance.
((call method: (identifier) @_m arguments: (argument_list (constant) @ref.inherits)) (#match? @_m "^(include|extend|prepend)$"))
(scope_resolution name: (constant) @ref.type)
