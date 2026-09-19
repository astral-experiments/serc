use crate::common;

#[test]
fn matches_cpython_marshal_for_bare_handler_manager_backedge() {
    let source = r"async def consume(read, convert, first, second, pause, record):
    while True:
        try:
            value = read()
            if value is None:
                return
            result, selected = await convert(value)
            if result is not None and selected:
                with first() as stream:
                    record(result)
                    stream.write(result)
            elif result is not None and not selected:
                with second() as stream:
                    stream.write(result)
        except:
            await pause()
            continue
";
    common::assert_matches_cpython_marshal(source, "bare_handler_manager_backedge.py");
}

#[test]
fn matches_cpython_marshal_for_bare_handler_expression_backedge() {
    let source = r"async def consume(read, convert, first, second, pause, record):
    while True:
        try:
            value = read()
            if value is None:
                return
            result, selected = await convert(value)
            if result is not None and selected:
                record(result)
                first(result)
            elif result is not None and not selected:
                second(result)
        except:
            await pause()
            continue
";
    common::assert_matches_cpython_marshal(source, "bare_handler_expression_backedge.py");
}
