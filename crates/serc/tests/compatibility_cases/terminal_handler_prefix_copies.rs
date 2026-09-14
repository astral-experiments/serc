use crate::common;

#[test]
fn matches_cpython_terminal_handler_conditionals_after_prefixes() {
    for mode in ["sync", "async", "generator"] {
        for capture in ["", " as error"] {
            for prefix in ["report(value)", "value = transform(value)"] {
                for condition in ["flag", "value is None"] {
                    for body in ["report(value)", "import math\n            report(math)"] {
                        let definition = if mode == "async" { "async def" } else { "def" };
                        let suspension = if mode == "generator" {
                            "    yield value\n"
                        } else {
                            ""
                        };
                        let source = format!(
                            "{definition} f(value, flag):\n{suspension}    try:\n        observe(value)\n    except Exception{capture}:\n        {prefix}\n        if {condition}:\n            {body}\n"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "terminal_handler_prefix_copies.py",
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn matches_cpython_terminal_named_handler_manager_exit_copies() {
    common::assert_matches_cpython_marshal(
        r"def probe(manager, read, write, recover):
    try:
        with manager():
            with manager():
                with manager():
                    while True:
                        chunk = read()
                        if not len(chunk):
                            break
                        write(chunk)
    except Exception as error:
        if recover(error):
            with manager():
                with manager():
                    with manager():
                        while True:
                            chunk = read()
                            if not len(chunk):
                                break
                            write(chunk)
                write(None)
",
        "manager_nested_loop_exit.py",
    );
}
