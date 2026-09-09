use crate::common;

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn assert_manager_suppression(kind: &str, outer: &str, body: &str, cleanup: &str) {
    let body = if body == "raise" {
        "raise error\n"
    } else {
        "work()\n"
    };
    let cleanup = match cleanup {
        "single" => format!("with manager:\n{}", indent(body)),
        "multi" => format!("with manager, other:\n{}", indent(body)),
        "following_call" => format!("with manager:\n{}finish_inner()\n", indent(body)),
        "following_if" => format!(
            "with manager:\n{}if flag:\n    finish_inner()\n",
            indent(body)
        ),
        "following_loop" => format!(
            "with manager:\n{}for item in values:\n    observe(item)\n",
            indent(body)
        ),
        _ => unreachable!("unknown manager cleanup"),
    };
    let inner = format!("try:\n    work()\nfinally:\n{}", indent(&cleanup));
    let mut suite = match outer {
        "none" => inner,
        "pass" => format!("try:\n{}finally:\n    pass\n", indent(&inner)),
        "call" | "nested" => format!("try:\n{}finally:\n    finish()\n", indent(&inner)),
        _ => unreachable!("unknown outer finalizer"),
    };
    if outer == "nested" {
        suite = format!("try:\n{}finally:\n    final_finish()\n", indent(&suite));
    }
    if kind == "generator" {
        suite = format!("yield seed\n{suite}");
    }
    let definition = if kind == "coroutine" {
        "async def"
    } else {
        "def"
    };
    let source = format!(
        "{definition} f(manager, other, outer_manager, flag, error, values, seed):\n{}",
        indent(&suite)
    );
    common::assert_matches_cpython_marshal(&source, "finally_manager_suppression_copies.py");
}

#[test]
fn copies_manager_suppression_reraise_with_its_finalizer_owner() {
    for kind in ["sync", "coroutine", "generator"] {
        for outer in ["call", "pass", "nested"] {
            for body in ["call", "raise"] {
                for cleanup in ["single", "multi"] {
                    assert_manager_suppression(kind, outer, body, cleanup);
                }
            }
        }
    }
}

#[test]
fn preserves_suppression_without_a_protected_outer_finalizer() {
    for kind in ["sync", "coroutine", "generator"] {
        for body in ["call", "raise"] {
            for cleanup in ["single", "multi"] {
                assert_manager_suppression(kind, "none", body, cleanup);
            }
        }
    }
}

#[test]
fn preserves_nonterminal_and_shared_manager_continuations() {
    for kind in ["sync", "coroutine", "generator"] {
        for cleanup in ["following_call", "following_if", "following_loop"] {
            assert_manager_suppression(kind, "call", "call", cleanup);
        }
    }
}
