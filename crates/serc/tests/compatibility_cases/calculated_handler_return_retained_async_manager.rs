use crate::common::assert_matches_cpython_marshal;

#[test]
fn calculated_named_handler_return_retained_async_manager() {
    assert_matches_cpython_marshal(
        r"def prepare(client, gate, prompt_for, environment, branch, limit, retries, timeout):
    active_ids: set[str] = set()

    async def process(item: str) -> tuple[str, str, dict | None]:
        async with gate:
            prompt = prompt_for(item)
            metadata = RequestMetadata(environment=environment, branch=branch)
            task_id: str | None = None
            try:
                for attempt in range(retries):
                    try:
                        task_turn = await client.create_task(prompt, metadata=metadata)
                        task_id = task_turn.task.id
                        active_ids.add(task_id)
                        current_active = len(active_ids)
                        if current_active > limit:
                            raise RuntimeError(
                                f'Exceeded active task limit: {current_active} > {limit}'
                            )
                        turn_id = task_turn.turn.id
                        output = await poll_task(client, task_id, turn_id, timeout)
                        return item, task_id, output
                    except Exception as error:
                        if attempt == retries - 1:
                            print(
                                f'Request failed for {item} after {retries} attempts: {error}'
                            )
                            return item, '', None
                        await sleep(2**attempt)
            finally:
                if task_id:
                    active_ids.discard(task_id)
            return item, '', None

    return process
",
        "calculated_named_handler_return_retained_async_manager.py",
    );
}

#[test]
fn matches_cpython_awaited_named_handler_none_return_cleanup() {
    let source = r#"async def run(manager, resource):
    async with manager:
        try:
            await work()
        except Exception as error:
            if resource is not None:
                await report(error)
            return f"failed: {error}"
        finally:
            if resource is not None:
                try:
                    await resource.close()
                except Exception as cleanup_error:
                    warn(cleanup_error)
"#;
    assert_matches_cpython_marshal(
        source,
        "saved_named_handler_none_async_await_return_target.py",
    );
}

#[test]
fn matches_cpython_named_handler_between_async_managers() {
    for (filename, source) in [
        (
            "named_handler_between_async_managers_none.py",
            r"async def f(outer, inner, values, work, retry, report):
    try:
        async with outer():
            for item in values:
                try:
                    async with inner(item):
                        try:
                            await work(item)
                        except ValueError as error:
                            if retry(error):
                                continue
                            report(error)
                            return None
                except RuntimeError:
                    report(item)
    except LookupError:
        report(None)
",
        ),
        (
            "named_handler_between_async_managers_await_none.py",
            r"async def f(outer, inner, values, work, retry, report):
    try:
        async with outer():
            for item in values:
                try:
                    async with inner(item):
                        try:
                            await work(item)
                        except ValueError as error:
                            if await retry(error):
                                continue
                            report(error)
                            return None
                except RuntimeError:
                    report(item)
    except LookupError:
        report(None)
",
        ),
    ] {
        assert_matches_cpython_marshal(source, filename);
    }
}
