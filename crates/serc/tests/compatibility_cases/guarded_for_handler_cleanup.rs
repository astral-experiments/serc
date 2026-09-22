use crate::common;

#[test]
fn guarded_for_handler_cleanup_copies_break_exit() {
    common::assert_matches_cpython_marshal(
        r"def convert(values, items):
    for value in values:
        try:
            found = lookup(value)
        except TypeError:
            found = None
            if value in unhashable or value in hashable:
                for item in items:
                    if item.value == value:
                        found = item
                        break
        consume(found)
",
        "guarded_for_handler_cleanup.py",
    );
}
