use crate::common;

#[test]
fn matches_cpython_marshal_for_protected_non_final_iterator_branches() {
    let source = r"def inside_finally(items, first, second, third, manager):
    try:
        for item in items:
            if first:
                if second:
                    value = 1
                    if third:
                        process()
                else:
                    alternate()
            else:
                final()
    finally:
        cleanup()


def conjunction_in_except(items, first, second, third, manager):
    try:
        for item in items:
            if first:
                if second:
                    value = 1
                    if first and third:
                        process()
                else:
                    alternate()
            else:
                final()
    except Error:
        cleanup()


def disjunction_in_with(items, first, second, third, manager):
    with manager:
        for item in items:
            if first:
                if second:
                    value = 1
                    if first or third:
                        process()
                else:
                    alternate()
            else:
                final()


def generator(items, first, second, third, manager):
    for item in items:
        if first:
            if second:
                value = 1
                if third():
                    process()
            else:
                alternate()
        else:
            final()
    yield None
";
    common::assert_matches_cpython_marshal(source, "protected_non_final_iterator_branches.py");
}

#[test]
fn matches_cpython_marshal_for_nested_elif_branches() {
    let source = r"def case_0(items, first, second, third, manager):
    with manager:
        for item in items:
            if first:
                if second:
                    value = 1
                    if previous:
                        previous_action()
                    elif first and third:
                        process()
                else:
                    alternate()
            else:
                final()


def case_1(items, first, second, third, manager):
    try:
        for item in items:
            if first:
                if second:
                    value = 1
                    if previous:
                        previous_action()
                    elif first or third:
                        process()
                else:
                    alternate()
            else:
                final()
    except Error:
        cleanup()
";
    common::assert_matches_cpython_marshal(source, "nested_elif_branches.py");
}

#[test]
fn matches_cpython_marshal_for_compacted_nested_iterator_condition_owners() {
    let source = r"def case_0(items, first, second, third, manager):
    try:
        for row in items:
            for item in row:
                if first:
                    choice = factory()
                    if choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()
    finally:
        cleanup()


def case_1(items, first, second, third, manager):
    try:
        for row in items:
            for item in row:
                if first:
                    choice = factory()
                    if not choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()
    except Error:
        cleanup()


def case_2(items, first, second, third, manager):
    with manager:
        for row in items:
            for item in row:
                if first:
                    choice = second()
                    if choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()


def case_3(items, first, second, third, manager):
    try:
        for row in items:
            for item in row:
                if first:
                    choice = second()
                    if not choice:
                        if third:
                            process()
                    else:
                        alternate()
                else:
                    final()
    finally:
        cleanup()
    yield None
";
    common::assert_matches_cpython_marshal(source, "compacted_nested_iterator_condition_owners.py");
}

#[test]
fn matches_cpython_marshal_for_nested_loop_tail_conditions_with_pass_handler() {
    let source = r"def nested_conditions(items, first, second, third):
    for item in items:
        try:
            if first and second:
                value = 1
                if second and third:
                    value = 2
                    if first and third:
                        value = operation()
        except Error:
            pass
";
    common::assert_matches_cpython_marshal(
        source,
        "nested_loop_tail_conditions_with_pass_handler.py",
    );
}
