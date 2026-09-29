use crate::common;

fn async_else_conditional_finally_source() -> &'static str {
    include_str!(
        "../fixtures/compatibility/normal_finally_original_edges/matches_cpython_marshal_for_async_else_conditional_finally_target/async_else_conditional_finally_target.py"
    )
}

#[test]
fn matches_cpython_marshal_for_async_else_conditional_finally_following_call_control() {
    let source = async_else_conditional_finally_source().replace(
        "                raise errors.Cancelled\n        finally:",
        "                raise errors.Cancelled\n            finish_else(status)\n        finally:",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "async_else_conditional_finally_following_call_control.py",
    );
}

fn nonlinear_arm_nested_finally_source() -> &'static str {
    include_str!(
        "../fixtures/compatibility/normal_finally_original_edges/matches_cpython_marshal_for_nonlinear_arm_nested_finally_target/nonlinear_arm_nested_finally_target.py"
    )
}

#[test]
fn matches_cpython_marshal_for_nonlinear_arm_nested_finally_linear_arm_control() {
    let source = nonlinear_arm_nested_finally_source().replace(
        r#"            durable = committed and available(state)
            if not durable:
                record_blocked(kind="state", reason="commit")
"#,
        r#"            durable = merge_commit(committed, state)
            record_blocked(kind="state", reason="commit")
"#,
    );
    common::assert_matches_cpython_marshal(
        &source,
        "nonlinear_arm_nested_finally_linear_arm_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_nonlinear_arm_nested_finally_entry_call_control() {
    let source = nonlinear_arm_nested_finally_source().replace(
        "    finally:\n        if state.dirty:\n",
        "    finally:\n        cleanup_started()\n        if state.dirty:\n",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "nonlinear_arm_nested_finally_entry_call_control.py",
    );
}

fn conditional_handler_guarded_cleanup_source() -> &'static str {
    include_str!(
        "../fixtures/compatibility/normal_finally_original_edges/matches_cpython_marshal_for_conditional_handler_guarded_cleanup_target/conditional_handler_guarded_cleanup_target.py"
    )
}

#[test]
fn matches_cpython_marshal_for_conditional_handler_guarded_cleanup_entry_call_control() {
    let source = conditional_handler_guarded_cleanup_source().replace(
        "        finally:\n            if worker is not None:\n",
        "        finally:\n            cleanup_started()\n            if worker is not None:\n",
    );
    common::assert_matches_cpython_marshal(
        &source,
        "conditional_handler_guarded_cleanup_entry_call_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_conditional_handler_guarded_cleanup_body_call_control() {
    let source = conditional_handler_guarded_cleanup_source().replace(
        r#"                raise RuntimeError("stream ended before completion")
        except ActionRequired as error:
"#,
        r#"                raise RuntimeError("stream ended before completion")
            body_finished()
        except ActionRequired as error:
"#,
    );
    common::assert_matches_cpython_marshal(
        &source,
        "conditional_handler_guarded_cleanup_body_call_control.py",
    );
}

#[test]
fn matches_cpython_original_for_else_normal_finally_exit() {
    for (filename, condition) in [
        ("folded_for_else_finally_exit.py", "True"),
        ("runtime_for_else_finally_exit.py", "enabled"),
    ] {
        let source = format!(
            "def f(items, value, enabled):\n    for outer in items:\n        try:\n            value += outer\n        finally:\n            for item in items:\n                if {condition}:\n                    break\n            else:\n                consume(value)\n        consume(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}
