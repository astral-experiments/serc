use crate::common::assert_matches_cpython_marshal;

#[test]
fn matches_cpython_fresh_try_owner_comprehension_prefix() {
    let source = r#"def factory(ticketing_service, audit_calls=True):
    tool_name = "enrich"
    async def f(attributes, option, destination):
        status = "success"
        error = None
        support_name = str(option.get("name") or "").strip()
        support_option = str(option.get("id") or "").strip()
        attributes = (
            [attribute for attribute in attributes if isinstance(attribute, Mapping)]
            if isinstance(attributes, list)
            else []
        )
        audit_args = {
            "system": "issue",
            "operation": "enrich",
            "destination_hash": digest(destination),
            "support_name": support_name,
            "support_option": support_option,
            "attributes_hash": digest([
                {
                    "label": str(attribute.get("label") or "").strip(),
                    "value": str(attribute.get("value") or "").strip(),
                }
                for attribute in attributes
            ]),
        }
        try:
            if not support_option:
                raise ValueError("missing option")
            payload = await ticketing_service.visit(support_option, audit_args)
        except Exception as exc:
            status = "error"
            error = repr(exc)
            payload = {"error": "failed", "message": str(exc), "system": "issue"}
            report(tool_name, error)
        if audit_calls:
            await record(tool_name, audit_args, status, payload, error)
        return payload
    return f
"#;
    assert_matches_cpython_marshal(source, "async_try_after_comprehension_dict.py");
}

#[test]
fn matches_cpython_fresh_try_owner_cell_while_prefix() {
    let source = r"async def factory(enabled, request, service, queue):
    async def generate():
        try:
            message = None
            stream = service.open_stream()
            iterator = stream_iterator(stream)

            async def stopped():
                if not await request.disconnected():
                    return False
                await iterator.close()
                await queue.put(None)
                return True

            async def emit(message, delta):
                message.extend(delta)
                await queue.put(message)

            if enabled:
                try:
                    first = await wait_for(iterator.next(), timeout=LIMIT)
                except StopAsyncIteration as error:
                    raise RuntimeError('empty stream') from error
                except (TimeoutError, ServiceTimeout) as error:
                    raise RuntimeError('initial timeout') from error
            else:
                first = await first_item(iterator, service)

            if await stopped():
                return
            message = make_message(first)
            await emit(message, first)

            while True:
                try:
                    if enabled:
                        delta = await wait_for(iterator.next(), timeout=LIMIT)
                    else:
                        delta = await iterator.next()
                except StopAsyncIteration:
                    break
                except (TimeoutError, ServiceTimeout) as error:
                    if not enabled:
                        raise
                    raise RuntimeError('stream timeout') from error
                if await stopped():
                    return
                await emit(message, delta)
            assert message is not None
            await queue.put(message)
            await queue.put(None)
        except Cancelled:
            report_cancelled()
    return generate
";
    assert_matches_cpython_marshal(source, "async_cell_while_try_prefix.py");
}
