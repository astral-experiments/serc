use crate::common;

#[test]
fn matches_cpython_original_boundary_compare_target() {
    let source = r#"async def scan(groups):
    for group in groups:
        if isinstance(group.parts, list):
            for item in group.parts:
                if item.kind == "picture":
                    payload: str = item.image.url

                    try:
                        if payload.startswith("inline:"):
                            prefix, body = payload.split(",", 1)
                            url = None
                        else:
                            prefix, body = "inline:picture", None
                            url = payload
                    except Exception as error:
                        raise ValueError("invalid image") from error

                    if body is not None:
                        consume(body)
"#;
    common::assert_matches_cpython_marshal(source, "original_boundary_compare_target.py");
}

#[test]
fn matches_cpython_original_boundary_primitive_control() {
    let source = r#"async def scan(groups):
    for group in groups:
        if isinstance(group.parts, list):
            for item in group.parts:
                if item.ready:
                    payload: str = item.image.url

                    try:
                        if payload.startswith("inline:"):
                            prefix, body = payload.split(",", 1)
                            url = None
                        else:
                            prefix, body = "inline:picture", None
                            url = payload
                    except Exception as error:
                        raise ValueError("invalid image") from error

                    if body is not None:
                        consume(body)
"#;
    common::assert_matches_cpython_marshal(source, "original_boundary_primitive_control.py");
}

#[test]
fn matches_cpython_runtime_while_break_original_empty_try_exit() {
    for (filename, source) in [
        (
            "runtime_while_break_original_empty_try_exit.py",
            r"async def f(pending, source, limit):
    try:
        while pending or source.ready:
            if pending:
                pending = await source.take()
                continue
            if source.ready:
                await source.fill()
                continue
            break
    except (ValueError, TypeError):
        await source.cancel()
        raise
    values = list(value for value in source.values)
    if limit:
        return source.finish(values, limit)
    return values
",
        ),
        (
            "runtime_while_work_original_try_exit_control.py",
            r"async def f(pending, source, limit):
    try:
        while pending or source.ready:
            if pending:
                pending = await source.take()
                continue
            if source.ready:
                await source.fill()
                continue
            break
        source.finished()
    except (ValueError, TypeError):
        await source.cancel()
        raise
    values = list(value for value in source.values)
    if limit:
        return source.finish(values, limit)
    return values
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_folded_false_original_try_marker() {
    for (filename, source) in [
        (
            "folded_false_try_marker.py",
            r"async def f(work, after):
    try:
        if False:
            await work()
    except Exception:
        pass
    await after()
",
        ),
        (
            "runtime_try_marker.py",
            r"async def f(work, after, flag):
    try:
        if flag:
            await work()
    except Exception:
        pass
    await after()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_copied_nested_try_exit_boundaries() {
    for (filename, recovery) in [
        ("copied_nested_try_exit.py", "recover(value)"),
        ("copied_nested_try_return.py", "return None"),
    ] {
        let source = format!(
            r"def process(flag, value):
    try:
        if flag:
            try:
                consume(value)
            except TypeError:
                raise ValueError from None
            finish(value)
            return None
        if value:
            consume(value)
        elif flag:
            return None
    except BaseException:
        {recovery}
    return None
"
        );
        common::assert_matches_cpython_marshal(&source, filename);
    }
}
