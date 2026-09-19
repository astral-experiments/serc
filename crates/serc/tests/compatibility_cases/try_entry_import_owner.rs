use crate::common;

#[test]
fn matches_cpython_try_entry_import_owner_target() {
    let source = r#"def probe(options):
    try:
        if options.mode == "other":
            consume(options)
        elif options.mode == "run":
            from fixture_sdk import EngineError
            try:
                if options.token and options.fresh:
                    raise ValueError("conflicting options")
                consume(options.token)
            except EngineError as error:
                raise ValueError(str(error)) from error
    except RuntimeError as error:
        raise ValueError(str(error)) from error
"#;
    common::assert_matches_cpython_marshal(source, "try_entry_import_owner_target.py");
}

#[test]
fn matches_cpython_try_entry_assignment_owner_control() {
    let source = r#"def probe(options):
    try:
        if options.mode == "other":
            consume(options)
        elif options.mode == "run":
            EngineError = RuntimeError
            try:
                if options.token and options.fresh:
                    raise ValueError("conflicting options")
                consume(options.token)
            except EngineError as error:
                raise ValueError(str(error)) from error
    except RuntimeError as error:
        raise ValueError(str(error)) from error
"#;
    common::assert_matches_cpython_marshal(source, "try_entry_assignment_owner_control.py");
}
