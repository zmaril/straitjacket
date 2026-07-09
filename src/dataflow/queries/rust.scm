; Dataflow binding spec for Rust (tree-sitter-rust).
; Capture vocabulary: see src/dataflow/mod.rs. Anything not classified here
; degrades to a read (@ref), which can only mask findings, never invent them.

; Every identifier is a read unless re-classified by a def/assign capture.
(identifier) @ref

; Not variable uses: attribute contents, macro names, enum variant names.
(attribute_item) @ignore
(inner_attribute_item) @ignore
(macro_invocation macro: (identifier) @ignore)
(enum_variant name: (identifier) @ignore)

; Scopes. Functions and closures are the units of analysis.
(function_item) @scope.function
(closure_expression) @scope.function
(async_block) @scope.function
(block) @scope

; Loops and branches. Binary expressions count as branches because of
; short-circuit operators — over-approximating conditionality only suppresses.
(for_expression) @loop
(while_expression) @loop
(loop_expression) @loop
; Branches are captured per *arm*, not per construct: two writes in different
; arms of one if/match must not look equally conditional to each other.
(if_expression consequence: (_) @branch)
(if_expression alternative: (_) @branch)
(match_arm) @branch
(binary_expression) @branch

; Bindings that resolve names but are never flagged themselves.
(function_item name: (identifier) @def.hoist)
(parameters) @def.param
(closure_parameters) @def.param
(const_item name: (identifier) @def.pattern)
(static_item name: (identifier) @def.pattern)
(mod_item name: (identifier) @def.pattern)
(use_declaration) @def.pattern

; let with a value is the flaggable kind; the @anchor makes the binding
; visible only after its initializer, so `let x = x + 1` reads the outer x.
(let_declaration
  pattern: (identifier) @def
  value: (_) @anchor)
(let_declaration
  pattern: (identifier) @def.bare
  !value)
; Destructuring / refutable patterns: extracted bindings, never flagged.
(let_declaration pattern: (_) @def.pattern)

; Pattern bindings scoped to the region they cover.
(match_arm pattern: (match_pattern) @def.pattern) @def.pattern.scope
(if_expression
  condition: (let_condition pattern: (_) @def.pattern)
  consequence: (block) @def.pattern.scope)
(while_expression
  condition: (let_condition pattern: (_) @def.pattern)
  body: (block) @def.pattern.scope)
(for_expression
  pattern: (_) @def.pattern
  body: (block) @def.pattern.scope)

; Writes. Compound assignment reads before it writes.
(assignment_expression
  left: (identifier) @assign
  right: (_) @anchor)
(compound_assignment_expr
  left: (identifier) @assign.update
  right: (_) @anchor)

; Format-style macro strings: "{name}" interpolations count as reads.
(token_tree (string_literal) @string.interp)
(token_tree (raw_string_literal) @string.interp)
