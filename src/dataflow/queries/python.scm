; Dataflow binding spec for Python (tree-sitter-python). Capture vocabulary:
; see src/dataflow/mod.rs. Python has merged_vars scoping: no block scopes, a
; name written anywhere in a function is local to it (the engine creates the
; variable implicitly), and all bindings of a name in one scope are the same
; variable. `match` patterns and `del` are deliberately not modelled — their
; identifiers degrade to reads, which can only mask findings, never invent them.

; Every identifier is a read unless re-classified by a def/assign capture.
(identifier) @ref

; Not variable uses: attribute names after a dot, keyword-argument names.
(attribute attribute: (identifier) @ignore)
(keyword_argument name: (identifier) @ignore)

; Scopes. Comprehensions and lambdas genuinely are nested function scopes in
; Python 3; class bodies are opaque (their bindings are externally visible).
(function_definition) @scope.function
(lambda) @scope.function
(list_comprehension) @scope.function
(set_comprehension) @scope.function
(dictionary_comprehension) @scope.function
(generator_expression) @scope.function
(class_definition body: (block) @scope.opaque)

; Loops and branches. Boolean operators count as branches because of
; short-circuit evaluation — over-approximating conditionality only suppresses.
(for_statement) @loop
(while_statement) @loop
; Branches are captured per *arm*, not per construct: two writes in different
; arms of one if/elif/else, try/except, or match must not look equally
; conditional to each other.
(if_statement consequence: (_) @branch)
(elif_clause) @branch
(else_clause) @branch
(try_statement body: (_) @branch)
(except_clause) @branch
(finally_clause) @branch
(case_clause) @branch
(conditional_expression) @branch
(boolean_operator) @branch

; Bindings that resolve names but are never flagged themselves.
(function_definition name: (identifier) @def.hoist)
(class_definition name: (identifier) @def.pattern)
(parameters) @def.param
(lambda_parameters) @def.param
(import_statement) @def.pattern
(import_from_statement) @def.pattern
(as_pattern alias: (as_pattern_target) @def.pattern)

; Writes. A simple assignment target is the flaggable kind; the @anchor makes
; the write take effect after its right-hand side, so `x = x + 1` reads first.
(assignment
  left: (identifier) @assign
  right: (_) @anchor)
(assignment
  left: [(pattern_list) (tuple_pattern) (list_pattern)] @assign.multi)
(augmented_assignment
  left: (identifier) @assign.update
  right: (_) @anchor)
; Loop targets and walrus targets: writes, never flagged.
(for_statement left: (_) @assign.multi)
(for_in_clause left: (_) @assign.multi)
(named_expression name: (identifier) @assign.multi)

; global/nonlocal: never flag this name in this function.
(global_statement (identifier) @escape)
(nonlocal_statement (identifier) @escape)
