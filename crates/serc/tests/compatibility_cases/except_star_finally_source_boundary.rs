use crate::common;

#[test]
fn except_star_final_handler_copies_conditional_cleanup() {
    for (filename, handler, body, previous, following) in [
        (
            "except_star_final_conditional.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    other()",
            "",
            "",
        ),
        (
            "except_star_final_named_conditional.py",
            "except* ValueError as group:",
            "if condition:\n    call()\nelse:\n    other()",
            "",
            "",
        ),
        (
            "except_star_final_elif.py",
            "except* ValueError:",
            "if condition:\n    call()\nelif value:\n    other()\nelse:\n    call()",
            "",
            "",
        ),
        (
            "except_star_final_elif_without_else.py",
            "except* ValueError:",
            "if condition:\n    call()\nelif value:\n    other()",
            "",
            "",
        ),
        (
            "except_star_final_nested_conditional.py",
            "except* ValueError:",
            "if condition:\n    if value:\n        call()\n    else:\n        other()\nelse:\n    other()",
            "",
            "",
        ),
        (
            "except_star_final_conditional_pass.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    pass",
            "",
            "",
        ),
        (
            "except_star_final_conditional_assert.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    assert other()",
            "",
            "",
        ),
        (
            "except_star_final_conditional_previous_handler.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    other()",
            "    except* TypeError:\n        call()\n",
            "",
        ),
        (
            "except_star_final_conditional_following_statement.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    other()",
            "",
            "    other()\n",
        ),
        (
            "except_star_nonfinal_conditional_control.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    other()",
            "",
            "    except* TypeError:\n        call()\n",
        ),
        (
            "except_star_final_partial_conditional_control.py",
            "except* ValueError:",
            "if condition:\n    call()",
            "",
            "",
        ),
        (
            "except_star_final_raising_conditional_control.py",
            "except* ValueError:",
            "if condition:\n    raise\nelse:\n    other()",
            "",
            "",
        ),
        (
            "except_star_final_raising_else.py",
            "except* ValueError:",
            "if condition:\n    call()\nelse:\n    raise",
            "",
            "",
        ),
        (
            "except_star_final_constant_conditional_control.py",
            "except* ValueError:",
            "if True:\n    call()\nelse:\n    other()",
            "",
            "",
        ),
        (
            "except_star_final_linear_control.py",
            "except* ValueError:",
            "call()\nother()",
            "",
            "",
        ),
        (
            "ordinary_except_conditional_control.py",
            "except ValueError:",
            "if condition:\n    call()\nelse:\n    other()",
            "",
            "",
        ),
    ] {
        for asynchronous in [false, true] {
            let body = format!("        {}\n", body.replace('\n', "\n        "));
            let prefix = if asynchronous { "async " } else { "" };
            let source = format!(
                "{prefix}def f(value, condition, other):\n    try:\n        call()\n{previous}    {handler}\n{body}{following}"
            );
            common::assert_matches_cpython_marshal(&source, filename);
        }
    }
}
