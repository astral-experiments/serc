use crate::common::assert_matches_cpython_marshal;

#[test]
fn restores_inner_handlers_before_unwinding_enclosing_finally_bodies() {
    for handler in ["except Error", "except", "except Error as exc"] {
        for returned in ["", " 1", " value", " result(value)"] {
            let source = format!(
                "def f(value):
    try:
        work()
    finally:
        try:
            recover()
        {handler}:
            return{returned}
        done()
"
            );
            assert_matches_cpython_marshal(&source, "handler_before_finally_unwind.py");
        }
    }
}

#[test]
fn preserves_handler_finally_order_with_suspension_and_pending_returns() {
    for (prefix, body, recovery) in [
        ("async ", "await work()", "await recover()"),
        ("", "yield value", "recover()"),
        ("", "return value", "recover()"),
    ] {
        for returned in ["", " value"] {
            let source = format!(
                "{prefix}def f(value):
    try:
        {body}
    finally:
        try:
            {recovery}
        except Error:
            return{returned}
        done()
"
            );
            assert_matches_cpython_marshal(&source, "suspended_handler_finally_unwind.py");
        }
    }
}

#[test]
fn unwinds_finally_bodies_entered_inside_handlers_before_restoring_handlers() {
    // Reversing the lexical nesting reverses the cleanup order. A conditional
    // return also retains the ordinary fallthrough from this finally body.
    for handler in ["except Error", "except Error as exc"] {
        for returned in ["", " value", " result(value)"] {
            let source = format!(
                "def f(value, flag):
    try:
        work()
    {handler}:
        try:
            recover()
        finally:
            if flag:
                return{returned}
"
            );
            assert_matches_cpython_marshal(&source, "finally_before_handler_unwind.py");
        }
    }
}

#[test]
fn matches_cpython_shared_finally_handler_return_boundaries() {
    for (filename, source) in [
        (
            "shared_finally_unnamed_handler_authored_tail_control.py",
            r"def recover(engine):
    try:
        return engine.run()
    except Exception:
        try:
            return engine.retry()
        finally:
            try:
                engine.close()
            except OSError:
                pass
            engine.flush()
",
        ),
        (
            "shared_finally_unnamed_handler_outer_try_target.py",
            r"def recover(engine):
    try:
        try:
            return engine.run()
        except Exception:
            try:
                return engine.retry()
            finally:
                try:
                    engine.close()
                except OSError:
                    pass
    except RuntimeError:
        raise
",
        ),
        (
            "shared_finally_unnamed_handler_return_target.py",
            r"def recover(engine):
    try:
        return engine.run()
    except Exception:
        try:
            return engine.retry()
        finally:
            try:
                engine.close()
            except OSError:
                pass
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
