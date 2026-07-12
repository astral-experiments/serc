#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_first_iterator_if_inside_sync_with() {
    let source = r"def nested_else(manager, items, shifted):
    with manager:
        for item in items:
            if shifted:
                first(item)
            else:
                if ready(item):
                    second(item)

def flat_else(manager, items, shifted):
    with manager:
        for item in items:
            if shifted:
                first(item)
            else:
                second(item)

def following(manager, items, shifted):
    with manager:
        for item in items:
            if shifted:
                first(item)
            else:
                if ready(item):
                    second(item)
            after(item)

def outside(items, shifted):
    for item in items:
        if shifted:
            first(item)
        else:
            if ready(item):
                second(item)
";
    common::assert_matches_cpython_marshal(source, "sync_with_first_iterator_if_compatibility.py");
}
