use std::fmt::Write;

use crate::common;

#[test]
fn matches_cpython_copied_finally_prefix_alias_owners() {
    for (definition, prologue, outer) in [
        ("def", "", ""),
        ("async def", "", "async with manager"),
        ("def", "    yield value\n", "with manager"),
    ] {
        for handler in [false, true] {
            for has_else in [false, true] {
                for condition in ["value is None", "value is not None"] {
                    for expression in [
                        "result(k0=work(value), k1=work(value))",
                        "result(k0=value, k1=value, k2=value, k3=value, k4=value, k5=value)",
                    ] {
                        let mut source =
                            format!("{definition} f(value, flag, obj, manager):\n{prologue}");
                        let indent = if outer.is_empty() {
                            "    "
                        } else {
                            writeln!(source, "    {outer}:").unwrap();
                            "        "
                        };
                        writeln!(
                            source,
                            "{indent}try:\n{indent}    if flag:\n{indent}        work()"
                        )
                        .unwrap();
                        if has_else {
                            writeln!(source, "{indent}    else:\n{indent}        other()").unwrap();
                        }
                        writeln!(source, "{indent}    return {expression}").unwrap();
                        if handler {
                            writeln!(
                                source,
                                "{indent}except Error as error:\n{indent}    return {expression}"
                            )
                            .unwrap();
                        }
                        writeln!(source, "{indent}finally:\n{indent}    if {condition}:\n{indent}        cleanup(value)").unwrap();
                        common::assert_matches_cpython_marshal(
                            &source,
                            "copied_finally_prefix_alias_owners.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_normal_finally_prefix_alias_owners() {
    for (definition, tail) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for calls in [4, 5, 6] {
            let body = "            work()\n".repeat(calls);
            let source = format!(
                "{definition} f(value, enabled, manager):\n    with manager:\n        try:\n            if enabled:\n                work()\n            else:\n                other()\n{body}        finally:\n            if value:\n                close(value)\n{tail}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "normal_finally_prefix_alias_owners.py",
            );
        }
    }
}

#[test]
fn matches_cpython_copied_finally_prefix_alias_boundary_controls() {
    for definition in ["def", "async def"] {
        for body in [
            "        if flag:\n            work()\n            work()\n            work()\n",
            "        if flag:\n            work()\n            work()\n            work()\n        else:\n            other()\n            other()\n            other()\n",
        ] {
            for condition in ["value is not None", "value"] {
                let source = format!(
                    "{definition} f(value, flag):\n    try:\n{body}    finally:\n        if {condition}:\n            cleanup(value)\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "targeted_finally_prefix_alias_boundary.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_copied_finally_prefix_alias_setup_controls() {
    for definition in ["def", "async def"] {
        for finalbody in [
            "        try:\n            if value is not None:\n                cleanup(value)\n        except Error:\n            recover()\n",
            "        with manager:\n            if value is not None:\n                cleanup(value)\n",
        ] {
            let source = format!(
                "{definition} f(value, flag, manager):\n    try:\n        if flag:\n            work()\n        return result(k0=work(value), k1=work(value))\n    finally:\n{finalbody}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "setup_finally_prefix_alias_boundary.py",
            );
        }
    }
}
