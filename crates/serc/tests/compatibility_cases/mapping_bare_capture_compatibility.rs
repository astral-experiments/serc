use crate::common;

#[test]
fn matches_cpython_marshal_for_guarded_final_mapping_capture() {
    let source = r#"def guarded_final(subject, use):
    match subject:
        case {"skip": True}:
            return []
        case {"key": value} if use(value):
            return value
        case _:
            return []

def guarded_single(subject, use):
    match subject:
        case {"key": value} if use(value):
            return value
        case _:
            return []
"#;
    common::assert_matches_cpython_marshal(source, "mapping_bare_capture_compatibility.py");
}

#[test]
fn preserves_mapping_literal_key_registration_order() {
    for prefix in ["", "    'documentation'\n", "    consume('prefix')\n"] {
        for key in ["'item'", "512", "-512", "3.5", "1+2j", "b'item'"] {
            for between in ["", "    consume('between')\n"] {
                let source = format!(
                    "def f(subject):\n{prefix}    match subject:\n        case {{{key}: found, **rest}}:\n            return subject\n{between}    return {key}\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "mapping_literal_key_registration_order.py",
                );
            }
        }
    }
}

#[test]
fn folds_capture_rotation_after_multiline_mapping_checks() {
    let source = r#"def f(subject):
    match subject:
        case {
            "kind": "target",
            "value": value,
        }:
            return value
        case _:
            return None
"#;
    common::assert_matches_cpython_marshal(source, "multiline_mapping_capture_rotation.py");
}

#[test]
fn folds_guarded_capture_rotation_after_mapping_checks() {
    let source = r#"def f(subject, accept):
    match subject:
        case {
            "kind": "target",
            "value": value,
        } if accept(value):
            return value
        case _:
            return None
"#;
    common::assert_matches_cpython_marshal(source, "guarded_mapping_capture_rotation.py");
}

#[test]
fn folds_each_mapping_capture_rotation_in_a_match() {
    let source = r#"def f(subject, consume):
    match subject:
        case {
            "kind": "first",
            "value": value,
        }:
            consume(value)
        case {
            "kind": "second",
            "value": value,
        }:
            consume(value)
        case _:
            return None
    return value
"#;
    common::assert_matches_cpython_marshal(source, "repeated_mapping_capture_rotations.py");
}

#[test]
fn mapping_global_capture_keeps_nonlocal_store_order() {
    let source = r#"def f(subject):
    global value
    match subject:
        case {
            "kind": "target",
            "value": value,
        }:
            return value
        case _:
            return None
"#;
    common::assert_matches_cpython_marshal(source, "mapping_global_capture_order.py");
}

#[test]
fn mapping_cell_capture_keeps_nonlocal_store_order() {
    let source = r#"def f(subject):
    def capture():
        return value
    match subject:
        case {
            "kind": "target",
            "value": value,
        }:
            return capture()
        case _:
            return None
"#;
    common::assert_matches_cpython_marshal(source, "mapping_cell_capture_order.py");
}

#[test]
fn sequence_capture_preserves_separate_wildcard_locations() {
    let source = r"def f(subject):
    match subject:
        case [
            value,
            _,
            _,
        ]:
            return value
        case _:
            return None
";
    common::assert_matches_cpython_marshal(source, "sequence_capture_wildcard_locations.py");
}

#[test]
fn protected_mapping_capture_preserves_following_marker_owners() {
    let source = r#"def f(manager, subject, accept, consume):
    with manager:
        match subject:
            case {
                "kind": "target",
                "value": value,
            }:
                if accept(value):
                    consume(value)
            case _:
                consume(None)
"#;
    common::assert_matches_cpython_marshal(source, "protected_mapping_capture_markers.py");
}

#[test]
fn protected_single_line_mapping_capture_preserves_two_vacated_slots() {
    let source = r#"def f(manager, subject, accept, consume):
    with manager:
        match subject:
            case {"kind": "target", "value": value}:
                if accept(value):
                    consume(value)
            case _:
                consume(None)
"#;
    common::assert_matches_cpython_marshal(source, "single_line_mapping_capture_markers.py");
}

#[test]
fn protected_multiple_mapping_captures_keep_longer_rotation_order() {
    let source = r#"def f(manager, subject, accept, consume):
    with manager:
        match subject:
            case {
                "kind": "target",
                "first": first,
                "last": last,
            }:
                if accept(first):
                    consume(first, last)
            case _:
                consume(None, None)
"#;
    common::assert_matches_cpython_marshal(source, "multiple_mapping_capture_markers.py");
}

#[test]
fn folds_class_capture_rotations_after_unpack() {
    let source = r#"def single(subject):
    match subject:
        case {"value": str(value)}:
            return value
    return None


def pair(subject):
    match subject:
        case {"left": str(left), "right": int(right)}:
            return left, right
    return None


def pair_as(subject):
    match subject:
        case {"left": str() as left, "right": str() as right}:
            return left, right
    return None
"#;
    common::assert_matches_cpython_marshal(source, "mapping_class_capture_rotation.py");
}

#[test]
fn folds_nested_mapping_capture_rotations_with_line_boundaries() {
    let source = r#"def target(subject):
    match subject:
        case {"left": {"value": left}, "right": {"value": right}}:
            return left, right
    return None


def different_cleanup_lines(left, right):
    match left, right:
        case (
            {"value": str(first)},
            {
                "value": str(second),
                "extra": str(third),
            },
        ):
            return first, second, third
    return None
"#;
    common::assert_matches_cpython_marshal(source, "nested_mapping_capture_rotation.py");
}

#[test]
fn folds_sequence_capture_rotations_after_unpack() {
    let source = r#"def target(subject):
    match subject:
        case {
            "first": str(first),
            "second": str(second),
            "third": str(third),
            "last": [str(last)],
        } if first and last:
            return first, second, third, last
    return None
"#;
    common::assert_matches_cpython_marshal(source, "mapping_sequence_capture_rotation.py");
}

#[test]
fn folds_same_line_capture_rotations_before_guards_and_bodies() {
    for (source, filename) in [
        (
            r"def f(subject):
    match subject:
        case (captured, _, _):
            return captured
    return None
",
            "sequence_capture_two_discards.py",
        ),
        (
            r"def f(manager, subject, consume):
    with manager:
        match subject:
            case (captured, _, _):
                if captured is None:
                    consume(captured)
            case _:
                consume(None)
",
            "protected_sequence_capture_marker.py",
        ),
        (
            r#"def f(subject):
    match subject:
        case ("red" | "blue") as captured, _, _:
            return captured
    return None
"#,
            "or_capture_two_discards.py",
        ),
        (
            r"def f(subject):
    match subject:
        case list() as captured, _, _ if captured:
            return captured
    return None
",
            "class_capture_two_discards_guard.py",
        ),
        (
            r#"def f(subject, convert):
    match subject:
        case {"value": captured}:
            try:
                return convert(captured)
            except ValueError:
                return None
    return None
"#,
            "mapping_capture_try_body.py",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nested_mapping_capture_guard_cleanup() {
    let cases: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("fixtures/nested_mapping_capture_guards.json"))
            .expect("public nested capture cases are filename-to-source strings");
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(&source, &filename);
    }
}

#[test]
fn matches_cpython_high_local_mapping_capture_guards() {
    let cases: std::collections::BTreeMap<String, String> = serde_json::from_str(include_str!(
        "fixtures/mapping_capture_high_local_guards.json"
    ))
    .expect("public high-local capture cases are filename-to-source strings");
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(&source, &filename);
    }
}

#[test]
fn matches_cpython_complete_capture_cleanup_rotations() {
    for (filename, source) in [
        (
            "nested_star_capture_cleanup_target.py",
            r#"def f(value):
    match value:
        case {"outer": {"inner": [*items]}}:
            return items
    return None
"#,
        ),
        (
            "nested_star_or_capture_cleanup_target.py",
            r#"def f(value):
    match value:
        case {"outer": {"inner": [*items]}} | {"inner": [*items]}:
            return items
    return None
"#,
        ),
        (
            "class_wildcard_capture_cleanup_target.py",
            r"def f(value):
    match value:
        case Event(at=_, message=message, exception=_, request_id=_, raw=_):
            return message
    return None
",
        ),
        (
            "multiline_mapping_three_capture_cleanup_target.py",
            r#"def f(value):
    match value:
        case {
            "first": first,
            "second": second,
            "flag": False,
            "last": last,
        } if isinstance(first, str):
            return first, second, last
    return None
"#,
        ),
        (
            "protected_nested_star_capture_owner_target.py",
            r#"def f(value, finish):
    try:
        match value:
            case {"outer": {"inner": [*items]}}:
                if items is None:
                    return None
                return items
        return None
    finally:
        finish()
"#,
        ),
        (
            "protected_multiline_capture_owner_target.py",
            r#"def f(value, finish):
    try:
        match value:
            case {
                "first": first,
                "second": second,
                "flag": False,
                "last": last,
            } if first is not None:
                return first, second, last
        return None
    finally:
        finish()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_mapping_capture_before_try_boundaries() {
    for (filename, source) in [
        (
            "mapping_capture_before_try_target.py",
            r#"def f(subject, convert):
    match subject:
        case {"outer": {"inner": captured}}:
            try:
                result = convert(captured)
            except ValueError:
                result = captured
            return result
    return None
"#,
        ),
        (
            "mapping_capture_before_call_control.py",
            r#"def f(subject, convert):
    match subject:
        case {"outer": {"inner": captured}}:
            convert(captured)
            try:
                result = convert(captured)
            except ValueError:
                result = captured
            return result
    return None
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn folds_single_multiline_mapping_capture_with_one_retained_line() {
    for pattern in [
        "{\n        \"key\": captured\n    }",
        "{\"key\": (\n        captured\n    )}",
    ] {
        let matched = format!(
            "match subject:\n    case {pattern}:\n        if captured is None:\n            consume(None)\n        else:\n            consume(captured)\n    case _:\n        consume(None)\n"
        );
        for body in [
            matched.clone(),
            format!(
                "try:\n{}except Error:\n    consume(None)\n",
                common::indent(&matched)
            ),
            format!("try:\n{}finally:\n    finish()\n", common::indent(&matched)),
            format!("with manager:\n{}", common::indent(&matched)),
        ] {
            let source = format!(
                "def f(subject, consume, manager, finish):\n{}",
                common::indent(&body)
            );
            common::assert_matches_cpython_marshal(&source, "single_multiline_mapping_capture.py");
        }
    }
}
