use crate::common;

#[test]
fn matches_cpython_marshal_for_match_tail_if_explicit_else_target() {
    let source = r#"def transform(parts, role, supports_audio, replace, author):
    changed = False
    for part in parts or []:
        match part:
            case Audio():
                changed = True
                if role == "user" and supports_audio:
                    if part.expired:
                        if not replace:
                            report(
                                "expired",
                                part.key,
                                author.role if author is not None else None,
                                part.pointer,
                            )
                    else:
                        accept(part)
            case Image():
                visit(part)
    return changed
"#;
    common::assert_matches_cpython_marshal(source, "match_tail_if_explicit_else.py");
}
