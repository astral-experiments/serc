use crate::common;

#[test]
fn matches_cpython_nested_manager_exit_nops() {
    for definition in ["def", "async def"] {
        for managers in [
            "        a,\n        b,",
            "        a,\n        b,\n        c,",
        ] {
            for body in [
                "        while True:\n            work()\n",
                "        try:\n            work()\n        except ValueError:\n            return 1\n",
            ] {
                let source = format!(
                    "{definition} f(a, b, c, flag):
    with (
{managers}
    ):
{body}    done()
"
                );
                common::assert_matches_cpython_marshal(&source, "nested_manager_exit_nops.py");
            }
        }
    }
}

#[test]
fn matches_cpython_async_manager_exit_nops() {
    for (body, cleanup) in [
        ("yield", "await cleanup()"),
        ("await work()", "await cleanup()"),
        ("work()", "cleanup()"),
    ] {
        let source = format!(
            "async def f(a, b, flag):
    with a:
        try:
            async with b:
                {body}
        finally:
            if flag:
                {cleanup}
"
        );
        common::assert_matches_cpython_marshal(&source, "async_manager_exit_nops.py");
    }
}

#[test]
fn preserves_cpython_located_manager_exit_nops() {
    for source in [
        "def f(a, b, xs, flag):
    with (
        a,
        b,
    ):
        return 1
    done()
",
        "def f(a, b, xs, flag):
    with (
        a,
        b,
    ):
        raise ValueError()
    done()
",
        "def f(a, b, xs, flag):
    with a:
        try:
            work()
        except ValueError:
            return 1
    done()
",
        "def f(a, b, xs, flag):
    with a:
        with b:
            try:
                work()
            except ValueError:
                return 1
    done()
",
        "def f(a, b, flag):
    with (
        a,
        b,
    ):
        while True:
            work()
        else:
            return 2
    done()
",
        "def f(a, b, flag):
    with (
        a,
        b,
    ):
        while True:
            work()
        else:
            raise ValueError()
    done()
",
        "def f(a, b, flag):
    with (
        a,
        b,
    ):
        while True:
            work()
        else:
            if flag:
                return 2
            else:
                return 3
    done()
",
        "async def f(a, b, flag):
    with (
        a,
        b,
    ):
        while True:
            work()
        else:
            return 2
    done()
",
        "def f(a, b, flag):
    with (
        a,
        b,
        a,
    ):
        while True:
            work()
        else:
            return 2
    done()
",
    ] {
        common::assert_matches_cpython_marshal(source, "located_manager_exit_nops.py");
    }
}

#[test]
fn matches_cpython_unprotected_manager_exit_markers() {
    for source in [
        "def f(a, b, flag):
    with (
        a,
        b,
        a,
    ):
        try:
            return flag
        except Error:
            raise
    done()
",
        "def f(a, b, flag):
    with (
        a,
        b,
        a,
        b,
    ):
        try:
            return flag
        except Error:
            raise
    done()
",
        "def f(a, b, flag):
    with (
        a,
        b,
        a,
    ):
        try:
            raise Error()
        except Error:
            return flag
    done()
",
        "def f(a, b, flag):
    try:
        with (
            a,
            b,
            a,
        ):
            try:
                return flag
            except Error:
                raise
    except Error:
        raise
",
        "def f(a, b, flag):
    try:
        with (
            a,
            b,
            a,
        ):
            try:
                return flag
            except Error:
                raise
    finally:
        done()
",
        "def f(a, b, flag):
    yield flag
    try:
        with (
            a,
            b,
            a,
        ):
            try:
                return flag
            except Error:
                raise
    finally:
        done()
",
    ] {
        common::assert_matches_cpython_marshal(source, "unprotected_manager_exit_markers.py");
    }
}

#[test]
fn matches_cpython_terminal_nested_manager_exit_nops() {
    fn indent(source: &str) -> String {
        format!("    {}\n", source.trim_end().replace('\n', "\n    "))
    }

    for scope in 0..4 {
        let prefix = if matches!(scope, 1 | 3) { "async " } else { "" };
        let suspension = match scope {
            1 => "await prepare(a)\n",
            2 | 3 => "yield a\n",
            _ => "",
        };
        for body in [
            "pass\n",
            "if b:\n    return\nelse:\n    return\n",
            "if b:\n    raise ValueError\nelse:\n    return\n",
            "try:\n    return\nexcept ValueError:\n    raise\n",
            "try:\n    return\nexcept ValueError as error:\n    raise error\n",
            "try:\n    action(a)\nexcept ValueError:\n    pass\n",
            "while b:\n    action(a)\n",
            "for item in a:\n    action(item)\nelse:\n    return\n",
        ] {
            for layout in 0..6 {
                if matches!(layout, 1 | 4) && !matches!(scope, 1 | 3) {
                    continue;
                }
                let inner_header = if matches!(layout, 1 | 4) {
                    "async with manager:\n"
                } else {
                    "with manager:\n"
                };
                let inner = format!("{inner_header}{}", indent(body));
                let suite = match layout {
                    0 | 1 => format!("with outer:\n{}", indent(&inner)),
                    2 => format!("with (\n    outer,\n    manager,\n):\n{}", indent(body)),
                    3 | 4 => format!(
                        "with (\n    extra,\n    outer,\n):\n{}",
                        indent(&format!("action(a)\n{inner}")),
                    ),
                    _ => format!(
                        "with extra:\n{}",
                        indent(&format!("with outer:\n{}", indent(&inner))),
                    ),
                };
                let source = format!(
                    "{prefix}def f(a, b, manager, outer):\n{}",
                    indent(&format!("{suspension}{suite}return\n")),
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminal_nested_manager_exit_nops.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_elif_manager_exit_nops() {
    for definition in ["def", "async def"] {
        for (first, last) in [
            ("return", "raise ValueError"),
            ("return a", "return b"),
            ("raise ValueError", "raise TypeError"),
        ] {
            let source = format!(
                "{definition} f(manager, a, b):
    with manager:
        if a:
            {first}
        elif b:
            {last}
"
            );
            common::assert_matches_cpython_marshal(&source, "terminal_elif_manager_exit.py");
        }
    }
}

#[test]
fn preserves_cpython_fallthrough_manager_exits() {
    for definition in ["def", "async def"] {
        for alternative in ["elif b", "else"] {
            let source = format!(
                "{definition} f(manager, a, b):
    with manager:
        if a:
            return
        {alternative}:
            work()
"
            );
            common::assert_matches_cpython_marshal(&source, "fallthrough_manager_exit.py");
        }
    }
}

#[test]
fn preserves_cpython_manager_exit_predecessor_boundaries() {
    for test in ["b or c", "b and c"] {
        let source = format!(
            "def f(manager, a, b, c):
    with manager:
        if a:
            return
        elif {test}:
            raise ValueError
"
        );
        common::assert_matches_cpython_marshal(&source, "manager_exit_predecessors.py");
    }
    for source in [
        "def f(manager, a, b):
    with manager:
        if a:
            while True:
                consume()
        elif b:
            raise ValueError
",
        "async def f(manager, a, b):
    async with manager:
        if a:
            return
        elif b:
            raise ValueError
",
    ] {
        common::assert_matches_cpython_marshal(source, "manager_exit_boundaries.py");
    }
}

#[test]
fn terminal_match_call_preserves_manager_exit_nop() {
    let source = r"def function(manager, value, fallback):
    with manager:
        match value:
            case Kind.FIRST:
                return handle(value)
            case _:
                fallback(value)
";
    common::assert_matches_cpython_marshal(source, "terminal_match_manager_exit.py");
}

#[test]
fn terminal_async_match_call_preserves_manager_exit_nop() {
    let source = r"async def function(manager, value, fallback):
    with manager:
        match value:
            case Request():
                return await handle(value)
            case _:
                fallback(value)
";
    common::assert_matches_cpython_marshal(source, "terminal_async_match_manager_exit.py");
}

#[test]
fn terminal_match_prefix_preserves_manager_exit_nop() {
    let source = r"def function(manager, value, prepare, fallback):
    with manager:
        match value:
            case Kind.FIRST:
                return handle(value)
            case _:
                prepare(value)
                fallback(value)
";
    common::assert_matches_cpython_marshal(source, "terminal_match_prefix_manager_exit.py");
}

#[test]
fn ordinary_call_manager_exit_control() {
    let source = r"def function(manager, value, fallback):
    with manager:
        fallback(value)
";
    common::assert_matches_cpython_marshal(source, "ordinary_call_manager_exit.py");
}

#[test]
fn matches_cpython_coroutine_return_join_before_sync_manager_exit() {
    let source = r"async def f(manager, read, flag):
    with manager:
        try:
            value = await read()
            return value if flag else None
        except ValueError:
            raise
";
    common::assert_matches_cpython_marshal(source, "coroutine_return_join_sync_manager.py");
}

#[test]
fn matches_cpython_coroutine_return_join_before_async_manager_exit() {
    let source = r"async def f(manager, read, flag):
    async with manager:
        try:
            value = await read()
            return value if flag else None
        except ValueError:
            raise
";
    common::assert_matches_cpython_marshal(source, "coroutine_return_join_async_manager.py");
}

#[test]
fn retains_cpython_sync_return_join_before_manager_exit() {
    let source = r"def f(manager, read, flag):
    with manager:
        try:
            value = read()
            return value if flag else None
        except ValueError:
            raise
";
    common::assert_matches_cpython_marshal(source, "sync_return_join_manager_control.py");
}

#[test]
fn retains_cpython_coroutine_return_join_without_inner_try() {
    let source = r"async def f(manager, read, flag):
    async with manager:
        value = await read()
        return value if flag else None
";
    common::assert_matches_cpython_marshal(source, "coroutine_return_join_no_try_control.py");
}

#[test]
fn nested_manager_suppression_match_exit() {
    let source = r#"
def make_runner(outer, inner, kind):
    with outer:
        match kind:
            case "primary":
                with inner:
                    def run(value):
                        return consume(value)
                    return run
            case _:
                raise ValueError(kind)
"#;
    common::assert_matches_cpython_marshal(source, "nested_manager_match_suppression.py");
}

#[test]
fn nested_manager_suppression_if_exit_control() {
    let source = r#"
def make_runner(outer, inner, kind):
    with outer:
        if kind == "primary":
            with inner:
                def run(value):
                    return consume(value)
                return run
        else:
            raise ValueError(kind)
"#;
    common::assert_matches_cpython_marshal(source, "nested_manager_if_suppression.py");
}
