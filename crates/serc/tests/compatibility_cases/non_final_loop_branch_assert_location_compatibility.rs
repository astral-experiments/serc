use crate::common;

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
            "negated_and_true_tail.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert not (item.done and True)
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "negated_or_false_tail.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert not (item.done or False)
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "positive_and_true_tail.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert item.done and True
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "positive_and_true_tail_message.py",
            r#"
def check(items):
    for item in items:
        if item.selected:
            assert item.done and True, "not done"
        else:
            raise ValueError(item)
    finish()
"#,
        ),
        (
            "positive_or_false_tail.py",
            r"
def check(items):
    for item in items:
        if item.selected:
            assert item.done or False
        else:
            raise ValueError(item)
    finish()
",
        ),
        (
            "positive_or_false_tail_message.py",
            r#"
def check(items):
    for item in items:
        if item.selected:
            assert item.done or False, "not done"
        else:
            raise ValueError(item)
    finish()
"#,
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

#[test]
fn matches_cpython_marshal_for_non_final_loop_conjunction_assert() {
    let cases = [
        (
            "nonfinal_loop_boolean_assert.py",
            r"def probe(items, outer, first, second, work):
    for item in items:
        if outer(item):
            assert first(item) and second(item)
        else:
            work(item)
",
        ),
        (
            "nonfinal_loop_single_assert_control.py",
            r"def probe(items, outer, first, second, work):
    for item in items:
        if outer(item):
            assert first(item)
        else:
            work(item)
",
        ),
        (
            "nonfinal_loop_ifexp_false_arm_control.py",
            r"def probe(items, outer, first, selector, work):
    for item in items:
        if outer(item):
            assert (first(item) if selector(item) else False)
        else:
            work(item)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_non_final_loop_disjunction_assert() {
    let cases = [
        (
            "nonfinal_loop_or_assert.py",
            r"def check_gradients(values):
    for left, right in values:
        if left is None:
            assert right is None or (right == 0).all(), right
        else:
            assert left.ready is None
            assert right.ready is None
            assert (left == right).all(), (left, right)
",
        ),
        (
            "nonfinal_loop_and_assert_control.py",
            r"def check_gradients(values):
    for left, right in values:
        if left is None:
            assert right is None and (right == 0).all(), right
        else:
            assert left.ready is None
            assert right.ready is None
            assert (left == right).all(), (left, right)
",
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
