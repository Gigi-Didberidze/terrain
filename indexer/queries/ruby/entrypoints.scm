; RSpec / minitest files are test roots.
((call method: (identifier) @_m) (#match? @_m "^(describe|context|it|test|RSpec)$")) @entry.file
((method name: (identifier) @entry) (#match? @entry "^test_"))
