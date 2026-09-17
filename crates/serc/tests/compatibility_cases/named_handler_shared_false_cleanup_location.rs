use crate::common;

#[test]
fn matches_cpython_named_handler_conditional_test_cleanup_location() {
    common::assert_matches_cpython_marshal(
        r#"def query(flag, fallback):
    try:
        return request()
    except TimeoutError as exc:
        record("timeout")
        if flag if flag is not None else fallback:
            raise RuntimeError("request timed out") from exc
    except OSError as exc:
        record("error", str(exc))
        if flag if flag is not None else fallback:
            raise RuntimeError(str(exc)) from exc
    return None
"#,
        "named_handler_conditional_test_cleanup_location.py",
    );
}

#[test]
fn matches_cpython_named_handler_conditional_test_cleanup_location_single_edge_control() {
    common::assert_matches_cpython_marshal(
        r#"def query(flag, fallback):
    try:
        return request()
    except TimeoutError as exc:
        record("timeout")
        if flag:
            raise RuntimeError("request timed out") from exc
    except OSError as exc:
        record("error", str(exc))
        if flag:
            raise RuntimeError(str(exc)) from exc
    return None
"#,
        "named_handler_conditional_test_cleanup_location_single_edge_control.py",
    );
}

#[test]
fn matches_cpython_named_handler_finally_false_join_location() {
    common::assert_matches_cpython_marshal(
        r#"def publish(path, data, *, key):
    parent = storage.dirname(path)
    if not storage.exists(parent):
        try:
            storage.makedirs(parent)
        except Exception:
            if not storage.isdir(parent):
                raise
    temporary_path = f"{path}.tmp-{new_id()}"
    try:
        with storage.open(temporary_path, "wb", partial_writes_on_exc=False) as stream:
            stream.write(data)
        try:
            storage.copy(temporary_path, path, overwrite=False)
        except Exception as exc:
            if not storage.exists(path):
                raise
            existing = storage.read_bytes(path)
            if existing != data and not records_match(
                decode(existing, key=key).state,
                decode(data, key=key).state,
            ):
                raise ValueError(
                    f"conflicting records were produced for {path}"
                ) from exc
    finally:
        if storage.exists(temporary_path):
            storage.remove(temporary_path)
"#,
        "named_handler_finally_false_join_location.py",
    );
}

#[test]
fn matches_cpython_named_handler_finally_false_join_location_single_edge_control() {
    common::assert_matches_cpython_marshal(
        r#"def publish(path, data, *, key):
    parent = storage.dirname(path)
    if not storage.exists(parent):
        try:
            storage.makedirs(parent)
        except Exception:
            if not storage.isdir(parent):
                raise
    temporary_path = f"{path}.tmp-{new_id()}"
    try:
        with storage.open(temporary_path, "wb", partial_writes_on_exc=False) as stream:
            stream.write(data)
        try:
            storage.copy(temporary_path, path, overwrite=False)
        except Exception as exc:
            if not storage.exists(path):
                raise
            existing = storage.read_bytes(path)
            if existing != data:
                raise ValueError(
                    f"conflicting records were produced for {path}"
                ) from exc
    finally:
        if storage.exists(temporary_path):
            storage.remove(temporary_path)
"#,
        "named_handler_finally_false_join_location_single_edge_control.py",
    );
}
