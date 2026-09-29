use crate::common;

#[test]
fn matches_cpython_marshal_for_conditional_constant_tuple_exits() {
    for tail in ["", ", 0", ", 0, ''", ", 0, '', 3", ", 0, '', 3, 4"] {
        for expression in [
            "0 if flag else 1",
            "0 if flag else (1 if other else 2)",
            "(0 if other else 1) if flag else 2",
            "0 if flag and other else 1",
        ] {
            let source = format!("def f(flag, other):\n    return ({expression}{tail},)\n");
            common::assert_matches_cpython_marshal(&source, "conditional_constant_tuple_exits.py");
        }
    }
}

#[test]
fn folds_tuple_concatenations_with_repeated_operands() {
    for definition in ["def", "async def"] {
        for prefix in ["", "    'documentation'\n", "    consume('prefix')\n"] {
            for expression in [
                "(0.25,) * 5 + (0.5,) * 5",
                "(0.2,) * 2 + (0.4,) * 2 + (0.6,) * 2",
                "(1,) + (2,)",
                "() + (1, 2)",
                "(1,) * 256 + (2,) * 256",
                "(1,) * 257 + (2,)",
                "((1, 2),) + ((3, 4),)",
            ] {
                for body in [
                    format!("    return {expression}\n"),
                    format!(
                        "    if flag:\n        return {expression}\n    consume((0.25,) * 5)\n    return (0.5,) * 5\n"
                    ),
                    format!(
                        "    if flag:\n        value = {expression}\n    else:\n        value = (0.25,) * 5\n    return value\n"
                    ),
                ] {
                    let source = format!("{definition} f(flag):\n{prefix}{body}");
                    common::assert_matches_cpython_marshal(
                        &source,
                        "folded_tuple_concatenation.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_tuple_concatenation_operand_construction_order() {
    for (left, right, literal) in [
        ("(1,)", "(2,)", "(1, 2)"),
        ("(('left',),)", "(('right',),)", "(('left',), ('right',))"),
        ("(((),),)", "((),)", "(((),), ())"),
        ("()", "('right',)", "('right',)"),
        ("('left',)", "()", "('left',)"),
        ("(True,)", "(1,)", "(True, 1)"),
    ] {
        for prefix in ["", "    consume('prefix')\n"] {
            for reused in [left, right] {
                for preconstruct in [false, true] {
                    let early = if preconstruct {
                        format!("    consume({literal})\n")
                    } else {
                        String::new()
                    };
                    let source = format!(
                        "def f(flag):\n{prefix}{early}    if flag:\n        result = {left} + {right}\n    else:\n        result = {reused}\n    consume({left})\n    consume({right})\n    return result\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "tuple_concatenation_dependency_order.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_nested_tuple_construction_order_after_exit_copying() {
    for definition in ["def", "async def"] {
        for prefix in [
            "",
            "    'documentation'\n",
            "    consume('prefix')\n",
            "    if early:\n        return ('earlier',)\n",
        ] {
            for child in ["()", "(1, 2)", "('label',)", "(('inner',), ())"] {
                for parent in [format!("({child}, {child})"), format!("(None, {child})")] {
                    let source = format!(
                        "{definition} f(flag, early):\n{prefix}    if flag:\n        value = {parent}\n    else:\n        value = {child}\n    return value\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "nested_tuple_construction_order.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_constant_tuple_order_after_exit_copying() {
    for definition in ["def", "async def"] {
        for prefix in [
            "",
            "    'doc'\n",
            "    consume('before')\n",
            "    if early:\n        return '', 'early'\n",
        ] {
            for first in ["''", "0"] {
                for branch in [
                    "        if value:\n            return value, 'dynamic'\n",
                    "        if value:\n            return value, 'dynamic'\n        observe()\n",
                    "        return value, 'dynamic'\n",
                ] {
                    let source = format!(
                        "{definition} f(flag, value, early):\n{prefix}    if flag:\n{branch}    else:\n        return {first}, 'alternative'\n    return {first}, 'suffix'\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "copied_exit_constant_tuple_order.py",
                    );
                }
            }
        }
    }
}

#[test]
fn preserves_folded_numeric_intermediate_order_after_exit_copying() {
    for definition in ["def", "async def"] {
        for prefix in [
            "",
            "    'doc'\n",
            "    consume(key=value)\n",
            "    consume((1, 2))\n",
        ] {
            for body in [
                "    if -(1 << 63) <= value < (1 << 63):\n        return value\n    return f'number:{value:x}'\n",
                "    factors = {'y': 60*60*24*365, 'd': 60*60*24, 'h': 60*60}\n    unit = value[-1].lower()\n    digits = value[:-1]\n    if unit in factors and digits.isdigit():\n        result = int(digits) * factors[unit]\n    else:\n        raise ValueError(f'bad:{value}')\n    return result\n",
            ] {
                let source = format!("{definition} f(value):\n{prefix}{body}");
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_exit_numeric_constant_order.py",
                );
            }
        }
    }
}
