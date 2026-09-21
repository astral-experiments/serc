use crate::common;

#[test]
fn matches_cpython_conditional_jump_shared_disjunction_join_location() {
    let source = r"def probe(selector, outer, left, right):
    if outer and (left or right if selector else True):
        consume(1)
    else:
        consume(2)
    consume(3)
";
    common::assert_matches_cpython_marshal(source, "conditional_jump_shared_join.py");
}

#[test]
fn matches_cpython_conditional_jump_single_predecessor_join_location() {
    let source = r"def probe(selector, outer, left, right):
    if outer and (right if selector else True):
        consume(1)
    else:
        consume(2)
    consume(3)
";
    common::assert_matches_cpython_marshal(source, "conditional_jump_single_join.py");
}

#[test]
fn matches_cpython_conditional_jump_after_unconditional_exits() {
    common::assert_matches_cpython_marshal(
        r"def finalizer(outer, inner, call):
    if outer:
        if inner:
            before()
            try:
                call()
            finally:
                after()
    else:
        alternate()
    finish()

def exhaustive_match(outer, inner, mode, value):
    if outer:
        if inner:
            match mode:
                case None:
                    raise ValueError
                case 0:
                    value = inner
                case _:
                    raise TypeError
    else:
        value = outer
    finish(value)

def following_work(outer, inner, mode, value):
    if outer:
        if inner:
            match mode:
                case None:
                    raise ValueError
                case 0:
                    value = inner
                case _:
                    raise TypeError
            consume(value)
    else:
        value = outer
    finish(value)
",
        "conditional_jump_after_unconditional_exits.py",
    );
}

#[test]
fn matches_cpython_loop_false_edge_join_locations() {
    for (filename, source) in [
        (
            "loop_exit_for_else_return.py",
            r"def f(flag, items):
    if flag:
        for item in items:
            if item:
                break
        else:
            if flag:
                value = 1
                return value
    else:
        other()
    after()
",
        ),
        (
            "loop_exit_for_else_raise.py",
            r"def f(flag, items):
    try:
        work()
    except Exception:
        recover()
    else:
        for item in items:
            if item:
                break
        else:
            if flag:
                raise ValueError()
    after()
",
        ),
        (
            "loop_exit_while_branch.py",
            r"def f(flag, items):
    if flag:
        if other():
            work()
        else:
            values = list(items)
            while values:
                consume(values.pop())
    else:
        other()
    after()
",
        ),
        (
            "loop_exit_for_condition1.py",
            r"def f(flag, items):
    if flag:
        for item in items:
            if item:
                break
        else:
            if flag or item:
                value = 1
                return value
    else:
        other()
    after()
",
        ),
        (
            "loop_exit_while_condition2.py",
            r"def f(flag, items):
    if flag:
        if other():
            work()
        else:
            values = list(items)
            while not flag:
                consume(values.pop())
    else:
        other()
    after()
",
        ),
        (
            "loop_exit_for_else_following.py",
            r"def f(flag, items):
    if flag:
        for item in items:
            if item:
                break
        else:
            if flag:
                value = 1
                return value
            finish()
    else:
        other()
    after()
",
        ),
        (
            "loop_exit_while_following.py",
            r"def f(flag, items):
    if flag:
        if other():
            work()
        else:
            values = list(items)
            while values:
                consume(values.pop())
            finish()
    else:
        other()
    after()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_final_else_after_terminating_elif() {
    for (filename, source) in [
        (
            "matches_cpython_final_else_after_terminating_elif_role00.py",
            r"def f(flag, first, second, last):
    if flag:
        if first:
            raise ValueError()
        elif second:
            raise TypeError()
        else:
            if not last():
                raise RuntimeError()
    else:
        other()
    after()
",
        ),
        (
            "matches_cpython_final_else_after_terminating_elif_role11.py",
            r"def f(flag, first, second, last):
    if flag:
        if first:
            work()
        elif second:
            recover()
        else:
            if not last():
                raise RuntimeError()
    else:
        other()
    after()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_for_else_shared_branch_exit() {
    const SOURCE: &str = r"def f(flag, items, again, done):
    if flag:
        for item in items:
            if again:
                break
            work(item)
        else:
            if done:
                result = work()
    else:
        return fallback()
    after()
";
    for (filename, source) in [
        ("for_else_shared.py", SOURCE.to_owned()),
        (
            "for_else_and.py",
            SOURCE.replace("if done:", "if done and again:"),
        ),
        (
            "for_else_no_break.py",
            SOURCE.replace("            if again:\n                break\n", ""),
        ),
        (
            "for_else_return_control.py",
            SOURCE.replace("result = work()", "return work()"),
        ),
        (
            "for_else_raise_control.py",
            SOURCE.replace("result = work()", "raise Error()"),
        ),
        (
            "for_else_folded_control.py",
            SOURCE.replace("if done:", "if False:"),
        ),
    ] {
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_with_shared_nested_branch_exit() {
    const SOURCE: &str = r"def f(items, flag, branch, value, seen):
    for item in items:
        with manager():
            if flag:
                if branch:
                    if seen:
                        ACTION
                else:
                    value = work()
                    if value is not None and seen:
                        ACTION
            else:
                after()
";
    for action in ["yield value", "work(value)", "result = value"] {
        for managers in ["manager()", "manager(), other_manager()"] {
            let source = SOURCE
                .replace("ACTION", action)
                .replace("manager()", managers);
            common::assert_matches_cpython_marshal(&source, "with_shared_nested_branch.py");
        }
    }
}
