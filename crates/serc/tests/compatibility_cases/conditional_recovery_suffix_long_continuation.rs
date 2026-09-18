use crate::common;

#[test]
fn conditional_recovery_suffix_long_continuation_matches_cpython() {
    let cases = [
        (
            "conditional_recovery_suffix_long_continuation_target.py",
            r#"def decode(text, parse, choose, raw, valid):
    text = text.strip()
    if not text:
        return None, "empty"
    try:
        decoded = parse(text)
    except ValueError:
        selected = choose(text)
        if selected is not None:
            try:
                decoded = parse(selected)
            except ValueError:
                return None, "invalid"
            mode = "selected"
        else:
            try:
                decoded, end = raw(text)
            except ValueError:
                return None, "invalid"
            if not valid(text[end:]):
                return None, "invalid"
            mode = "trailing"
    else:
        mode = "plain"
    if not isinstance(decoded, dict):
        return None, mode
    return decoded, mode
"#,
        ),
        (
            "conditional_recovery_suffix_long_continuation_control.py",
            r#"def decode(text, parse, choose, raw, valid):
    text = text.strip()
    if not text:
        return None, "empty"
    try:
        decoded = parse(text)
    except ValueError:
        selected = choose(text)
        if selected is not None:
            try:
                decoded = parse(selected)
            except ValueError:
                decoded = {}
            mode = "selected"
        else:
            try:
                decoded, end = raw(text)
            except ValueError:
                decoded, end = {}, 0
            if not valid(text[end:]):
                return None, "invalid"
            mode = "trailing"
    else:
        mode = "plain"
    if not isinstance(decoded, dict):
        return None, mode
    return decoded, mode
"#,
        ),
    ];
    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
