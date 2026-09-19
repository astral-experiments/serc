use crate::common;

#[test]
fn iterator_pass_return_borrow_matches_cpython() {
    let cases = [
        (
            "iterator_pass_return_target.py",
            r"def collect(load, walk, report):
    values = []
    try:
        root = load()
        if root:
            for value in walk(root):
                values.append(value)
        else:
            pass
    except Exception as error:
        report(error)
        return []

    return values
",
        ),
        (
            "iterator_pass_return_control.py",
            r"def collect(load, walk, report):
    values = []
    try:
        root = load()
        if root:
            for value in walk(root):
                values.append(value)
        else:
            report(root)
    except Exception as error:
        report(error)
        return []

    return values
",
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
