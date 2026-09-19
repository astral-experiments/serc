use crate::common;

#[test]
fn matches_cpython_post_finally_older_iterator_owner() {
    common::assert_matches_cpython_marshal(
        r#"def read_info(path, index, *, config):
    for attempt in range(1, 4):
        try:
            resource = open_resource(path, config=config)
            try:
                return Info(
                    path=path,
                    index=index,
                    count=int(resource.metadata.count),
                )
            finally:
                resource.reader.close()
        except Exception:
            if attempt == 3:
                raise
            wait(float(2 ** (attempt - 1)))
    raise AssertionError("unreachable")
"#,
        "post_finally_older_iterator_owner_target.py",
    );
}

#[test]
fn matches_cpython_post_finally_older_iterator_owner_enclosing_try_control() {
    common::assert_matches_cpython_marshal(
        r#"def read_info(path, index, *, config):
    try:
        for attempt in range(1, 4):
            resource = open_resource(path, config=config)
            try:
                return Info(
                    path=path,
                    index=index,
                    count=int(resource.metadata.count),
                )
            finally:
                resource.reader.close()
    except Exception:
        if attempt == 3:
            raise
        wait(float(2 ** (attempt - 1)))
    raise AssertionError("unreachable")
"#,
        "post_finally_older_iterator_owner_control.py",
    );
}
