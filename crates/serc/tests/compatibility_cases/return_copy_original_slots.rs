use crate::common;

#[test]
fn matches_cpython_return_copies_after_generator_branches() {
    for definition in ["def", "async def"] {
        for prefix in [
            "",
            "    with manager:\n        observe()\n",
            "    for item in items:\n        observe(item)\n",
        ] {
            for expression in [
                "tuple(item for item in value)",
                "tuple(item.copy() for item in value)",
            ] {
                let source = format!(
                    "{definition} f(flag, other, ready, value, items, manager):
{prefix}    if flag:
        if other:
            result = None
        elif ready:
            result = {expression}
        else:
            result = value
    else:
        result = value
    return result
"
                );
                common::assert_matches_cpython_marshal(&source, "return_copy_jump_slots.py");
            }
        }
    }
}

#[test]
fn keeps_direct_nested_handler_returns() {
    for definition in ["def", "async def"] {
        let source = format!(
            "{definition} collect():
    try:
        value = build()
    except Exception:
        try:
            value = build()
        except Exception as error:
            return None
    return value
"
        );
        common::assert_matches_cpython_marshal(&source, "direct_nested_handler_return.py");
    }
}

#[test]
fn keeps_authored_return_locations_after_copied_keyword_calls() {
    use std::fmt::Write as _;

    for (filename, protected, large, conditional) in [
        (
            "protected_large_keyword_return_conditional.py",
            true,
            true,
            true,
        ),
        (
            "protected_large_keyword_return_linear_control.py",
            true,
            true,
            false,
        ),
        (
            "protected_short_keyword_return_conditional_control.py",
            true,
            false,
            true,
        ),
        (
            "unprotected_large_keyword_return_conditional_control.py",
            false,
            true,
            true,
        ),
    ] {
        let indent = if protected {
            "            "
        } else {
            "        "
        };
        let keywords = if large {
            let mut keywords = String::new();
            for index in 0..36 {
                writeln!(keywords, "{indent}field{index:02}=value,").unwrap();
            }
            keywords
        } else {
            format!("{indent}initial=value,\n")
        };
        let final_value = if conditional {
            "(value if flag else None)"
        } else {
            "value"
        };
        let (prefix, suffix) = if protected {
            (
                "    try:\n        return factory(\n",
                "        )\n    except ValueError:\n        raise\n",
            )
        } else {
            ("    return factory(\n", "    )\n")
        };
        let source = format!(
            "def target(factory, flag, value):\n{prefix}{keywords}{indent}final={final_value},\n{suffix}"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}

#[test]
fn matches_cpython_final_literal_attribute_assignment_copy_budget() {
    for marker in ["'attribute documentation'", "42", "None", "True", "pass"] {
        for constants in [1, 2] {
            let markers = format!("        {marker}\n").repeat(constants);
            let source = format!(
                "class Container:\n    def __init__(self, values=None):\n        self.values = values[:] if values else []\n{markers}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "final_literal_attribute_copy_budget.py",
            );
        }
    }
}
