use crate::common;

#[test]
fn matches_cpython_comprehension_temporary_name_preregistration() {
    for source in [
        "[i if True else 0 for i in values]\n",
        "[0 if True else i for i in values]\n",
        "{i: i if True else (other := i) for i in values}\nprint(other)\n",
        "class C:\n    items = [i if True else 0 for i in values]\n",
    ] {
        common::assert_matches_cpython_marshal(source, "comprehension_temporary_names.py");
    }
}

#[test]
fn preserves_cpython_comprehension_temporary_name_boundaries() {
    for source in [
        "[i for i in values]\n",
        "def f(values):\n    return [i if True else 0 for i in values]\n",
        "[i.i if True else 0 for i in values]\n",
        "[global_name if True else 0 for i in values]\n",
    ] {
        common::assert_matches_cpython_marshal(
            source,
            "comprehension_temporary_name_boundaries.py",
        );
    }
}
