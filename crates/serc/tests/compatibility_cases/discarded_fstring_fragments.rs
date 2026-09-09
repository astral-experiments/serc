use crate::common;

#[test]
fn admits_discarded_fstring_fragments_before_later_constants() {
    for expression in [
        r#"f"prefix {name} suffix""#,
        r#"f"{name} suffix""#,
        r#"f"prefix {name = } suffix""#,
        r#"f"prefix " "more " f"{name} suffix""#,
        r#"f"{name:spec{width}tail}""#,
        r#"f"""#,
    ] {
        for anchor in ["", "    emit('anchor')\n"] {
            let source = format!(
                "def function(values, flag, width):\n{anchor}    for name in values:\n        if flag:\n            continue\n            emit({expression})\n        emit('middle')\n        emit({expression})\n"
            );
            common::assert_matches_cpython_marshal(&source, "discarded_fstring_order.py");
        }
    }
}

#[test]
fn preserves_merged_raw_surrogate_and_nested_fstring_fragments() {
    for expression in [
        r#"rf"raw\n{name}\t""#,
        r#""start" f"\ud800{name}" "\udfff""#,
        r#"f"{name:\ud800{width}}""#,
        r#"f"prefix {(f'inner {name}') = } suffix""#,
        r#"f"prefix {(lambda: 'child')()} suffix""#,
        r#"f"{{prefix}} {name} {{tail}}""#,
    ] {
        let source = format!(
            "async def function(values, name, flag, width):\n    await ready()\n    yield name\n    if flag:\n        return\n        emit({expression})\n    emit('middle')\n    emit({expression})\n"
        );
        common::assert_matches_cpython_marshal(&source, "discarded_merged_fstring.py");
    }
}

#[test]
fn admits_empty_join_before_large_discarded_fstring_components() {
    for repetitions in [15, 16] {
        for prefix in ["", "x"] {
            let components = "{name}x".repeat(repetitions);
            let expression = format!("f\"{prefix}{components}\"");
            let source = format!(
                "def function(values, flag):\n    for name in values:\n        if flag:\n            break\n            emit({expression})\n        emit('middle')\n        emit({expression})\n"
            );
            common::assert_matches_cpython_marshal(&source, "discarded_large_fstring.py");
        }
    }
}
