#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_non_final_loop_branch_assert_locations() {
    let cases = [
        (
            "negated_attribute.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert not item.done
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "negated_attribute_message.py",
            r#"
def check(items):
    for item in items:
        if item.selected:
            assert not item.done, "already done"
        else:
            raise ValueError(item)
    finish()
"#,
        ),
        (
            "positive_attribute_control.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert item.done
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "non_tail_control.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert not item.done
            consume(item)
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "outside_loop_control.py",
            r"
def check(item):
    if item.selected:
        assert not item.done
    else:
        raise ValueError(item)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
