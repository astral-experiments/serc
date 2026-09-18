use crate::common;

#[test]
fn matches_cpython_nested_with_loop_return_order() {
    for (header, prefix, manager, iterator) in [
        ("def", "", "with", "for"),
        ("async def", "", "async with", "async for"),
        ("def", "    yield value\n", "with", "for"),
    ] {
        for pattern in ["WFWF", "FWFW", "WFWFWF"] {
            for value in ["value", "None"] {
                let mut body = format!("return {value}\n");
                for (depth, kind) in pattern.bytes().enumerate().rev() {
                    let nested = indent(&body);
                    body = if kind == b'W' {
                        format!("{manager} manager() as bound_{depth}:\n{nested}")
                    } else {
                        format!("{iterator} item_{depth} in items:\n{nested}")
                    };
                }
                let body = indent(&body);
                let source = format!("{header} f(items, manager, value):\n{prefix}{body}");
                common::assert_matches_cpython_marshal(&source, "nested_with_loop_return_order.py");
            }
        }
    }
}

#[test]
fn exits_nested_managers_before_discarding_surrounding_iterators() {
    let source = r"events = []
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append(('enter', self.name))
        return self
    def __exit__(self, *args):
        events.append(('exit', self.name))

def f():
    with Manager('outer'):
        for item in [1, 2]:
            with Manager('inner'):
                for other in [3, 4]:
                    return item + other

print(f())
print(events)
";
    common::assert_executes(
        source,
        "4\n[('enter', 'outer'), ('enter', 'inner'), ('exit', 'inner'), ('exit', 'outer')]\n",
    );
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
fn matches_cpython_with_return_iterator_exception_owners() {
    for (filename, source) in [
        (
            "with_outer_iterator_try_finally.py",
            r"def f(manager, items, done):
    try:
        for item in items:
            try:
                with manager:
                    return item
            except ValueError:
                pass
    finally:
        if done:
            done()
",
        ),
        (
            "with_two_outer_iterators_try_finally.py",
            r"def f(manager, items, done):
    try:
        for item in items:
            try:
                for other in items:
                    with manager:
                        return item
            except ValueError:
                pass
    finally:
        if done:
            done()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn exits_async_managers_before_retry_iterator_in_finally_return() {
    for (filename, source) in [
        (
            "async_manager_inside_retry_return_finally.py",
            r"async def f(items, manager, value, cleanup):
    try:
        for item in items:
            async with manager:
                return value
    finally:
        cleanup()
",
        ),
        (
            "async_managers_around_retry_return_finally.py",
            r"async def f(items, outer, inner, value, cleanup):
    try:
        async with outer:
            for item in items:
                async with inner:
                    return value
    finally:
        cleanup()
",
        ),
        (
            "async_nested_managers_retry_await_return_finally.py",
            r"async def f(items, outer, inner, work, cleanup):
    try:
        for item in items:
            async with outer:
                async with inner:
                    return await work()
    finally:
        cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
