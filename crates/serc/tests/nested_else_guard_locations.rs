use std::fmt::Write as _;
use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str) {
    assert_matches_cpython_versions(
        source,
        &[
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ],
    );
}

#[test]
fn loop_handler_returns_keep_the_outer_ordinary_try_pop() {
    for handler in ["except:", "except Error:", "except Error as error:"] {
        for returned in [
            "value",
            "[value]",
            "value or [item]",
            "value if flag else [item]",
        ] {
            for cleanup in [
                "if connection is not None:\n            await close(connection, deadline)",
                "if connection is not None and not await close(connection, deadline):\n            raise Error()",
            ] {
                assert_matches_cpython(&format!(
                    "async def f(connection, deadline, items, item, value, flag):
    try:
        for item in items:
            try:
                check(item)
            {handler}
                return {returned}
        return value
    except Exception:
        recover()
        return [value]
    finally:
        {cleanup}
"
                ));
            }
        }
    }
}

#[test]
fn original_try_star_barriers_stop_threaded_manager_exits() {
    for manager in ["with", "async with"] {
        for handler in ["except Error", "except* Error"] {
            let source = format!(
                "async def f(value, other, manager, flag):
    try:
        try:
            {manager} manager:
                work(value)
        {handler}:
            recover(other)
        if flag:
            work(value)
        elif other:
            work(other)
    except Error:
        raise
    finally:
        if value:
            consume(value)
        consume(other)
"
            );
            assert_matches_cpython(&source);
        }
    }
}

#[test]
fn terminal_nested_recovery_keeps_the_manager_borrow_boundary() {
    for manager in ["with", "async with"] {
        for handler in ["except Error", "except Error as error"] {
            for following in ["", "        consume(source)\n"] {
                let source = format!(
                    "async def run(manager, source, sink):
    {manager} manager:
        try:
            try:
                async for item in await source():
                    break
            except Error as error:
                if bad(error) or worse(error):
                    raise Wrapped(error) from error
                record(error)
            try:
                await sink(source)
            {handler}:
                pass
        except Error:
            recover(source)
{following}    consume(source)
"
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn explicit_else_keeps_its_original_borrow_entry_inside_a_manager() {
    for prefix in ["", "            work(value)\n"] {
        assert_matches_cpython(&format!(
            "def f(value, flag):
    with manager():
        try:
            value = acquire()
        except Error:
            result = 0
        else:
{prefix}            try:
                value = derive(value)
            except Error:
                result = 1
            else:
                if flag:
                    result = classify(value)
                else:
                    result = 2
                    value = None
        if result:
            consume(value, result)
        return value
"
        ));
    }
}

fn assert_matches_cpython_versions(source: &str, versions: &[PythonVersion]) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        if !versions.contains(&version) {
            continue;
        }
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let filename = "nested_else_guard_locations.py";
        let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
            .args([
                target.tag.trim_start_matches('v'),
                &target.magic_hex(),
                filename,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let expected = child.wait_with_output().unwrap();
        assert!(
            expected.status.success(),
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for actual in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert_eq!(actual.marshal(), expected.stdout, "{version:?}:\n{source}");
        }
    }
}

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[test]
fn post_try_global_call_conditions_reuse_compacted_owner_slots() {
    for scope in ["function", "coroutine", "generator"] {
        for suffixed in [false, true] {
            for condition in [
                "flag",
                "obj.flag",
                "obj.check()",
                "obj.check(value)",
                "obj.check(local(value))",
                "obj.check(get(value))",
                "obj.check(get(value), other(value))",
                "obj.check(helper.get(value))",
                "obj.check(value=get(value))",
                "obj.check((\"first\", \"second\"))",
                "get(value)",
                "obj.check(local(value), other(value))",
            ] {
                for negation in ["", "not "] {
                    let awaited = if scope == "coroutine" { "await " } else { "" };
                    let prefix = if scope == "generator" {
                        "yield value\n"
                    } else {
                        ""
                    };
                    let suffix = if suffixed { "    consume(value)\n" } else { "" };
                    let body = format!(
                        "{prefix}try:\n    {awaited}work(value)\n{suffix}except Error:\n    return False\nif {negation}{condition}:\n    return False\nreturn True\n"
                    );
                    let definition = if scope == "coroutine" {
                        "async def"
                    } else {
                        "def"
                    };
                    assert_matches_cpython(&format!(
                        "{definition} f(obj, value, local, helper, flag):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn nested_cleanup_return_retains_the_outer_finally_pop_location() {
    for returned in [
        "value",
        "None",
        "{}",
        "call()",
        "{'x': value if flag else None}",
    ] {
        for guarded in [false, true] {
            for cleanup in ["close(value)", "await close(value)"] {
                let returned = if guarded {
                    format!("if flag:\n    return {returned}\n")
                } else {
                    format!("return {returned}\n")
                };
                let inner = format!(
                    "try:\n{}finally:\n    try:\n        call()\n    finally:\n        {cleanup}\n",
                    indent(&returned),
                );
                let body = format!(
                    "try:\n{}finally:\n    if value is not None:\n        pass\n",
                    indent(&inner),
                );
                assert_matches_cpython(&format!("async def f(value, flag):\n{}", indent(&body)));
            }
        }
    }
}

#[test]
fn copied_value_return_unwinds_both_tries_around_while() {
    for returned in ["value", "build(value)"] {
        for handler in ["except Exception:", "except Exception as error:"] {
            for condition in ["value is not None", "value"] {
                for prefix in ["", "    observe(value)\n"] {
                    let inner =
                        format!("try:\n{prefix}    return {returned}\n{handler}\n    pass\n");
                    let loop_body = format!("while True:\n{}", indent(&inner));
                    let body = format!(
                        "try:\n{}except ValueError:\n    pass\nfinally:\n    if {condition}:\n        finish(value)\n",
                        indent(&loop_body)
                    );
                    assert_matches_cpython(&format!("async def f(value):\n{}", indent(&body)));
                }
            }
        }
    }
}

#[test]
fn match_constant_return_keeps_the_finally_pop_location() {
    for (first, second) in [("1", "2"), ("A()", "B()")] {
        for returned in ["return False", "return None", "return"] {
            for (prefix, cleanup) in [
                ("", "close()"),
                ("async ", "close()"),
                ("async ", "await close()"),
            ] {
                let source = format!(
                    "{prefix}def f(value):\n\
                     \x20   try:\n\
                     \x20       match value:\n\
                     \x20           case {first}:\n\
                     \x20               {returned}\n\
                     \x20           case {second}:\n\
                     \x20               {returned}\n\
                     \x20       return None\n\
                     \x20   finally:\n\
                     \x20       {cleanup}\n"
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn ordinary_try_singletons_keep_fresh_owner_slots_after_cold_splits() {
    for pattern in ["True, False", "False, True", "True, True", "False, False"] {
        for handler in [
            "except (TypeError, ValueError):\n    raise RuntimeError from None\n",
            "except (TypeError, ValueError) as error:\n    raise RuntimeError from error\n",
        ] {
            let body = format!(
                "try:\n    match value:\n        case {pattern}:\n            return action()\n        case _:\n            return None\n{handler}"
            );
            assert_matches_cpython(&format!("def f(value, action):\n{}", indent(&body)));
        }
    }
}

#[test]
fn ordinary_try_singletons_preserve_ordered_owners_with_extended_branches() {
    for subject in ["value", "(first, second)"] {
        for prefix in [String::new(), "item = action()\n".repeat(80)] {
            for handler in [
                "except (TypeError, ValueError):\n    raise RuntimeError from None\n",
                "except (TypeError, ValueError) as error:\n    raise RuntimeError from error\n",
            ] {
                let mut cases = format!("match {subject}:\n");
                for pattern in ["True, False", "False, True", "True, True"] {
                    write!(
                        cases,
                        "    case {pattern}:\n{}",
                        indent(&indent(&format!("{prefix}return action()\n")))
                    )
                    .unwrap();
                }
                cases.push_str("    case _:\n        raise RuntimeError\n");
                let body = format!("try:\n{}{handler}", indent(&cases));
                assert_matches_cpython(&format!(
                    "def f(value, first, second, action):\n{}",
                    indent(&body)
                ));
            }
        }
    }
}

#[test]
fn copied_finally_none_guard_keeps_nested_try_manager_pop_order() {
    for load in ["global", "local", "cell"] {
        for condition in ["is None", "is not None"] {
            for tail in 0..3 {
                let parameters = if load == "global" {
                    "flag, value"
                } else {
                    "flag, value, resource"
                };
                let capture = if load == "cell" {
                    "    def captured():\n        return resource\n"
                } else {
                    ""
                };
                let drain = if tail == 2 {
                    "                        while await stream.receive():\n                            pass\n"
                } else {
                    "                        close(stream)\n"
                };
                let close = if tail == 2 {
                    "                    await stream.aclose()\n"
                } else {
                    ""
                };
                let finish = if tail == 0 {
                    "finish(resource)"
                } else {
                    "await finish(resource)"
                };
                let source = format!(
                    "async def f({parameters}):\n\
                     {capture}\
                     \x20   try:\n\
                     \x20       with manager():\n\
                     \x20           if flag:\n\
                     \x20               pass\n\
                     \x20           try:\n\
                     \x20               return parser.parse(output.get('data', b''), strict=True)\n\
                     \x20           except ValueError:\n\
                     \x20               pass\n\
                     \x20   except Error:\n\
                     \x20       pass\n\
                     \x20   finally:\n\
                     \x20       if resource {condition}:\n\
                     \x20           with manager():\n\
                     \x20               if state is not None:\n\
                     \x20                   try:\n\
                     \x20                       close(state)\n\
                     \x20                   except Error:\n\
                     \x20                       record(state)\n\
                     \x20               for stream in resource.streams:\n\
                     \x20                   try:\n\
                     {drain}\
                     \x20                   except Error:\n\
                     \x20                       record(stream)\n\
                     {close}\
                     \x20               {finish}\n"
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn manager_pop_keeps_a_copied_finally_call_at_its_original_while_end() {
    for definition in ["def", "async def"] {
        for cleanup in ["action()", "obj.close()"] {
            for prefix in ["", "action()\n"] {
                for continuation in ["", "        if flag:\n            action()\n"] {
                    let body = format!(
                        "{prefix}with manager:\n\
                         \x20   while flag:\n\
                         \x20       try:\n\
                         \x20           result = action()\n\
                         \x20       except ValueError:\n\
                         \x20           pass\n\
                         \x20       else:\n\
                         \x20           if result:\n\
                         \x20               break\n\
                         \x20       finally:\n\
                         \x20           {cleanup}\n\
                         {continuation}\
                         \x20   else:\n\
                         \x20       raise RuntimeError\n\
                         return obj\n"
                    );
                    let source = format!(
                        "{definition} f(manager, action, obj, flag):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython_versions(&source, &[PythonVersion::Py315]);
                }
            }
        }
    }
}

#[test]
fn manager_pop_ignores_dead_returning_handler_join_alias() {
    for definition in ["def", "async def"] {
        for target in ["result", "result, extra"] {
            for call in ["action()", "action(value=obj)"] {
                let source = format!(
                    "{definition} f(manager, flag, action, obj):\n\
                         \x20   with manager:\n\
                         \x20       if flag:\n\
                         \x20           {target} = {call}\n\
                         \x20       else:\n\
                         \x20           try:\n\
                         \x20               return action()\n\
                         \x20           except ValueError as error:\n\
                         \x20               raise RuntimeError(obj) from error\n\
                         \x20   return obj\n"
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn shared_while_exits_keep_separate_return_copies() {
    for context in ["sync", "async", "generator"] {
        let (definition, action) = match context {
            "async" => ("async def", "value += await work(value)"),
            "generator" => ("def", "yield value\nvalue += 1"),
            _ => ("def", "value += work(value)"),
        };
        for condition in [
            "active and ready",
            "active and (ready or other)",
            "active or ready",
        ] {
            for handler in [
                "except Error:\n    recover()\n",
                "except Error as error:\n    recover(error)\n",
            ] {
                let body = format!(
                    "while {condition}:\n{}    if stop:\n        break\n",
                    indent(action)
                );
                for returned in ["value", "None"] {
                    let suite = format!("try:\n{}{handler}return {returned}\n", indent(&body));
                    let source = format!(
                        "{definition} f(active, ready, other, value, stop):\n{}",
                        indent(&suite)
                    );
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}

#[test]
fn shared_while_false_edges_keep_the_normal_try_exit_unlocated() {
    for context in ["sync", "async", "generator"] {
        let definition = if context == "async" {
            "async def"
        } else {
            "def"
        };
        let action = match context {
            "async" => "await work(value)",
            "generator" => "yield value",
            _ => "work(value)",
        };
        for condition in [
            "active",
            "not active",
            "active and ready",
            "active or ready",
            "active and (ready or other)",
        ] {
            for handler in [
                "except Error as error:\n    raise Other from error\n",
                "except Error:\n    raise\n",
                "except Error as error:\n    recover(error)\n",
            ] {
                for has_break in [false, true] {
                    let mut body = format!("while {condition}:\n    {action}\n    value -= 1\n");
                    if has_break {
                        body.push_str("    if stop or finished:\n        break\n");
                    }
                    let guarded = format!("try:\n{}{handler}", indent(&body));
                    let suite = format!(
                        "if selected:\n    prepare()\nelif alternate:\n{}else:\n    raise Missing\nfinish()\n",
                        indent(&guarded)
                    );
                    let source = format!(
                        "{definition} f(active, ready, other, value, stop, finished, selected, alternate):\n{}",
                        indent(&suite)
                    );
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}

#[test]
fn paired_async_for_exits_preserve_their_copied_return_nops() {
    for condition in ["flag is not None", "flag", "flag and ready"] {
        for other in [
            "async for value in values:\n    result.append(value)\n",
            "values = await load()\nasync for value in values:\n    if value is not None:\n        result.append(value)\n",
        ] {
            for handler in [
                "except Error as error:\n    return result, error\n",
                "except Error:\n    return None\n",
            ] {
                for returned in ["result", "(result, None)", "(result, flag)", "None"] {
                    let branch = format!(
                        "if {condition}:\n    async for value in values:\n        result.append(value)\nelse:\n{}",
                        indent(other)
                    );
                    let body = format!(
                        "result = []\ntry:\n{}{handler}return {returned}\n",
                        indent(&branch)
                    );
                    assert_matches_cpython(&format!(
                        "async def f(flag, ready, values):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn populated_break_sources_keep_their_copied_return_line() {
    for arm in 0..3 {
        for prefix in ["action()\n", "action(key=value)\n"] {
            for closure in [false, true] {
                for returned in ["obj", "None"] {
                    for condition in ["not obj.done", "flag"] {
                        for guard in ["value", "value.get('grade') == 'PASS'"] {
                            let loop_body = format!(
                                "while {condition}:\n    value = action()\n    if {guard}:\n{}",
                                indent(&indent(&format!("{prefix}break\n")))
                            );
                            let mut arms = [
                                "work()\n".to_owned(),
                                "other()\n".to_owned(),
                                "finish()\n".to_owned(),
                            ];
                            arms[arm] = loop_body;
                            let local = if closure {
                                "def action(**kwargs):\n    return obj.step(kwargs)\n"
                            } else {
                                ""
                            };
                            let body = format!(
                                "{local}if choice:\n{}elif alternate:\n{}else:\n{}return {returned}\n",
                                indent(&arms[0]),
                                indent(&arms[1]),
                                indent(&arms[2]),
                            );
                            assert_matches_cpython(&format!(
                                "def f(obj, choice, alternate, flag):\n{}",
                                indent(&body)
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn builtin_generator_guards_reuse_observed_local_fusion_slots() {
    for fusions in 1..=3 {
        for element in ["item", "convert(item)"] {
            let mut prefix = "items = normalize(items, helper.name)\n".to_owned();
            for _ in 1..fusions {
                prefix.push_str("work(items, other)\n");
            }
            prefix.push_str("work(items, {1, 2, 3, 4}, helper.name)\n");
            prefix.push_str("work(check(other, list), 'invalid')\n");
            let body = format!(
                "{prefix}try:\n    value = tuple({element} for item in items)\nexcept Error as error:\n    raise OtherError('invalid') from error\nreturn value\n"
            );
            assert_matches_cpython(&format!("def f(items, other, helper):\n{}", indent(&body)));
        }
    }
}

#[test]
fn builtin_guard_owners_survive_empty_cold_region_removal() {
    for builtin in ["tuple", "all", "any"] {
        for extra_fusions in 0..=3 {
            for depth in 1..=3 {
                for finalizer in ["pass", "finish()"] {
                    let mut prefix = "if flag:\n    pass\na, b, c = unpack()\nprefix = combine(c, other)\ntext = prepare(a)\nclient = create()\n".to_owned();
                    for _ in 0..extra_fusions {
                        prefix.push_str("work(value, other)\n");
                    }
                    let mut body = format!(
                        "result = make(text, client=client)\ntry:\n    return {builtin}(transform(name) for item in iterate(prefix)\n                    if (name := match(item, prefix)) and 'x' not in name)\nfinally:\n    finish(result)\n"
                    );
                    for _ in 1..depth {
                        body = format!("try:\n{}finally:\n    {finalizer}\n", indent(&body));
                    }
                    assert_matches_cpython(&format!(
                        "def f(value, other, flag):\n{}",
                        indent(&format!("{prefix}{body}"))
                    ));
                }
            }
        }
    }
}

#[test]
fn builtin_entry_cells_consume_the_original_retired_guard_slots() {
    for builtin in ["tuple", "all", "any"] {
        for cells in 0..=4 {
            let captured = (0..cells)
                .map(|cell| format!("cell{cell}"))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if captured.is_empty() {
                String::new()
            } else {
                format!(", {captured}")
            };
            for extra_fusions in 0..=7 {
                for mode in ["entry", "branch", "generator"] {
                    let mut prefix = String::new();
                    if mode == "branch" {
                        prefix.push_str("if flag:\n    return None\n");
                    }
                    prefix.push_str("a, b, c = unpack()\nprefix = combine(c, other)\ntext = prepare(a)\nclient = create()\n");
                    for _ in 0..extra_fusions {
                        prefix.push_str("work(value, other)\n");
                    }
                    let mut body = format!(
                        "{prefix}result = make(text, client=client)\ntry:\n    return {builtin}(transform(item{suffix}) for item in iterate(prefix))\nfinally:\n    finish(result)\n"
                    );
                    if mode == "generator" {
                        body.push_str("yield value\n");
                    }
                    assert_matches_cpython(&format!(
                        "def f(value, other, flag{suffix}):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn builtin_guards_after_terminal_arms_keep_their_original_owner() {
    for fusions in 1..=3 {
        let mut prefix =
            "if skip:\n    return None\nitems = normalize(items, helper.name)\n".to_owned();
        for _ in 1..fusions {
            prefix.push_str("work(items, other)\n");
        }
        prefix.push_str("work(items, {1, 2, 3, 4}, helper.name)\n");
        prefix.push_str("work(check(other, list), 'invalid')\n");
        let body = format!(
            "{prefix}try:\n    value = tuple(convert(item) for item in items)\nexcept Error as error:\n    raise OtherError('invalid') from error\nreturn value\n"
        );
        assert_matches_cpython(&format!(
            "def f(items, other, helper, skip):\n{}",
            indent(&body)
        ));
    }
}

#[test]
fn nested_try_else_assignments_keep_their_independent_borrow_entry() {
    for handler in ["except Error:", "except Error as error:"] {
        for condition in ["flag", "obj.value > 0", "not flag", "flag and other"] {
            for prefix in ["", "    work(value)\n"] {
                let body = format!(
                    "try:\n    value = acquire()\n{handler}\n    result = 0\nelse:\n{prefix}    try:\n        value = derive(value)\n    {handler}\n        result = 1\n    else:\n        if {condition}:\n            result = classify(value)\n        else:\n            result = 2\n            value = None\nif result:\n    consume(value, result)\nreturn value\n"
                );
                assert_matches_cpython(&format!(
                    "def f(value, flag, other, obj, items):\n{}",
                    indent(&body)
                ));
            }
        }
    }
}

#[test]
fn nested_else_return_and_raise_guards_donate_the_normal_try_exit_location() {
    for definition in ["def", "async def"] {
        for test in ["value", "not value", "not value[0]"] {
            for terminal in ["return result()", "raise Error(value)"] {
                for depth in [1, 2, 3] {
                    let mut body = format!("if {test}:\n    {terminal}\n");
                    for _ in 0..depth {
                        body = format!(
                            "if flag:\n    update(value)\nelse:\n{}",
                            indent(&format!("for item in values:\n    prepare(item)\n{body}")),
                        );
                    }
                    for in_else in [false, true] {
                        let suite = if in_else {
                            format!(
                                "try:\n    work(value)\nexcept Error as error:\n    recover(error)\nelse:\n{}",
                                indent(&body)
                            )
                        } else {
                            format!(
                                "try:\n{}except Error as error:\n    recover(error)\n",
                                indent(&body)
                            )
                        };
                        let source = format!(
                            "{definition} f(value, flag, values):\n{}    finish(value)\n",
                            indent(&suite)
                        );
                        assert_matches_cpython(&source);
                    }
                }
            }
        }
    }
}

#[test]
fn nested_guards_keep_their_donor_inside_protected_cleanup_and_loops() {
    for definition in ["def", "async def"] {
        for terminal in [
            "return result()",
            "raise Error(value)",
            "raise Error(value) from other",
            "if other:\n    raise Error(value)\nraise Other(value)",
        ] {
            let mut body = format!("if not value:\n{}", indent(terminal));
            for _ in 0..2 {
                body = format!(
                    "if flag:\n    update(value)\nelse:\n{}",
                    indent(&format!("for item in values:\n    prepare(item)\n{body}")),
                );
            }
            let suite = format!(
                "try:\n{}except Error as error:\n    recover(error)\n",
                indent(&body),
            );
            for protected in [
                format!("{suite}finally:\n    cleanup(value)\n"),
                format!(
                    "while ready(value):\n{}    advance(value)\n",
                    indent(&suite)
                ),
            ] {
                let source = format!(
                    "{definition} f(value, flag, values, other):\n{}    finish(value)\n",
                    indent(&protected),
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn nested_final_else_loops_donate_the_shared_branch_exit_location() {
    for definition in ["def", "async def"] {
        for condition in [
            "n > 0",
            "n",
            "not n",
            "n is not None",
            "n or active",
            "check(n)",
            "(n := step())",
        ] {
            for depth in [1, 3] {
                let mut body =
                    format!("n = initial\nwhile {condition}:\n    work(n)\n    n -= 1\n");
                for _ in 0..depth {
                    body = format!(
                        "if choice:\n    work()\nelif alternate:\n    other()\nelse:\n{}",
                        indent(&body)
                    );
                }
                let source = format!(
                    "{definition} f(initial, outer, active, choice, alternate):\n    if outer:\n{}    elif active:\n        other()\n    finish()\n",
                    indent(&indent(&body))
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn nested_final_else_loop_donors_preserve_scope_and_folded_boundaries() {
    let base = "if outer:\n    if choice:\n        work()\n    elif alternate:\n        other()\n    else:\n        n = initial\n        while n > 0:\n            work(n)\n            n -= 1\nelif active:\n    other()\nfinish()\n";
    let variants = [
        base.to_owned(),
        base.replace("if choice:", "if True:"),
        base.replace("if choice:", "if choice and extra:"),
        base.replace("elif alternate:", "elif True:"),
        base.replace("elif alternate:", "elif alternate or extra:"),
        base.replace("        work()\n", "        raise Error\n"),
        base.replace("while n > 0:", "while n and active:"),
        base.replace("while n > 0:", "while True:"),
        base.replace(
            "            n -= 1\n",
            "            n -= 1\n        else:\n            done()\n",
        ),
        base.replace("            n -= 1\n", "            n -= 1\n        pass\n"),
    ];
    for body in variants {
        for (definition, prefix, suite) in [
            ("def", "", body.clone()),
            ("async def", "", body.clone()),
            ("def", "yield initial\n", body.clone()),
            ("async def", "yield initial\n", body.clone()),
            (
                "def",
                "",
                format!("while ready():\n{}    advance()\n", indent(&body)),
            ),
            ("def", "", format!("with manager():\n{}", indent(&body))),
            (
                "def",
                "",
                format!("try:\n{}except Error:\n    recover()\n", indent(&body)),
            ),
            (
                "def",
                "",
                format!("try:\n    prepare()\nfinally:\n{}", indent(&body)),
            ),
            (
                "def",
                "",
                format!("try:\n    return initial\nfinally:\n{}", indent(&body)),
            ),
        ] {
            let source = format!(
                "{definition} f(initial):\n{}{}",
                indent(prefix),
                indent(&suite)
            );
            assert_matches_cpython(&source);
        }
        assert_matches_cpython(&format!("class Subject:\n{}", indent(&body)));
    }
}

#[test]
fn copied_method_finalizers_keep_the_iterator_break_location() {
    for (definition, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "yield resource\n"),
        ("async def", "yield resource\n"),
    ] {
        for loop_header in ["for item in items:", "while active:"] {
            for prefix in ["", "result = False\n", "result = item\n", "record(item)\n"] {
                let loop_body = format!(
                    "resource = open_resource(item)\n{suspension}try:\n    if choose(resource):\n{}finally:\n    resource.close()\n",
                    indent(&indent(&format!("{prefix}break\n"))),
                );
                let protected = format!(
                    "try:\n{}except Error:\n    result = False\n",
                    indent(&format!("{loop_header}\n{}", indent(&loop_body)))
                );
                for wrapped in [false, true] {
                    let body = if wrapped {
                        format!(
                            "if enabled:\n{}    if marker is not None:\n        record(marker)\n",
                            indent(&protected)
                        )
                    } else {
                        protected.clone()
                    };
                    let source = format!(
                        "{definition} f(items, item, active, enabled, marker):\n    result = True\n{}    finish(result)\n",
                        indent(&body)
                    );
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}

#[test]
fn populated_nested_try_arms_keep_explicit_borrow_entries() {
    for (definition, depths) in [("def", 0..2), ("async def", 0..3)] {
        for depth in depths {
            for named in [false, true] {
                for action in ["work()", "value = 1"] {
                    for alternate in ["elif obj.second:", "else:"] {
                        let handler = if named {
                            "except Error as error:\n    pass\n"
                        } else {
                            "except Error:\n    pass\n"
                        };
                        let mut body = format!(
                            "try:\n    work()\n{handler}else:\n    if obj.first:\n        {action}\n    {alternate}\n        pass\n"
                        );
                        for _ in 0..depth {
                            body = format!("try:\n    work()\n{handler}else:\n{}", indent(&body));
                        }
                        body.push_str("consume(obj, obj.attribute)\n");
                        assert_matches_cpython(&format!(
                            "{definition} probe(obj):\n{}",
                            indent(&body),
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn empty_finalizer_exit_does_not_gain_an_ordinary_borrow_route() {
    for (definition, suspended) in [("def", ""), ("async def", "await ")] {
        for outer in [false, true] {
            for independent_entry in [false, true] {
                let trigger = if independent_entry {
                    "try:\n    work()\nfinally:\n    try:\n        work()\n    except Error:\n        pass\n    marker = 1\n"
                } else {
                    ""
                };
                let body = format!(
                    "try:\n    work()\nfinally:\n    try:\n        if obj is None:\n            {suspended}finish()\n    finally:\n        {suspended}close()\nconsume(obj, obj.value)\n{trigger}consume(obj, obj.value)\n",
                );
                let body = if outer {
                    format!(
                        "try:\n{}finally:\n    if obj is not None:\n        {suspended}close()\n",
                        indent(&body),
                    )
                } else {
                    body
                };
                assert_matches_cpython(&format!("{definition} probe(obj):\n{}", indent(&body)));
            }
        }
    }
}

#[test]
fn completed_cold_cleanup_keeps_the_following_manager_pop_line() {
    for protected in [
        "try:\n    try:\n        raise First(value)\n    except First as cause:\n        raise Second(value) from cause\nexcept Second as error:\n    record(error)\n",
        "try:\n    work()\nexcept Error:\n    record()\n    raise\nelse:\n    finish()\nfinally:\n    if ready:\n        close()\n    for item in items:\n        if item is not None:\n            close(item)\n",
    ] {
        for (definition, prefix) in [
            ("def", ""),
            ("def", "prepare()\n"),
            ("async def", ""),
            ("async def", "await prepare()\n"),
        ] {
            for tail in ["return value\n", "finish()\n"] {
                let source = format!(
                    "{definition} f(manager, value, ready, items):\n    with manager:\n{}{}{}",
                    indent(&indent(prefix)),
                    indent(&indent(protected)),
                    indent(tail),
                );
                assert_matches_cpython(&source);
            }
        }
    }
}

#[test]
fn unnamed_handler_break_replays_its_original_finally_none_cells() {
    for prefix in ["", "yield None\n"] {
        for operator in ["is", "is not"] {
            for cleanup in ["pass", "await cleanup(subscriber)"] {
                for closure in [false, true] {
                    let source = format!(
                        "async def run():\n{}    while await ready():\n        try:\n            work()\n        except Cancelled:\n            pass\n        except Exception:\n            if await ready():\n                break\n        finally:\n            if subscriber {operator} None:\n                {cleanup}\n",
                        indent(prefix),
                    );
                    let source = if closure {
                        format!(
                            "def outer(subscriber):\n{}    return run\n",
                            indent(&source)
                        )
                    } else {
                        source
                    };
                    assert_matches_cpython(&source);
                }
            }
        }
    }
}
