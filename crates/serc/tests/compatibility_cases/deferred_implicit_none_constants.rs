use crate::common;

#[test]
fn matches_cpython_marshal_for_deferred_implicit_none_constants() {
    for source in [
        "def f(flag):\n    if flag:\n        try:\n            call(first=1)\n        finally:\n            call(final=2)\n    else:\n        call(last=3)\n",
        "def f(flag):\n    call(first=1)\n    try:\n        while True:\n            if flag:\n                break\n            call(last=3)\n            if flag is None:\n                call()\n    except Error:\n        call()\n",
    ] {
        common::assert_matches_cpython_marshal(source, "deferred_implicit_none_constants.py");
    }
}

#[test]
fn matches_cpython_marshal_for_none_patterns_after_comprehensions() {
    for source in [
        "def f(values, subject):\n    values = [x for x in values]\n    match subject:\n        case None:\n            pass\n    return values\n",
        "def f(values):\n    values = [x for x in values]\n    for value in values:\n        match value:\n            case 1:\n                call(value)\n            case None:\n                pass\n    return values\n",
        "def f(values):\n    for value in values:\n        match value:\n            case 1:\n                call(value)\n            case None:\n                pass\n        call(after=1)\n    return values\n",
    ] {
        common::assert_matches_cpython_marshal(source, "none_patterns_after_comprehensions.py");
    }
}
