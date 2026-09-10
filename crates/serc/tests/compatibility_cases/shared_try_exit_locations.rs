use crate::common;

#[test]
fn matches_cpython_shared_try_exit_locations() {
    for depth in 1..=4 {
        for action in ["result = value", "consume(value)", "return value"] {
            let mut body = action.to_owned();
            for _ in 0..depth {
                body = format!("if predicate(value):\n    {}", body.replace('\n', "\n    "));
            }
            for in_handler in [false, true] {
                for successor in [
                    "consume(value)\nreturn value",
                    "return value",
                    "return False, value",
                ] {
                    let mut body = format!(
                        "try:\n    {}\nexcept Error:\n    pass\n{successor}",
                        body.replace('\n', "\n    "),
                    );
                    if in_handler {
                        body = format!(
                            "try:\n    consume(value)\nexcept Failure as exc:\n    {}",
                            body.replace('\n', "\n    "),
                        );
                    }
                    let source = format!("def f(value):\n    {}\n", body.replace('\n', "\n    "));
                    common::assert_matches_cpython_marshal(&source, "shared_try_exit_locations.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_shared_async_try_exit_locations() {
    for action in ["consume(result)", "value = result"] {
        let source = format!(
            "async def f(value):\n    if value:\n        try:\n            result = await produce()\n            if result:\n                {action}\n        except Error:\n            consume(value)\n    else:\n        consume(value)\n    return value\n"
        );
        common::assert_matches_cpython_marshal(&source, "shared_async_try_exit_locations.py");
    }
}

#[test]
fn matches_cpython_guarded_match_async_comprehension_try_exit() {
    let source = r"async def probe(values, stream, permit):
    for value in values:
        try:
            match value:
                case 0:
                    result = [item async for item in stream]
                case Kind.VALUE if permit:
                    result = [item async for item in stream]
        except Error:
            recover()
";
    common::assert_matches_cpython_marshal(source, "guarded_match_async_try_exit.py");
}
