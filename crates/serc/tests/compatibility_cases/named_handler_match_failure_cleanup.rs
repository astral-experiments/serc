use crate::common;

#[test]
fn matches_cpython_marshal_for_named_handler_final_pattern_cleanup() {
    let source = r"class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case Behavior.RETRY:
                        raise RuntimeError(
                            f'Cannot retry {exc} with behavior {self.behavior}'
                        ) from exc
        return skipped
";
    common::assert_matches_cpython_marshal(source, "named_handler_final_pattern_cleanup.py");
}

#[test]
fn matches_cpython_marshal_for_named_handler_wildcard_cleanup_control() {
    let source = r"class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case _:
                        raise RuntimeError(
                            f'Cannot retry {exc} with behavior {self.behavior}'
                        ) from exc
        return skipped
";
    common::assert_matches_cpython_marshal(source, "named_handler_wildcard_cleanup_control.py");
}

#[test]
fn matches_cpython_marshal_for_named_handler_match_success_cleanup_control() {
    let source = r"class Runner:
    async def process(self, items):
        skipped = 0
        for item in items:
            try:
                consume(item)
            except Exception as exc:
                match self.behavior:
                    case Behavior.SKIP:
                        skipped += 1
                        continue
                    case Behavior.RAISE:
                        raise exc
                    case Behavior.RETRY:
                        recover(exc)
        return skipped
";
    common::assert_matches_cpython_marshal(
        source,
        "named_handler_match_success_cleanup_control.py",
    );
}

#[test]
fn matches_cpython_singleton_match_failure_cleanup() {
    for (filename, source) in [
        (
            "named_handler_none_then_singleton_control.py",
            r"def f(work, pair):
    try:
        work()
    except ValueError as error:
        match pair():
            case (None, True):
                return error
            case (True, False):
                return None
    return False
",
        ),
        (
            "named_handler_tuple_singleton_finally_target.py",
            r"def f(run, work, pair, report, finish):
    try:
        while run():
            try:
                work()
            except Exception as error:
                match pair():
                    case (False, True):
                        report(error)
                        return
                    case (False, False):
                        report(error)
                    case (True, _):
                        pass
    finally:
        finish()
",
        ),
        (
            "named_handler_tuple_singleton_target.py",
            r"def f(work, pair):
    try:
        work()
    except ValueError as error:
        match pair():
            case (False, True):
                return error
            case (True, False):
                return None
    return False
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_named_handler_literal_value_pattern_cleanup() {
    let source = r#"def f(work, pair, finish):
    try:
        try:
            work()
        except Exception as error:
            match pair():
                case (1, "ready"):
                    return error
                case (2, "stopped"):
                    return None
    finally:
        finish()
"#;
    common::assert_matches_cpython_marshal(
        source,
        "named_handler_literal_value_pattern_control.py",
    );
}
