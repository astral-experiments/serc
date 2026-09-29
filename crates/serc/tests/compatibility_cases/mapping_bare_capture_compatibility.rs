use crate::common;

#[test]
fn commits_mapping_captures_only_after_enclosing_patterns_succeed() {
    let source = r"def guarded_sequence(value, condition):
    a = 'initial'
    match value:
        case [{'x': a}, 0] if condition:
            pass
    return a

def guarded_mapping(value, condition):
    a = 'initial'
    match value:
        case {'nested': {'x': a}, 'tail': 0} if condition:
            pass
    return a

def guarded_alternatives(value, condition):
    a = 'initial'
    match value:
        case [{'x': a}, 0] | [{'x': a}, 2] if condition:
            pass
    return a

def mixed_alternatives(value, condition):
    a = 'initial'
    match value:
        case [{'x': a}, 0] | [_, a] if condition:
            pass
    return a

def unguarded_alternatives(value, condition):
    a = 'initial'
    match value:
        case [{'x': a}, 0] | [{'x': a}, 2]:
            pass
    return a

def class_sequence(value, condition):
    a = 'initial'
    match value:
        case [{'x': int() as a}, 0]:
            pass
    return a

def class_alternatives(value, condition):
    a = 'initial'
    match value:
        case [{'x': int() as a}, 0] | [{'x': int() as a}, 2]:
            pass
    return a

def top_level_mapping(value, condition):
    a = 'initial'
    match value:
        case {'x': a} if condition:
            pass
    return a

def top_level_class(value, condition):
    a = 'initial'
    match value:
        case {'x': int() as a} if condition:
            pass
    return a

for f in (guarded_sequence, guarded_alternatives, mixed_alternatives,
          unguarded_alternatives, class_sequence, class_alternatives):
    print(f([{'x': 99}, 0], True), f([{'x': 99}, 1], True),
          f([{'x': 99}, 2], True), f([{'x': 99}, 0], False))
print(guarded_mapping({'nested': {'x': 99}, 'tail': 1}, True),
      guarded_mapping({'nested': {'x': 99}, 'tail': 0}, False))
for f in (top_level_mapping, top_level_class):
    print(f({}, True), f({'x': 99}, False), f({'x': 99}, True))
";
    common::assert_matches_cpython_marshal(source, "enclosing_mapping_capture.py");
    common::assert_executes(
        source,
        "99 initial initial 99\n99 initial 99 99\n99 1 2 99\n99 initial 99 99\n99 initial initial 99\n99 initial 99 99\ninitial 99\ninitial 99 99\ninitial 99 99\n",
    );
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
