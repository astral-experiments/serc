use crate::common;

#[test]
fn matches_cpython_named_handler_first_truth_owner_and_new_arrays() {
    for (filename, source) in [
        (
            "named_handler_name_outerTrue.py",
            r"def f(warn, flag):
    try:
        if exists():
            try:
                if check():
                    remove()
                elif other():
                    unlink()
            except Error as e:
                if warn:
                    from module import Display
                    display = Display()
                    display(e)
    except Error:
        pass
",
        ),
        (
            "named_handler_name_outerFalse.py",
            r"def f(warn, flag):
    try:
        if check():
            remove()
        elif other():
            unlink()
    except Error as e:
        if warn:
            from module import Display
            display = Display()
            display(e)
",
        ),
        (
            "named_handler_later_truth.py",
            r"def f(warn, flag, manager):
    try:
        work()
    except Error as e:
        if flag:
            work(e)
        if warn:
            other(e)
",
        ),
        (
            "named_handler_while_truth.py",
            r"def f(warn, flag, manager):
    try:
        work()
    except Error as e:
        while warn:
            work(e)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
