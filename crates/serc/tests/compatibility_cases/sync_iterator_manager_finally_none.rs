use crate::common;

#[test]
fn sync_iterator_manager_finally_none_target() {
    let source = r"def run(manager, response, chunks, selected):
    try:
        with manager:
            for chunk in chunks:
                if selected:
                    return make_result(convert(response))
    except Error:
        raise
    finally:
        if response is not None:
            response.close()
";
    common::assert_matches_cpython_marshal(source, "sync_iterator_manager_finally_none_target.py");
}

#[test]
fn sync_iterator_manager_finally_none_one_call_control() {
    let source = r"def run(manager, response, chunks, selected):
    try:
        with manager:
            for chunk in chunks:
                if selected:
                    return make_result(response)
    except Error:
        raise
    finally:
        if response is not None:
            response.close()
";
    common::assert_matches_cpython_marshal(
        source,
        "sync_iterator_manager_finally_none_one_call_control.py",
    );
}
