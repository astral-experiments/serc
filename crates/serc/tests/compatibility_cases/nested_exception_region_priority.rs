use crate::common;

#[test]
fn matches_cpython_threaded_normal_exit_owners() {
    for prefix in ["", "async "] {
        for following in ["after()", "return value", "return None"] {
            let source = format!(
                "{prefix}def f(manager, value):
    try:
        start()
    except OuterError:
        try:
            with manager:
                work()
        except Error:
            recover()
        {following}
"
            );
            common::assert_matches_cpython_marshal(&source, "threaded_normal_exit_owners.py");
        }
    }
}

#[test]
fn matches_cpython_nested_exception_regions_after_loop_exclusions() {
    for padding in [0, 3, 20] {
        for body_size in [0, 3, 20] {
            let before = "        prepare()\n".repeat(padding);
            let body = "                use(value)\n".repeat(body_size);
            let source = format!(
                "def f(manager, read, use, prepare):
    try:
{before}        with manager:
            while True:
                value = read()
                if not value:
                    break
{body}                use(value)
    except OSError:
        raise ValueError from None
"
            );
            common::assert_matches_cpython_marshal(&source, "nested_region_loop_exclusions.py");
        }
    }
}
