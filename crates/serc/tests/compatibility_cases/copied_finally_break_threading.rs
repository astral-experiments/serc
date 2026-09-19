use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_break_threading_target() {
    let source = r#"def run(source_manager, temporary_manager, path_for, is_empty, upload, retry, finish, abort):
    completed = False
    try:
        with source_manager as source:
            part_number = 1
            while True:
                part_path = None
                try:
                    with temporary_manager as temporary:
                        part_path = path_for(temporary)
                        remaining = 3
                        while remaining:
                            chunk = source.read(remaining)
                            if not chunk:
                                break
                            temporary.write(chunk)
                            remaining -= len(chunk)
                    if is_empty(part_path):
                        break
                    result = upload(part_path, part_number)
                    if result != 0:
                        if retry(result):
                            return "RETRY"
                        raise RuntimeError(result)
                finally:
                    if part_path is not None:
                        part_path.unlink(missing_ok=True)
                part_number += 1
        finish()
        completed = True
        return "CREATED"
    finally:
        if not completed:
            abort()
"#;
    common::assert_matches_cpython_marshal(source, "copied_finally_break_target.py");
}

#[test]
fn matches_cpython_marshal_for_copied_finally_break_threading_after_finally_control() {
    let source = r#"def run(source_manager, temporary_manager, path_for, is_empty, upload, retry, finish, abort):
    completed = False
    try:
        with source_manager as source:
            part_number = 1
            while True:
                part_path = None
                stop = False
                try:
                    with temporary_manager as temporary:
                        part_path = path_for(temporary)
                        remaining = 3
                        while remaining:
                            chunk = source.read(remaining)
                            if not chunk:
                                break
                            temporary.write(chunk)
                            remaining -= len(chunk)
                    stop = is_empty(part_path)
                    if not stop:
                        result = upload(part_path, part_number)
                        if result != 0:
                            if retry(result):
                                return "RETRY"
                            raise RuntimeError(result)
                finally:
                    if part_path is not None:
                        part_path.unlink(missing_ok=True)
                if stop:
                    break
                part_number += 1
        finish()
        completed = True
        return "CREATED"
    finally:
        if not completed:
            abort()
"#;
    common::assert_matches_cpython_marshal(source, "after_finally_break_control.py");
}

#[test]
fn matches_cpython_marshal_for_conditional_finally_break_ordinary_continuations() {
    for (filename, source) in [
        (
            "conditional_finally_iterator_break_ordinary_continuation.py",
            r"def process(items, acquire, choose, finish):
    for item in items:
        reader = None
        try:
            reader = acquire(item)
            if choose(item):
                break
        finally:
            if reader is not None:
                reader.close()
    finish(reader)
    return reader
",
        ),
        (
            "conditional_finally_named_handler_break_ordinary_continuation.py",
            r"def process(items, acquire, choose, report, finish):
    reader = None
    for item in items:
        try:
            reader = acquire(item)
            if choose(item):
                break
        except Exception as error:
            report(error)
            break
        finally:
            if reader is not None:
                reader.close()
    finish(reader)
    return reader
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_guarded_try_finally_iterator_breaks() {
    for (filename, source) in [
        (
            "guarded_body_and_handler_break_none_guard.py",
            r"def connect(resources, limit, ping):
    last_error = None
    for attempt, resource in enumerate(resources):
        try:
            ping(resource)
            last_error = None
            break
        except Exception as error:
            last_error = error
            if attempt == limit:
                break
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    if last_error is not None:
        raise last_error
    return True
",
        ),
        (
            "guarded_continue_cleanup_control.py",
            r"def visit(resources, perform):
    count = 0
    for resource in resources:
        try:
            if perform(resource):
                continue
            count += 1
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    return count
",
        ),
        (
            "guarded_handler_break_call_guard.py",
            r"def transfer(resource, attempts, limit, perform, already_done):
    last_error = None
    for attempt in attempts:
        try:
            perform()
            return
        except Exception as error:
            if already_done(error):
                return
            last_error = error
            if attempt == limit:
                break
        finally:
            if resource.exists():
                try:
                    resource.close()
                except OSError:
                    pass
    assert last_error is not None
    raise last_error
",
        ),
        (
            "guarded_return_cleanup_control.py",
            r"def first(resources):
    for resource in resources:
        try:
            return resource
        finally:
            if resource is not None:
                try:
                    resource.close()
                except Exception:
                    pass
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
