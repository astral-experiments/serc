use crate::common;
use crate::common::indent;

fn function(body: &str, context: &str) -> String {
    let body = match context {
        "try" => format!(
            "try:\n{}except OuterError:\n    recover_outer()\n",
            indent(body)
        ),
        "with" => format!("with manager:\n{}", indent(body)),
        _ => body.to_owned(),
    };
    let tail = if context == "generator" {
        "yield values\n"
    } else {
        "return values\n"
    };
    format!(
        "{}def f(values, manager):\n{}",
        if context == "async" { "async " } else { "" },
        indent(&format!("{body}{tail}")),
    )
}

#[test]
fn copies_empty_nested_loop_break_exits_before_propagating_lines() {
    for handler in [
        "except:\n    break\n",
        "except Error:\n    break\n",
        "except Error as error:\n    break\n",
        "except Error:\n    recover()\n    break\n",
        "except Error:\n    break\nelse:\n    success()\n",
    ] {
        let inner = format!("try:\n    consume(inner)\n{handler}");
        let body = format!(
            "for outer in values:\n{}",
            indent(&format!("for inner in values:\n{}", indent(&inner))),
        );
        for context in ["plain", "generator", "async", "try", "with"] {
            common::assert_matches_cpython_marshal(
                &function(&body, context),
                "nested_loop_handler_backedge.py",
            );
        }
    }
}

#[test]
fn preserves_nested_else_pass_locations() {
    for handler in [
        "except Error as error:\n    break\n",
        "except Error:\n    recover()\n    break\n",
    ] {
        let inner = format!("try:\n    consume(inner)\n{handler}");
        let body = format!(
            "for outer in values:\n{}",
            indent(&format!(
                "for inner in values:\n{}else:\n    pass\n",
                indent(&inner),
            )),
        );
        for context in ["plain", "generator", "async", "with"] {
            common::assert_matches_cpython_marshal(
                &function(&body, context),
                "nested_loop_else_pass.py",
            );
        }
    }
}

#[test]
fn preserves_copied_finally_break_predecessor_locations() {
    for asynchronous_inner in [false, true] {
        for named in [false, true] {
            let handler = if named {
                "except Error as error:\n    recover(error)\n"
            } else {
                "except Error:\n"
            };
            let body = format!(
                "for outer in values:\n    prepare_outer()\n    {}for inner in values:\n        try:\n            consume()\n{}",
                if asynchronous_inner { "async " } else { "" },
                indent(&indent(&format!(
                    "{handler}    try:\n        break\n    finally:\n        cleanup()\n",
                ))),
            );
            let context = if asynchronous_inner { "async" } else { "plain" };
            common::assert_matches_cpython_marshal(
                &function(&body, context),
                "nested_loop_finally_break.py",
            );
        }
    }
}
