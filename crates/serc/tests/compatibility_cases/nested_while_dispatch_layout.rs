use crate::common;

#[test]
fn matches_cpython_marshal_for_nested_while_dispatch_target() {
    let source = r#"def stream(queue, total):
    completed = 0
    next_index = 0
    pending = {}
    while completed < total:
        message = queue.get()
        kind = message[0]
        if kind == "result":
            _, index, metadata, rows = message
            completed += 1
            pending[int(index)] = rows
            while next_index in pending:
                yield from pending.pop(next_index)
                next_index += 1
        elif kind == "error":
            raise RuntimeError(message) from None
        else:
            raise RuntimeError(kind)
    finish()
"#;
    common::assert_matches_cpython_marshal(source, "nested_while_dispatch_target.py");
}

#[test]
fn matches_cpython_marshal_for_nested_while_dispatch_direct_tail_control() {
    let source = r#"def stream(queue, total):
    completed = 0
    next_index = 0
    pending = {}
    while completed < total:
        message = queue.get()
        kind = message[0]
        if kind == "result":
            _, index, metadata, rows = message
            completed += 1
            pending[int(index)] = rows
        elif kind == "error":
            raise RuntimeError(message) from None
        else:
            raise RuntimeError(kind)
        while next_index in pending:
            yield from pending.pop(next_index)
            next_index += 1
    finish()
"#;
    common::assert_matches_cpython_marshal(source, "direct_while_tail_control.py");
}
