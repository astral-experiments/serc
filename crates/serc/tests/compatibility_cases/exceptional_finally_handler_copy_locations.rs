use crate::common;

fn enclose(source: &str, manager: &str, finalizer: bool) -> String {
    let body = source.replace('\n', "\n    ");
    if finalizer {
        format!("try:\n    {body}\nfinally:\n    finish()")
    } else {
        format!("{manager}\n    {body}")
    }
}

#[test]
fn matches_cpython_exceptional_finally_handler_copy_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        let manager = if function == "async def" {
            "async with manager:"
        } else {
            "with manager:"
        };
        for name in ["", " as error"] {
            for finalizer in [false, true] {
                for (nested, condition, following) in [
                    (false, "flag", ""),
                    (false, "True", ""),
                    (true, "flag", ""),
                    (false, "flag", "\nvalue = other"),
                    (false, "flag", "\nafter()"),
                ] {
                    for body in ["value = other", "work()"] {
                        let handler = format!(
                            "try:\n    work()\nexcept Exception{name}:\n    if {condition}:\n        {body}{following}"
                        );
                        let cleanup = if nested {
                            enclose(&handler, manager, finalizer)
                        } else {
                            handler
                        };
                        let protected = format!(
                            "try:\n    work()\nfinally:\n    {}",
                            cleanup.replace('\n', "\n    ")
                        );
                        let source = format!(
                            "{function} f(flag, ready, value, other, manager):\n{suspension}    {}\n",
                            enclose(&protected, manager, finalizer).replace('\n', "\n    ")
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "exceptional_finally_handler_copy_locations.py",
                        );
                    }
                }
            }
        }
    }
}
