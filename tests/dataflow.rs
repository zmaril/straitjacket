//! Behavioural tests for the experimental tree-sitter dataflow analysis
//! (`unused-assignment`), driven through the real `Engine` the same way the
//! CLI uses it. True positives per language, plus the tricky negatives the
//! prototype must not flag: shadowing that is genuinely read, match/if-let
//! bindings, augmented assignment, destructuring, closures capturing
//! variables, loop-carried values, and conditional overwrites.

use straitjacket::{Config, Engine};

fn engine() -> Engine {
    Engine::new(&Config {
        dataflow: true,
        ..Config::default()
    })
    .expect("rules compile")
}

/// (line, variable) pairs of unused-assignment findings for a snippet.
fn hits(src: &str, ext: &str) -> Vec<(usize, String)> {
    engine()
        .scan_text(src, "test", ext)
        .into_iter()
        .filter(|f| f.rule == "unused-assignment")
        .map(|f| (f.line, f.matched))
        .collect()
}

fn assert_clean(src: &str, ext: &str) {
    let h = hits(src, ext);
    assert!(h.is_empty(), "expected no findings, got {h:?}\nin:\n{src}");
}

// ---- default-off ---------------------------------------------------------

#[test]
fn dataflow_is_off_by_default() {
    let e = Engine::new(&Config::default()).expect("rules compile");
    let src = "fn f() -> u32 {\n    let x = 5;\n    let x = 2;\n    x\n}\n";
    let flagged: Vec<_> = e
        .scan_text(src, "test", "rs")
        .into_iter()
        .filter(|f| f.rule == "unused-assignment")
        .collect();
    assert!(flagged.is_empty());
    assert!(!e.rule_ids().contains(&"unused-assignment"));
}

#[test]
fn skip_disables_the_rule() {
    let mut e = engine();
    assert!(e.rule_ids().contains(&"unused-assignment"));
    let unknown = e.skip(&["unused-assignment".to_string()]);
    assert!(unknown.is_empty());
    let src = "fn f() -> u32 {\n    let x = 5;\n    let x = 2;\n    x\n}\n";
    assert!(e.scan_text(src, "test", "rs").is_empty());
}

#[test]
fn allow_markers_suppress() {
    let src = "fn f() -> u32 {\n    let x = 5; // straitjacket-allow:unused-assignment\n    let x = 2;\n    x\n}\n";
    assert_clean(src, "rs");
    let src = "// straitjacket-allow-file:unused-assignment\nfn f() -> u32 {\n    let x = 5;\n    let x = 2;\n    x\n}\n";
    assert_clean(src, "rs");
}

// ---- Rust ------------------------------------------------------------------

#[test]
fn rust_flags_shadowed_binding_never_read() {
    let src = "fn f() -> u32 {\n    let x = 5;\n    let x = 2;\n    x\n}\n";
    assert_eq!(hits(src, "rs"), vec![(2, "x".to_string())]);
}

#[test]
fn rust_flags_dead_store_before_reassignment() {
    let src = "fn f() -> u32 {\n    let mut x = compute();\n    x = 2;\n    x\n}\nfn compute() -> u32 { 1 }\n";
    assert_eq!(hits(src, "rs"), vec![(2, "x".to_string())]);
}

#[test]
fn rust_shadowing_that_reads_the_outer_binding_is_fine() {
    // `let x = x + 1` reads the previous x — the anchor makes the RHS resolve
    // to the outer binding.
    let src = "fn f() -> u32 {\n    let x = 5;\n    let x = x + 1;\n    x\n}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_underscore_names_are_skipped() {
    let src = "fn f() {\n    let _unused = compute();\n}\nfn compute() -> u32 { 1 }\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_module_scope_is_never_flagged() {
    let src = "const X: u32 = 5;\nstatic Y: u32 = 6;\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_if_let_binding_is_not_flagged_and_does_not_steal_reads() {
    // The if-let `x` is scoped to the consequence block, so the `x` in the
    // else branch still reads the outer binding.
    let src = "fn f(o: Option<u32>) -> u32 {\n    let x = 1;\n    if let Some(x) = o { g(x) } else { g(x) }\n}\nfn g(v: u32) -> u32 { v }\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_match_arm_bindings_stay_in_their_arm() {
    // `None => x` reads the outer x; the Some-arm binding must not steal it.
    let src = "fn f(o: Option<u32>) -> u32 {\n    let x = h();\n    match o { Some(x) => g(x), None => x }\n}\nfn g(v: u32) -> u32 { v }\nfn h() -> u32 { 2 }\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_loop_carried_value_is_not_flagged() {
    // `prev = v` is read on the next iteration by `g(prev)`.
    let src = "fn f(items: Vec<u32>) {\n    let mut prev = 0;\n    for v in items {\n        g(prev);\n        prev = v;\n    }\n}\nfn g(v: u32) {}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_dead_store_inside_loop_is_flagged() {
    let src = "fn f(items: Vec<u32>) {\n    for v in items {\n        let q = v * 2;\n    }\n}\n";
    assert_eq!(hits(src, "rs"), vec![(3, "q".to_string())]);
}

#[test]
fn rust_closure_capture_counts_as_a_read() {
    let src = "fn f() -> u32 {\n    let x = 5;\n    let get = move || x;\n    get()\n}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_write_from_closure_masks_the_variable() {
    let src =
        "fn f() -> u32 {\n    let mut n = 0;\n    let mut inc = || n += 1;\n    inc();\n    n\n}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_conditional_overwrite_is_not_a_kill() {
    // On the `!c` path the initial value IS read.
    let src =
        "fn f(c: bool) -> u32 {\n    let mut x = 1;\n    if c {\n        x = 2;\n    }\n    x\n}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_unconditional_overwrite_after_conditional_write_is_flagged() {
    let src = "fn f(c: bool) {\n    let mut x = 1;\n    if c {\n        x = 2;\n    }\n    x = 3;\n    g(x);\n}\nfn g(v: u32) {}\n";
    // x = 2 is dead: never read before the unconditional x = 3.
    assert_eq!(hits(src, "rs"), vec![(4, "x".to_string())]);
}

#[test]
fn rust_writes_in_both_arms_do_not_kill_each_other() {
    // x = 1 (then-arm) and x = 2 (else-arm) are alternatives, not a
    // reassignment chain — neither is dead.
    let src = "fn f(c: bool) -> u32 {\n    let mut x = 0;\n    g(x);\n    if c {\n        x = 1;\n    } else {\n        x = 2;\n    }\n    x\n}\nfn g(v: u32) {}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_match_guard_counts_as_a_read() {
    let src = "fn f(v: Option<u32>) -> u32 {\n    let is_big = size() > 3;\n    match v {\n        Some(n) if is_big => n,\n        _ => 0,\n    }\n}\nfn size() -> u32 { 1 }\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_format_string_interpolation_is_a_read() {
    let src = "fn f() {\n    let count = 3;\n    println!(\"{count}\");\n    let total = 4;\n    println!(\"{total:>8}\");\n}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_compound_assignment_reads_first() {
    let src = "fn f() -> u32 {\n    let mut acc = 0;\n    acc += 1;\n    acc\n}\n";
    assert_clean(src, "rs");
}

#[test]
fn rust_tuple_destructuring_is_not_flagged() {
    let src =
        "fn f() -> u32 {\n    let (a, b) = pair();\n    a\n}\nfn pair() -> (u32, u32) { (1, 2) }\n";
    assert_clean(src, "rs");
}

// ---- Python ----------------------------------------------------------------

#[test]
fn python_flags_dead_store_before_reassignment() {
    let src = "def f():\n    x = 1\n    x = 2\n    return x\n";
    assert_eq!(hits(src, "py"), vec![(2, "x".to_string())]);
}

#[test]
fn python_flags_trailing_dead_store() {
    let src = "def f():\n    r = compute()\n    return None\n";
    assert_eq!(hits(src, "py"), vec![(2, "r".to_string())]);
}

#[test]
fn python_augmented_assignment_is_a_read() {
    let src = "def f():\n    x = 1\n    x += 1\n    return x\n";
    assert_clean(src, "py");
}

#[test]
fn python_tuple_unpacking_is_not_flagged() {
    let src = "def f(p):\n    a, b = p\n    return a\n";
    assert_clean(src, "py");
}

#[test]
fn python_global_names_are_never_flagged() {
    let src = "def f():\n    global counter\n    counter = 5\n";
    assert_clean(src, "py");
}

#[test]
fn python_module_scope_is_never_flagged() {
    let src = "x = 1\nx = 2\n";
    assert_clean(src, "py");
}

#[test]
fn python_loop_carried_value_is_not_flagged() {
    let src =
        "def f(xs):\n    prev = None\n    for x in xs:\n        print(prev)\n        prev = x\n";
    assert_clean(src, "py");
}

#[test]
fn python_accumulator_via_rhs_read_is_not_flagged() {
    let src = "def f(xs):\n    out = 0\n    for x in xs:\n        out = out + x\n    return out\n";
    assert_clean(src, "py");
}

#[test]
fn python_lambda_capture_counts_as_a_read() {
    let src = "def f():\n    x = 1\n    g = lambda: x + 1\n    return g\n";
    assert_clean(src, "py");
}

#[test]
fn python_comprehension_reads_count() {
    let src = "def f(xs):\n    factor = 2\n    ys = [q * factor for q in xs]\n    return ys\n";
    assert_clean(src, "py");
}

#[test]
fn python_fstring_interpolation_is_a_read() {
    let src = "def f():\n    a = 1\n    return f\"{a}\"\n";
    assert_clean(src, "py");
}

#[test]
fn python_conditional_overwrite_is_not_a_kill() {
    let src = "def f(c):\n    x = 1\n    if c:\n        x = 2\n    return x\n";
    assert_clean(src, "py");
}

#[test]
fn python_writes_in_both_arms_do_not_kill_each_other() {
    let src = "def f(c):\n    if c:\n        x = 1\n    else:\n        x = 2\n    return x\n";
    assert_clean(src, "py");
}

#[test]
fn python_try_and_except_writes_do_not_kill_each_other() {
    let src = "def f():\n    ok = probe()\n    try:\n        ok = risky()\n    except ValueError:\n        ok = False\n    return ok\n";
    assert_clean(src, "py");
}

// ---- TypeScript / TSX --------------------------------------------------------

#[test]
fn ts_flags_dead_store_before_reassignment() {
    let src = "function f(): number {\n  let x = 1;\n  x = 2;\n  return x;\n}\n";
    assert_eq!(hits(src, "ts"), vec![(2, "x".to_string())]);
}

#[test]
fn ts_flags_never_read_const() {
    let src = "function f(): number {\n  const x = compute();\n  return 5;\n}\n";
    assert_eq!(hits(src, "ts"), vec![(2, "x".to_string())]);
}

#[test]
fn ts_bare_let_then_assign_then_read_is_fine() {
    let src = "function f(c: boolean): number {\n  let u;\n  u = 5;\n  return u;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_destructuring_is_not_flagged() {
    let src = "function f(p: { a: number; b: number }): number {\n  const { a, b } = p;\n  return a;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_closure_capture_counts_as_a_read() {
    let src = "function f(): () => number {\n  const x = 5;\n  return () => x;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_write_from_closure_masks_the_variable() {
    let src = "function f(): number {\n  let n = 0;\n  const inc = () => { n += 1; };\n  inc();\n  return n;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_loop_counter_is_not_flagged() {
    let src = "function f(): void {\n  for (let i = 0; i < 3; i++) {\n    use(i);\n  }\n}\ndeclare function use(v: number): void;\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_loop_carried_value_is_not_flagged() {
    let src = "function f(xs: number[]): void {\n  let prev = 0;\n  for (const x of xs) {\n    log(prev);\n    prev = x;\n  }\n}\ndeclare function log(v: number): void;\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_conditional_overwrite_is_not_a_kill() {
    let src = "function f(c: boolean): number {\n  let x = 1;\n  if (c) {\n    x = 2;\n  }\n  return x;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_writes_in_both_arms_do_not_kill_each_other() {
    let src = "function f(c: boolean): string {\n  let out: string;\n  if (c) {\n    out = \"a\";\n  } else {\n    out = \"b\";\n  }\n  return out;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_try_and_catch_writes_do_not_kill_each_other() {
    let src = "function f(): boolean {\n  let ok = false;\n  try {\n    ok = risky();\n  } catch {\n    ok = false;\n  }\n  return ok;\n}\ndeclare function risky(): boolean;\n";
    assert_clean(src, "ts");
}

#[test]
fn ts_template_literal_read_counts() {
    let src = "function f(): string {\n  const name = \"a\";\n  return `hi ${name}`;\n}\n";
    assert_clean(src, "ts");
}

#[test]
fn tsx_jsx_expression_counts_as_a_read() {
    let src = "const App = () => {\n  let x = 1;\n  x = 2;\n  return <div>{x}</div>;\n};\n";
    assert_eq!(hits(src, "tsx"), vec![(2, "x".to_string())]);
}

#[test]
fn tsx_component_usage_counts_as_a_read() {
    let src = "function make() {\n  const Inner = () => <span>hi</span>;\n  return <Inner />;\n}\n";
    assert_clean(src, "tsx");
}

#[test]
fn files_with_parse_errors_are_skipped() {
    let src = "function f( {\n  let x = 1;\n  x = 2;\n";
    assert_clean(src, "ts");
}
