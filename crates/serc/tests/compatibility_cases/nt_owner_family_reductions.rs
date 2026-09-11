use crate::common;

#[test]
fn local_assignment_before_nested_conditional_value() {
    common::assert_matches_cpython_marshal(
        r#"def collect(records):
    try:
        output = []
        for record in records:
            if record:
                try:
                    value = record.decode()
                    output.append({"label": value.label if value.label else ""})
                except Exception:
                    continue
        return output
    except Exception:
        raise
"#,
        "assigned_nested_conditional.py",
    );
}

#[test]
fn imported_decoder_before_nested_conditional_value() {
    common::assert_matches_cpython_marshal(
        r#"from adapters import Decoder

def collect(records):
    try:
        output = []
        for record in records:
            if record:
                try:
                    value = Decoder.decode(record)
                    output.append({
                        "key": record,
                        "prefix": value.label.split()[0],
                        "label": value.label if value.label else "",
                    })
                except Exception:
                    continue
        return output
    except Exception:
        raise
"#,
        "imported_decoder_conditional.py",
    );
}

#[test]
fn compacted_global_call_before_nested_conditional_value() {
    common::assert_matches_cpython_marshal(
        r#"def collect(records):
    try:
        output = []
        for record in records:
            if record:
                try:
                    value = decode(record)
                    output.append({"label": value.label if value.label else ""})
                except Exception:
                    continue
        return output
    except Exception:
        raise
"#,
        "compacted_global_conditional.py",
    );
}
