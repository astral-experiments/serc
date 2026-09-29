use crate::common;

#[test]
fn matches_cpython_copied_nested_try_exit_boundaries() {
    for (filename, recovery) in [
        ("copied_nested_try_exit.py", "recover(value)"),
        ("copied_nested_try_return.py", "return None"),
    ] {
        let source = format!(
            r"def process(flag, value):
    try:
        if flag:
            option = value
        else:
            try:
                consume(value)
            except TypeError:
                raise ValueError from None
            finish(value)
            return None
        if option == 0:
            if value:
                consume(value)
            else:
                raise ValueError
        elif option == 1:
            if value:
                consume(value)
            else:
                raise ValueError
    except BaseException:
        {recovery}
    return None
"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}
