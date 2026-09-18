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

#[test]
fn matches_cpython_marshal_for_match_tail_if_fallthrough_return_join() {
    for (filename, source) in [
        (
            "match_raise_else_call_return_join_nop_target.py",
            r"def f(subject, flag):
    match subject:
        case First():
            prepare(subject)
            if flag:
                raise ValueError()
            else:
                observe(subject)
        case Second():
            if flag:
                raise ValueError()
            else:
                observe(subject)
        case _:
            raise ValueError()
    return subject
",
        ),
        (
            "match_raise_else_call_nonreturn_successor_control.py",
            r"def f(subject, flag):
    match subject:
        case First():
            if flag:
                raise ValueError()
            else:
                observe(subject)
        case _:
            raise ValueError()
    finish(subject)
    return subject
",
        ),
        (
            "match_fallthrough_both_arms_return_join_control.py",
            r"def f(subject, flag):
    match subject:
        case First():
            if flag:
                prepare(subject)
            else:
                observe(subject)
        case _:
            raise ValueError()
    return subject
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
