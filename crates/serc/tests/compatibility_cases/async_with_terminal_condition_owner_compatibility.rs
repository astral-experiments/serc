use crate::common;

#[test]
fn terminal_with_elif_markers_retain_compacted_owners() {
    for (manager, indent) in [
        ("with manager:", "        "),
        ("async with manager:", "        "),
        (
            "async with manager:\n        async with manager:",
            "            ",
        ),
    ] {
        for condition in [
            "isinstance(value, str)",
            "value is not None",
            "work(value)",
            "value",
        ] {
            for body in ["return value", "value = await work()"] {
                let source = format!(
                    r#"async def select(manager, value, work):
    {manager}
{indent}value = await work()
{indent}if value:
{indent}    {body}
{indent}elif {condition}:
{indent}    {body}
{indent}else:
{indent}    return ""
"#
                );
                common::assert_matches_cpython_marshal(&source, "terminal_with_elif_owners.py");
            }
        }
    }
}

#[test]
fn preserves_pass_markers_inside_protected_async_managers() {
    for suspension in ["", "    yield None\n"] {
        for cleanup in ["except Error:\n    recover()\n", "finally:\n    finish()\n"] {
            for loop_header in ["", "while enabled:", "for item in values:"] {
                for call in ["work()", "await work()"] {
                    let body = format!(
                        "try:\n    async with manager:\n        {call}\n        pass\n{cleanup}"
                    );
                    let body = if loop_header.is_empty() {
                        body
                    } else {
                        format!("{loop_header}\n    {}", body.replace('\n', "\n    "))
                    };
                    let source = format!(
                        "async def process(manager, enabled, values):\n{suspension}    {}\n",
                        body.replace('\n', "\n    ")
                    );
                    common::assert_matches_cpython_marshal(&source, "protected_async_with_pass.py");
                }
            }
        }
    }
}
