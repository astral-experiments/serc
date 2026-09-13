use crate::common;

#[test]
fn matches_cpython_borrows_after_surviving_while_breaks() {
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let yielded = if scope >= 2 { "    yield result\n" } else { "" };
        let tail = if scope >= 2 {
            "consume(value, result)\n    yield result"
        } else {
            "return consume(value, result)"
        };
        for body in [
            "value = next_value()\nif flag:\n    break\nif other:\n    break",
            "value = next_value()\nif flag:\n    break\nif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nconsume(value)\nif done:\n    break",
            "value = next_value()\nif flag:\n    break\nif other:\n    continue\nbreak",
            "value = next_value()\nif flag:\n    if other:\n        break\n    continue\nconsume(value)\nif done:\n    break",
            "value = next_value()\nif flag:\n    break\nelif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nif flag:\n    break\nconsume(value)",
        ] {
            let source = format!(
                "{prefix}def function(flag, other, done, result, next_value):\n{yielded}    while True:\n        {}\n    {tail}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "surviving_while_break.py");
        }
    }
}

#[test]
fn matches_cpython_borrows_after_protected_surviving_while_breaks() {
    for (filename, source) in [
        (
            "protected_early_break_continue_call.py",
            r"def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
        (
            "protected_no_early_break_continue_call.py",
            r"def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
        (
            "protected_early_break_no_continue_call.py",
            r"def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
        (
            "protected_early_break_continue_call_async.py",
            r"async def collect(reader):
    items = []
    try:
        while True:
            ready, item = reader.read()
            if not ready:
                if reader.done():
                    break
                continue
            items.append(item)
            if reader.done():
                break
        count = reader.finish(items)
    finally:
        reader.close()
    return count
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn protected_while_name_return_preserves_small_exit_borrowing() {
    for (filename, source) in [
        (
            "protected_infinite_loop_return.py",
            r"def collect(factory):
    items = []
    try:
        reader = factory()
        while True:
            ready, item = reader.read()
            if ready:
                items.append(item)
            else:
                break
        return items
    except Exception as error:
        pass
",
        ),
        (
            "unprotected_infinite_loop_return.py",
            r"def collect(factory):
    items = []
    reader = factory()
    while True:
        ready, item = reader.read()
        if ready:
            items.append(item)
        else:
            break
    return items
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
