use crate::common;

#[test]
fn matches_cpython_copied_match_guard_subject_locations() {
    common::assert_matches_cpython_marshal(
        "def f(value):\n    match value:\n        case number if 0 < number < 10:\n            return Result.OK\n        case number if number == 20:\n            return Result.OTHER\n        case _:\n            return Result.FALLBACK\n",
        "copied_match_guard_subject.py",
    );
}

#[test]
fn matches_cpython_guarded_capture_match_failure_footer() {
    common::assert_matches_cpython_marshal(
        "def f(flag, other, value):\n    if flag:\n        match value:\n            case number if number > 1:\n                return Result.OK\n            case number if number <= 1:\n                return Result.OTHER\n    elif other:\n        return Result.ELSE\n    return Result.FALLBACK\n",
        "guarded_capture_match_footer.py",
    );
}
