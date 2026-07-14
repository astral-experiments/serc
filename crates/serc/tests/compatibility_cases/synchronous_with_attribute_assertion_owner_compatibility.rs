use crate::common;

#[test]
fn matches_cpython_marshal_for_synchronous_with_attribute_assertion_owners() {
    let source = r#"from __future__ import annotations


def protected_attribute_chain(manager, holder, flag):
    with manager:
        assert flag
        assert holder.calls[-1].request.url.path.endswith("/jobs/99/")


def protected_negated_attribute(manager, module, flag):
    with manager:
        assert flag
        assert not module.is_dtype([1, 2, 3])


def protected_slice_attribute(manager, holder, expected, flag):
    with manager:
        assert flag
        assert (holder.mmap[16:32, :] == expected).all()


def protected_attribute_after_fold(manager, holder, flag):
    with manager:
        assert flag
        parts = holder.path.split("/")
        directory = "/".join(parts[:-1])
        assert holder.ready(directory)


def multi_with_negated_attribute(first, second, third, module, flag):
    with first, second, third:
        assert flag
        assert not module.is_dtype([1, 2, 3])


def first_attribute_assert_stays_exact(manager, holder):
    with manager:
        assert holder.ready("value")


def ownerless_attribute_assert_stays_exact(manager, holder, flag):
    with manager:
        assert flag
        assert holder.ready("value")


def name_call_listcomp_stays_exact(manager, values, expected):
    with manager:
        assert len(values) == len(expected)
        assert all([value == item for value, item in zip(values, expected)])
"#;

    common::assert_matches_cpython_marshal(
        source,
        "synchronous_with_attribute_assertion_owners.py",
    );
}
