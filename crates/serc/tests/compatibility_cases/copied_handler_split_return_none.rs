use crate::common;

#[test]
fn constant_handler_while_finally_none() {
    let source = r#"async def run(manager, printer, observer, stop):
    async with manager:
        try:
            while True:
                try:
                    await action()
                except Error:
                    if stop is not None and stop():
                        printer.log("stopped")
                        return 130
                    raise
        finally:
            if observer is not None:
                observer(None)
"#;
    common::assert_matches_cpython_marshal(source, "constant_handler_while_finally_none.py");
}

#[test]
fn constant_handler_no_loop_control() {
    let source = r#"async def run(manager, printer, observer, stop):
    async with manager:
        try:
            try:
                await action()
            except Error:
                if stop is not None and stop():
                    printer.log("stopped")
                    return 130
                raise
        finally:
            if observer is not None:
                observer(None)
"#;
    common::assert_matches_cpython_marshal(source, "constant_handler_no_loop_control.py");
}

#[test]
fn nonconstant_handler_while_control() {
    let source = r#"async def run(manager, printer, observer, stop):
    async with manager:
        try:
            while True:
                try:
                    await action()
                except Error:
                    if stop is not None and stop():
                        printer.log("stopped")
                        return result
                    raise
        finally:
            if observer is not None:
                observer(None)
"#;
    common::assert_matches_cpython_marshal(source, "nonconstant_handler_while_control.py");
}
