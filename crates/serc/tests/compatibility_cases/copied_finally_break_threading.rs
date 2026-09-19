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
