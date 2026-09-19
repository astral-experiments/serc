use crate::common;

#[test]
fn matches_cpython_normal_finally_for_body_owner_slots() {
    for (header, suspend) in [
        ("def", "work()"),
        ("def", "yield value"),
        ("async def", "await work()"),
        ("async def", "yield value"),
    ] {
        for condition in [
            "if obj.flag:",
            "if value is None:",
            "if not attr.endswith('x'):",
        ] {
            let source = format!(
                "{header} f(manager, value, obj, items):\n    with manager:\n        try:\n            {suspend}\n        finally:\n            for attr in items:\n                {condition}\n                    continue\n                consume(value)\n",
            );
            common::assert_matches_cpython_marshal(&source, "for_body_owner_slots.py");
        }
    }
}

#[test]
fn matches_cpython_for_body_target_nop_owner_without_manager() {
    for (header, suspend) in [
        ("def", "yield file"),
        ("async def", "await consume(file)"),
        ("async def", "yield file"),
    ] {
        for target in ["file", "file, other", "(\n        file, other\n    )"] {
            let source = format!(
                "{header} f(items):\n    for {target} in items:\n        if not file.endswith('x'):\n            continue\n        {suspend}\n",
            );
            common::assert_matches_cpython_marshal(&source, "for_body_target_nop_owner.py");
        }
    }
}
