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

#[test]
fn matches_cpython_named_match_cleanup_original_join() {
    for (filename, source) in [
        (
            "named_match_cleanup_target.py",
            r#"def process(manager, batches, Plan, Task, load, emit, unexpected):
    with manager:
        while batches:
            for item in batches.pop():
                try:
                    result = load(item)
                except Exception as error:
                    match item:
                        case Plan() as planned:
                            prepared = None
                            emit(planned, error)
                            status = "failed"
                        case Task() as task:
                            emit(task, error)
                            continue
                        case impossible:
                            unexpected(impossible)
                else:
                    match result:
                        case Plan() as prepared:
                            status = "ready"
                        case bool() as changed:
                            emit(changed)
                            continue
                        case impossible_result:
                            unexpected(impossible_result)
                emit(prepared, status)
                emit("completed", item)
"#,
        ),
        (
            "populated_match_join_control.py",
            r#"def process(manager, batches, Plan, Task, load, emit, unexpected):
    with manager:
        while batches:
            for item in batches.pop():
                try:
                    result = load(item)
                except Exception as error:
                    match item:
                        case Plan() as planned:
                            prepared = None
                            emit(planned, error)
                            status = "failed"
                        case Task() as task:
                            emit(task, error)
                            continue
                        case impossible:
                            unexpected(impossible)
                    emit("handled", error)
                else:
                    match result:
                        case Plan() as prepared:
                            status = "ready"
                        case bool() as changed:
                            emit(changed)
                            continue
                        case impossible_result:
                            unexpected(impossible_result)
                emit(prepared, status)
                emit("completed", item)
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn match_recovery_preserves_pass_jump_kind_and_terminal_orphan() {
    for first in ["raise", "report(result)"] {
        for middle in ["pass", "finish()"] {
            for binding in [" as error", ""] {
                for last in ["3", "_"] {
                    let source = format!(
                        "def recover(items, mode, result):\n    output = []\n    for item in items:\n        try:\n            result = load(item)\n        except Error{binding}:\n            match mode:\n                case 1:\n                    {first}\n                case 2:\n                    {middle}\n                case {last}:\n                    report(result)\n        if result:\n            output.append(result)\n    return output\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "match_recovery_pass_orphan.py",
                    );
                }
            }
        }
    }
}
