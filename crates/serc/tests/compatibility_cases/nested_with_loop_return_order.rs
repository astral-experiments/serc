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
