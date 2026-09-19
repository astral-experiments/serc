use crate::common;

#[test]
fn matches_cpython_cleanup_after_located_shared_predecessor() {
    for function in ["def", "async def"] {
        for prefix in ["", "    yield value\n"] {
            for looped in [false, true] {
                for body in [
                    "result = left if flag else right",
                    "result = left and right",
                    "if flag:\n    prepare(value)\nrecover(value)",
                ] {
                    let handler = format!(
                        "try:\n    work(value)\nexcept Exception as error:\n    if check(value):\n        {}\n    else:\n        raise\n",
                        body.replace('\n', "\n        ")
                    );
                    let handler = if looped {
                        format!(
                            "for value in values:\n    {}",
                            handler.replace('\n', "\n    ")
                        )
                    } else {
                        handler
                    };
                    let source = format!(
                        "{function} process(values, value, flag, left, right):\n{prefix}    {}\n    after(value)\n",
                        handler.replace('\n', "\n    ")
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "located_handler_cleanup_predecessors.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_original_handler_store_tail_donors() {
    for (filename, source) in [
        (
            "handler_triple_store_fallthrough.py",
            r#"def probe(table, consume, finish):
    try:
        first, second, third = table.split(".", 2)
    except ValueError:
        parts = table.split(".")
        if len(parts) == 3:
            first, second, third = parts
        elif len(parts) == 2:
            first, second, third = "", parts[0], parts[1]
        else:
            first, second, third = "", "", parts[0]
    consume(first, second, third)
    finish()
"#,
        ),
        (
            "handler_pair_store_fallthrough_control.py",
            r#"def probe(table, consume, finish):
    try:
        first, second = table.split(".", 1)
    except ValueError:
        parts = table.split(".")
        if len(parts) == 2:
            first, second = parts
        else:
            first, second = "", parts[0]
    consume(first, second)
    finish()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
