use crate::common;

#[test]
fn matches_cpython_value_match_failure_before_finally() {
    for suspension in ["", "await "] {
        let prefix = if suspension.is_empty() { "" } else { "async " };
        let source = format!(
            "{prefix}def choose(value):\n    try:\n        match value:\n            case Choices.first:\n                {suspension}first()\n            case Choices.second:\n                {suspension}second()\n    except Error as error:\n        raise OtherError() from error\n    finally:\n        {suspension}cleanup()\n"
        );
        common::assert_matches_cpython_marshal(&source, "value_match_finally.py");
    }
}
