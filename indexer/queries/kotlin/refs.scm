; f(x) and receiver.f(x): the callee is the first child, or the last identifier of a navigation.
(call_expression . (identifier) @ref.call)
(call_expression . (navigation_expression (identifier) @ref.call .))
; class A : B(), I: a constructor call is the superclass, a bare type an interface.
(delegation_specifier (constructor_invocation (user_type (identifier) @ref.inherits)))
(delegation_specifier (constructor_invocation (type (user_type (identifier) @ref.inherits))))
(delegation_specifier (user_type (identifier) @ref.implements))
(delegation_specifier (type (user_type (identifier) @ref.implements)))
(user_type (identifier) @ref.type)
