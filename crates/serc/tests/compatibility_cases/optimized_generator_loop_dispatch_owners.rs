use crate::common;

#[test]
fn matches_cpython_marshal_for_optimized_generator_try_dispatch_slots() {
    for callable in ["any", "all"] {
        let source = format!(
            r"def prior_call(values):
    try:
        prepare()
        return {callable}(value for value in values)
    except Error:
        pass


def prior_unpack(values, pair):
    try:
        left, right = pair
        return {callable}(value for value in values)
    except Error:
        pass


def prior_comprehension(values):
    try:
        mapping = {{value: value for value in values}}
        return {callable}(value for value in values)
    except Error:
        pass


def first_guard_control(values):
    try:
        return {callable}(value for value in values)
    except Error:
        pass
"
        );
        common::assert_matches_cpython_marshal(
            &source,
            "optimized_generator_try_dispatch_slots.py",
        );
    }
}

#[test]
fn generator_condition_after_captured_comprehension_and_yield() {
    for annotation in ["", " -> None"] {
        for captured in ["values", "source, values"] {
            for condition in ["state.ready()", "ready()"] {
                let source = format!(
                    "def sample():
    source = data
    values = {{item.value for item in source.items()}}

    def captured(){annotation}:
        return {captured}

    state.callback = captured

    try:
        yield
        if not {condition}:
            pass
    finally:
        pass
"
                );
                common::assert_matches_cpython_marshal(&source, "captured_yield_condition.py");
            }
        }
    }
}
