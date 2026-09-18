use crate::common::assert_matches_cpython_marshal;

#[test]
fn normal_finally_pop_aliases_preserve_original_owners() {
    for (filename, source) in [
        (
            "boolean_join_five_calls.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "boolean_join_three_calls.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
            touch2()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "branch_join_five_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            if a:
                flag = b
            else:
                flag = a
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "folded_boolean_five_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = True and a
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn normal_finally_pop_aliases_keep_slot_boundaries() {
    for (filename, source) in [
        (
            "boolean_join_two_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a and b
            touch0()
            touch1()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
        (
            "direct_assignment_five_calls_control.py",
            r"def run(manager, a, b, process):
    with manager:
        try:
            flag = a
            touch0()
            touch1()
            touch2()
            touch3()
            touch4()
        finally:
            if process is not None:
                try:
                    process.close()
                except LookupError:
                    pass
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
