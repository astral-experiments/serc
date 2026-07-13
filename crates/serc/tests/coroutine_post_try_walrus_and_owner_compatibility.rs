#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_post_try_coroutine_walrus_and_owner() {
    let cases = [
        (
            "post_try_coroutine_walrus_and.py",
            r#"async def post_try_coroutine_walrus_and(message, service):
    try:
        parsed = validate(message)
    except ValueError as error:
        await report(error)
        raise

    if (token := parsed.get("token")) and (
        session := service.validate(token)
    ):
        return session
    return None
"#,
        ),
        (
            "no_preceding_try_control.py",
            r#"async def no_preceding_try_control(message, service):
    parsed = validate(message)
    if (token := parsed.get("token")) and (
        session := service.validate(token)
    ):
        return session
    return None
"#,
        ),
        (
            "synchronous_control.py",
            r#"def synchronous_control(message, service):
    try:
        parsed = validate(message)
    except ValueError:
        raise

    if (token := parsed.get("token")) and (
        session := service.validate(token)
    ):
        return session
    return None
"#,
        ),
        (
            "iterator_loop_control.py",
            r#"async def iterator_loop_control(messages, service):
    for message in messages:
        try:
            parsed = validate(message)
        except ValueError:
            continue

        if (token := parsed.get("token")) and (
            session := service.validate(token)
        ):
            return session
    return None
"#,
        ),
    ];

    for (filename, source) in cases {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
