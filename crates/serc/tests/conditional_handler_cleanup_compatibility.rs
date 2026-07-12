#[allow(dead_code)]
mod common;

#[test]
fn matches_cpython_marshal_for_conditional_handler_cleanup() {
    let source = r#"def named_for_handler(client, logger, received):
    try:
        for event in client.recv(received):
            handle(event)
    except Exception as error:
        if isinstance(error, NotImplementedError) and "workspace/didChangeConfiguration" in str(error):
            logger.debug(f"expected: {error}")
        else:
            logger.exception("error while handling event")
    return True


def named_no_else_control(client, logger, received):
    try:
        for event in client.recv(received):
            handle(event)
    except Exception as error:
        if interesting(error):
            logger.debug(f"expected: {error}")
    return True


def bare_import_handler(value):
    try:
        from primary import printer
    except ImportError:
        if isinstance(value, str):
            printer = print
        else:
            from fallback import printer
    printer(value)


def bare_multiple_handler_statement_control(value):
    try:
        from primary import printer
    except ImportError:
        report(value)
        if isinstance(value, str):
            printer = print
        else:
            from fallback import printer
    printer(value)


def assignment_arms_control(flag):
    try:
        action()
    except Error:
        value = None
        if flag:
            value = recover()
        else:
            value = fallback()
    return value


def named_import_handler_control(value):
    try:
        from primary import printer
    except ImportError as error:
        if isinstance(value, str):
            printer = print
        else:
            from fallback import printer
        report(error)
    printer(value)
"#;
    common::assert_matches_cpython_marshal(source, "conditional_handler_cleanup.py");
}
