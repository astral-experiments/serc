use crate::common;

#[test]
fn reserves_terminal_if_conditions_before_reused_body_constants() {
    for condition in [
        "True", "False", "None", "256", "'text'", "b'text'", "f'text'",
    ] {
        let source = format!(
            "def function():\n    'docstring'\n    if {condition}:\n        emit('middle', {condition})\n    else:\n        emit('middle', {condition})\n"
        );
        common::assert_matches_cpython_marshal(&source, "terminal_constant_condition.py");
    }
}

#[test]
fn reserves_folded_conditional_expression_tests_before_selected_values() {
    for condition in [
        "True",
        "False",
        "2.5",
        "''",
        "'token'",
        r"'\ud800'",
        r"f'\ud800'",
    ] {
        for returned in [false, true] {
            let body = if returned {
                format!(
                    "return make('middle', {condition}) if {condition} else make('middle', {condition})"
                )
            } else {
                format!("result = left if {condition} else right\n    emit('middle', {condition})")
            };
            let source = format!("def function(left, right):\n    'docstring'\n    {body}\n");
            common::assert_matches_cpython_marshal(&source, "folded_expression_condition.py");
        }
    }
}

#[test]
fn preserves_surrogate_condition_values_during_constant_admission() {
    for condition in [r"'\ud800'", r"f'\ud800'"] {
        let source = format!(
            "def function(left, right):\n    emit('anchor')\n    result = left if {condition} else right\n    emit('middle')\n    return '\\ufffd'\n"
        );
        common::assert_matches_cpython_marshal(&source, "surrogate_condition_constant.py");
    }
}
