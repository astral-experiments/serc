use crate::common;

#[test]
fn matches_cpython_marshal_for_async_loop_header_return_target() {
    let source = r#"async def stream(groups, workers, stop):
    while True:
        if stop.is_set():
            return
        values = []
        async for group in groups:
            with group:
                async for value in group.values():
                    if stop.is_set():
                        return
                    values.append(value)
                    yield value
        if not values:
            return
        if values[-1].done:
            break
        for worker in workers:
            if worker.ready:
                async for value in worker.values():
                    if stop.is_set():
                        return
                    yield value
                break
    yield "done"
"#;
    common::assert_matches_cpython_marshal(source, "async_loop_header_return_target.py");
}

#[test]
fn matches_cpython_marshal_for_async_loop_header_break_control() {
    let source = r#"async def stream(groups, workers, stop):
    while True:
        if stop.is_set():
            break
        values = []
        async for group in groups:
            with group:
                async for value in group.values():
                    if stop.is_set():
                        return
                    values.append(value)
                    yield value
        if not values:
            return
        if values[-1].done:
            break
        for worker in workers:
            if worker.ready:
                async for value in worker.values():
                    if stop.is_set():
                        return
                    yield value
                break
    yield "done"
"#;
    common::assert_matches_cpython_marshal(source, "async_loop_header_break_control.py");
}
