; Dataflow binding spec for TypeScript / TSX (tree-sitter-typescript; also used
; for plain JS — the TS grammar parses it). Capture vocabulary: see
; src/dataflow/mod.rs. Anything not classified here degrades to a read (@ref),
; which can only mask findings, never invent them.

; Every identifier is a read unless re-classified by a def/assign capture.
; Shorthand object properties ({ x }) read the variable too.
(identifier) @ref
(shorthand_property_identifier) @ref

; Scopes. Functions (all flavors) are the units of analysis; class bodies are
; opaque (field initializers run "elsewhere", members are externally visible).
(function_declaration) @scope.function
(function_expression) @scope.function
(generator_function_declaration) @scope.function
(generator_function) @scope.function
(arrow_function) @scope.function
(method_definition) @scope.function
(class_body) @scope.opaque
(statement_block) @scope
(for_statement) @scope
(for_in_statement) @scope

; Loops and branches. Binary expressions count as branches because of
; short-circuit operators — over-approximating conditionality only suppresses.
(for_statement) @loop
(for_in_statement) @loop
(while_statement) @loop
(do_statement) @loop
; Branches are captured per *arm*, not per construct: two writes in different
; arms of one if/else, switch, try/catch, or ternary must not look equally
; conditional to each other.
(if_statement consequence: (_) @branch)
(if_statement alternative: (else_clause) @branch)
(switch_case) @branch
(switch_default) @branch
(try_statement body: (_) @branch)
(catch_clause) @branch
(finally_clause) @branch
(ternary_expression consequence: (_) @branch)
(ternary_expression alternative: (_) @branch)
(binary_expression) @branch

; Bindings that resolve names but are never flagged themselves.
(function_declaration name: (identifier) @def.hoist)
(generator_function_declaration name: (identifier) @def.hoist)
(enum_declaration name: (identifier) @def.hoist)
(function_expression name: (identifier) @def.pattern)
(formal_parameters) @def.param
(arrow_function parameter: (identifier) @def.param)
(import_statement) @def.pattern

; Declarations with a value are the flaggable kind; the @anchor makes the
; binding visible only after its initializer.
(variable_declarator
  name: (identifier) @def
  value: (_) @anchor)
(variable_declarator
  name: (identifier) @def.bare
  !value)
; Destructuring declarations: extracted bindings, never flagged.
(variable_declarator
  name: [(object_pattern) (array_pattern)] @def.pattern)
(catch_clause
  parameter: (_) @def.pattern
  body: (_) @def.pattern.scope)
; for-of / for-in targets (declared or not): per-iteration bindings.
(for_in_statement left: (_) @def.pattern)

; Writes. Compound assignment and ++/-- read before they write.
(assignment_expression
  left: (identifier) @assign
  right: (_) @anchor)
(assignment_expression
  left: [(object_pattern) (array_pattern)] @assign.multi)
(augmented_assignment_expression
  left: (identifier) @assign.update
  right: (_) @anchor)
(update_expression argument: (identifier) @assign.update)
