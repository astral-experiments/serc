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
