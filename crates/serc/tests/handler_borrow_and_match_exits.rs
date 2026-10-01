#[allow(dead_code)]
mod common;

#[test]
fn nested_match_pass_threads_through_enclosing_case_exit() {
    for prefix in ["", "async "] {
        let source = format!(
            "{prefix}def f(first, second):\n    match first:\n        case K.A | K.B:\n            match second:\n                case K.C:\n                    work()\n                case K.D:\n                    pass\n                case _:\n                    other()\n        case K.E:\n            pass\n        case _:\n            default()\n    finish()\n"
        );
        common::assert_matches_cpython_marshal(&source, "nested_match_pass_exit.py");
    }
}

#[test]
fn protected_while_break_keeps_its_nop_before_empty_normal_exit() {
    let source = r"def f(first, second, value):
    try:
        while first and second:
            if value is None:
                break
    except RuntimeError as error:
        handle(error)
    if first or second:
        use(value)
";
    common::assert_matches_cpython_marshal(source, "protected_while_break_nop.py");
}

#[test]
fn terminal_match_recovery_keeps_its_empty_borrow_exit() {
    let source = r"async def f(value, flag):
    try:
        value = await work()
    except Exception as error:
        text = await read()
        selected = flag and value.ready
        detail = text if selected else fallback
        match value.status:
            case 1:
                raise Error(detail) from error
            case 2:
                raise Error(detail) from error
            case _:
                raise Error(detail) from error
    use(value)
";
    common::assert_matches_cpython_marshal(source, "terminal_match_recovery_borrow.py");
}

#[test]
fn completed_compound_guarded_recovery_keeps_its_live_borrow_exit() {
    for recovery in ["pass", "work()"] {
        let (prefix, suffix) = if recovery == "pass" {
            ("", "")
        } else {
            ("        handle(error)\n", "        finish()\n")
        };
        let source = format!(
            "def f(value):\n    try:\n        work()\n    except Exception as error:\n{prefix}        if value.first is not None and value.second is not None and value.third is None:\n            try:\n                {recovery}\n            except OtherError as nested:\n                handle(nested)\n{suffix}    use(value)\n"
        );
        common::assert_matches_cpython_marshal(&source, "compound_guarded_recovery_borrow.py");
    }
}

#[test]
fn guarded_finalizer_before_return_keeps_the_handler_exit_live() {
    for finalizer in [
        "finish(value)",
        "if value.active and value.owner:\n                        with manager():\n                            finish(value)",
        "with manager():\n                        finish(value)",
        "try:\n                        finish(value)\n                    finally:\n                        finish(value)",
    ] {
        let source = format!(
            "def f(value):\n    try:\n        result = work()\n    except Exception as error:\n        if value.active:\n            if value.owner:\n                try:\n                    yield value\n                finally:\n                    {finalizer}\n                return\n            raise Error from error\n        with manager():\n            work()\n        result = work()\n    with manager(result):\n        pass\n"
        );
        common::assert_matches_cpython_marshal(&source, "guarded_finalizer_return_borrow.py");
    }
}

#[test]
fn intermediate_folded_boolean_operand_keeps_its_empty_borrow_block() {
    for value in [
        "None or 'value'",
        "None or False or 'value'",
        "None or 'value' or False",
        "True and True and 'value'",
        "False or None or False or 'value'",
    ] {
        let source = format!("def f():\n    value = {value}\n    yield value\n");
        common::assert_matches_cpython_marshal(&source, "folded_boolean_borrow_block.py");
    }
}
