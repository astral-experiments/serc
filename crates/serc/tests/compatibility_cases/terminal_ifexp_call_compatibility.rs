use crate::common;

#[test]
fn matches_cpython_marshal_for_terminal_if_call_conditional_suffixes() {
    let source = r#"def original(ctx, tags):
    value = ctx.event
    if value.execution_arn:
        ctx.span.set(tags.arn, value.execution_arn)
    if value.is_replay_execution is not None:
        ctx.span.set(tags.replayed, "true" if value.is_replay_execution else "false")

def truthy_guard(ctx, tags, value):
    if value:
        ctx.span.set(tags.replayed, "true" if value else "false")

def global_call(value):
    if value is not None:
        sink("true" if value else "false")

def nonconstant_leaf(ctx, tags, value, left, right):
    if value is not None:
        ctx.span.set(tags.replayed, left if value else right)

def nested_argument(ctx, tags, value, test):
    if value is not None:
        ctx.span.set(tags.replayed, "true" if value else "fallback" if test else "false")

def call_two_conditionals(value, other):
    if value is not None:
        sink("true" if value else "false", "other" if other else "none")

def nested_terminal_if(value, other):
    if value is not None:
        if other:
            sink("true" if value else "false")

def terminal_if_else(value, other):
    if value is not None:
        sink("true" if value else "false")
    else:
        sink(other)

def explicit_return_control(ctx, tags, value):
    if value is not None:
        return ctx.span.set(tags.replayed, "true" if value else "false")

def nonterminal_control(ctx, tags, value):
    if value is not None:
        ctx.span.set(tags.replayed, "true" if value else "false")
    finish()

def call_then_raise_control(ctx, tags, value):
    if value is not None:
        ctx.span.set(tags.replayed, "true" if value else "false")
        raise ValueError(value)

def keyword_control(value):
    if value is not None:
        sink(flag="true" if value else "false")

def first_argument_control(value):
    if value is not None:
        sink("true" if value else "false", "tail")
"#;
    common::assert_matches_cpython_marshal(source, "terminal_ifexp_call_compatibility.py");
}

#[test]
fn matches_cpython_discarded_if_expression_exit_copies() {
    for (filename, source) in [
        (
            "discarded_ifexp_normal_finally_await_target.py",
            r"async def target(resource):
    try:
        await work()
    finally:
        await resource.close() if resource else None
    return build_result()
",
        ),
        (
            "discarded_ifexp_outer_arm_calls_target.py",
            r"def target(outer, choose, left, right):
    if outer:
        left() if choose else right()
    else:
        fallback()
    finish()
",
        ),
        (
            "discarded_ifexp_protected_outer_arm_target.py",
            r"def target(outer, choose, left, right):
    try:
        if outer:
            left() if choose else right()
        elif choose:
            fallback()
        finish()
    except Exception:
        recover()
    return None
",
        ),
        (
            "discarded_ifexp_following_call_control.py",
            r"def target(outer, choose, left, right):
    if outer:
        left() if choose else right()
        following()
    else:
        fallback()
    finish()
",
        ),
        (
            "discarded_ifexp_folded_false_selector_control.py",
            r"def target(outer, left, right):
    if outer:
        left() if False else right()
    else:
        fallback()
    finish()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_terminal_final_else_name_if_expression_assignments() {
    for (filename, source) in [
        (
            "terminal_final_else_local_ifexp_assignment.py",
            r"def target(enabled, preferred, fallback):
    if enabled:
        result = preferred
    else:
        result = fallback if fallback else None
",
        ),
        (
            "terminal_final_else_module_ifexp_assignment.py",
            r"if checking:
    class Selected(Base):
        pass
else:
    Selected = Fast if available else Slow
",
        ),
        (
            "terminal_tested_arm_name_ifexp_control.py",
            r"def target(enabled, choose, left, right):
    if enabled:
        result = left if choose else right
    else:
        fallback()
",
        ),
        (
            "terminal_direct_name_ifexp_control.py",
            r"def target(choose, left, right):
    result = left if choose else right
",
        ),
        (
            "terminal_nested_final_else_name_ifexp.py",
            r"def target(outer, inner, choose, left, right):
    if outer:
        if inner:
            fallback()
        else:
            result = left if choose else right
    else:
        finish()
",
        ),
        (
            "terminal_nested_tested_name_ifexp_control.py",
            r"def target(outer, inner, choose, left, right):
    if outer:
        fallback()
    else:
        if inner:
            result = left if choose else right
        else:
            finish()
",
        ),
        (
            "nonterminal_final_else_name_ifexp_control.py",
            r"def target(enabled, choose, left, right):
    if enabled:
        fallback()
    else:
        result = left if choose else right
    finish()
",
        ),
        (
            "terminal_final_else_attribute_ifexp_control.py",
            r"def target(holder, enabled, choose, left, right):
    if enabled:
        fallback()
    else:
        holder.value = left if choose else right
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_discarded_ifexp_direct_try_exit_copy() {
    for (filename, source) in [
        (
            "discarded_ifexp_terminal_try_chain_boundary_control.py",
            r#"def target(command, choose, left, right):
    try:
        if command == "one":
            left() if choose else right()
        elif command == "two":
            second()
        else:
            fallback()
    except ValueError:
        recover()
        return 2
    except RuntimeError:
        return 3
    return 0
"#,
        ),
        (
            "discarded_ifexp_try_chain_following_work_target.py",
            r#"def target(command, choose, left, right):
    try:
        if command == "one":
            left() if choose else right()
        elif command == "two":
            second()
        else:
            fallback()
        following()
    except ValueError:
        recover()
        return 2
    except RuntimeError:
        return 3
    return 0
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_discarded_ifexp_handler_cell_ownership() {
    for (filename, source) in [
        (
            "discarded_ifexp_handler_cell_assignment_target.py",
            r"def outer():
    saved = None
    def target(active, inner, choose, value, left, right):
        nonlocal saved
        if active:
            try:
                if inner:
                    left(value) if choose else right(value)
            except Exception:
                left(value) if choose else right(value)
        else:
            left(value) if choose else right(value)
        saved = value
    return target
",
        ),
        (
            "discarded_ifexp_handler_following_work_cell_control.py",
            r"def outer():
    saved = None
    def target(active, inner, choose, value, left, right):
        nonlocal saved
        if active:
            try:
                if inner:
                    left(value) if choose else right(value)
            except Exception:
                left(value) if choose else right(value)
                following()
        else:
            left(value) if choose else right(value)
        saved = value
    return target
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn preserves_terminal_if_discarded_conditional_before_shared_footer() {
    for (function, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for outer in ["flag", "True"] {
            for expression in [
                "left() if choose else right()",
                "left if choose else right",
                "(left() if other else fallback()) if choose else right()",
            ] {
                let source = format!(
                    "{function} f(flag, choose, other, left, right, value):\n{prefix}    if {outer}:\n        {expression}\n"
                );
                common::assert_matches_cpython_marshal(&source, "terminal_if_discarded_join.py");
            }
        }
    }
}
