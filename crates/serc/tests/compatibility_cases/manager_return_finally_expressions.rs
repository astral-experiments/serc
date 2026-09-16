use crate::common;

#[test]
fn matches_cpython_manager_return_finally_ternary_expression() {
    let source = r#"async def run(config, client, value):
    fallback: object | None = None
    resource: object | None = None
    try:
        with manager(prefix="fixture-") as directory:
            root = make_root(directory)
            inputs, options = await prepare(value, root)
            text = ""
            result: bytes | None = None
            if config.mode == "first":
                result, detail, fallback, resource = await first(
                    value=value, inputs=inputs, options=options
                )
            else:
                if config.mode == "second":
                    text, fallback = await second(value, inputs or None)
                else:
                    text, fallback = await third(value, options or None)
                assert client is not None
                result = await client.read(text, options=options)
                if result is None:
                    return None
            assert result is not None
            try:
                problem, details = inspect(result)
            except Exception as error:
                problem = "inspection_failed"
                details = {}
                report(f"inspection failed: {error}")
            if problem is not None:
                left = details.get("left")
                right = details.get("right")
                suffix = ""
                if isinstance(left, float) and isinstance(right, float):
                    suffix = f" ({left:.2f}, {right:.2f})"
                return failure(
                    value=value, inputs=inputs, options=options, text=text,
                    root=root, client=client, status="failed", error=f"{problem}{suffix}",
                )
            if text:
                await publish(text)
            await save(result)
        if fallback is not None:
            await archive(fallback)
        return {"value": value, "success": True}
    finally:
        selected = resource if resource is not None else fallback
        phase = "resource" if resource is not None else "fallback"
        if selected is not None:
            started = clock()
            await cleanup(selected, True)
            report_phase(phase=phase, started=started)
"#;
    common::assert_matches_cpython_marshal(source, "manager_return_finally_ternary_expression.py");

    // Separate source lines prevent local-load fusion. The appended marker then
    // reuses the original finally POP slot, whose exception owner is NULL.
    let multiline_source = source.replace(
        "value=value, inputs=inputs, options=options, text=text,\n                    root=root, client=client, status=\"failed\", error=f\"{problem}{suffix}\",",
        "value=value,\n                    inputs=inputs,\n                    options=options,\n                    text=text,\n                    root=root,\n                    client=client,\n                    status=\"failed\", error=f\"{problem}{suffix}\",",
    );
    common::assert_matches_cpython_marshal(
        &multiline_source,
        "manager_return_finally_ternary_separate_argument_lines.py",
    );
}

#[test]
fn matches_cpython_manager_return_finally_compound_expression() {
    let source = r#"async def run(state, tracker):
    process: object | None = None
    server: object | None = None
    worker: object | None = None
    try:
        with manager(prefix=f"fixture-{state.name}-") as directory:
            root = make_root(directory)
            process = await launch(root)
            server = start_server(root)
            worker = start_worker(server)
            phases: list[object] = []
            initial = await process.perform(state)
            if initial.blocked and state.retry:
                tracker.note("retry")
                update = await process.perform(state)
                phases.append(update)
            summary = {
                "result": initial,
                "phases": [encode(phase) for phase in phases],
            }
            state.output = make_output(
                model=f"fixture/{state.model}",
                content=serialize(summary, ensure_ascii=False, indent=2, sort_keys=True),
            )
            return state
    except BaseException as error:
        tracker.fail(f"failed with {type(error).__name__}")
        raise
    finally:
        if process is not None and process.returncode is None:
            process.terminate()
            try:
                await wait_for(process.wait(), timeout=5)
            except TimeoutError:
                process.kill()
                await process.wait()
        if server is not None:
            await on_thread(server.shutdown)
            server.close()
        if worker is not None:
            worker.join(timeout=5)
"#;
    common::assert_matches_cpython_marshal(source, "manager_return_finally_compound_expression.py");
}
