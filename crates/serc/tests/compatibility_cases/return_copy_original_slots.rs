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
