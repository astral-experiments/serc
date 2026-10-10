use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

#[path = "common/optimization.rs"]
mod cpython;

use cpython::{
    LEVELS, VERSIONS, assert_matches_cpython, assert_matches_cpython_versions,
    assert_outputs_match, assert_runs, compile_both, oracle, probe, run_successfully,
};
use serc::{
    CompileOptions, OptimizationLevel, PythonVersion, compile, compile_bytes_with_options,
    compile_bytes_with_path_and_options, compile_with_options,
};

#[test]
fn erased_async_iterator_suffix_preserves_conditional_backedges() {
    for async_iterator in [false, true] {
        for work in [
            "await touch(item)",
            "value = await touch(item)",
            "value = next(entry for entry in rows if entry.ready)",
        ] {
            for prefix in [false, true] {
                let mut source = String::from("async def sample(items, rows, flag):\n");
                if prefix {
                    source
                        .push_str("    if flag:\n        prepare()\n    else:\n        finish()\n");
                }
                source.push_str(if async_iterator {
                    "    async for item in items:\n"
                } else {
                    "    for item in items:\n"
                });
                writeln!(source, "        {work}").unwrap();
                source.push_str("        if flag:\n            assert item\n        assert rows\n");
                assert_matches_cpython(&source, "erased_async_iterator_suffix.py", &LEVELS);
            }
        }
    }
    for source in [
        "async def sample(items, rows, flag):\n    try:\n        async for item in items:\n            await touch(item)\n            if flag:\n                assert item\n            assert rows\n    finally:\n        await finish(rows)\n",
        "async def sample(items, rows, flag):\n    async for item in items:\n        await touch(item)\n        if flag:\n            assert item\n        assert rows\n        assert flag\n",
    ] {
        assert_matches_cpython(source, "erased_async_iterator_suffix.py", &LEVELS);
    }
}

#[test]
fn erased_nested_coroutine_arms_preserve_saved_finalizer() {
    for (case, (first, condition)) in [
        ("item.kind == 1", "item.kind == 3"),
        (
            "item.kind == 1 or item.kind == 2",
            "item.kind == 3 or item.kind == 4",
        ),
        ("item.kind == 1 and flag", "item.kind == 3 and flag"),
        (
            "not (item.kind == 1 and flag)",
            "not (item.kind == 3 and flag)",
        ),
        (
            "item.kind == 1 and item.ready()",
            "item.kind == 3 or (item.kind == 4 and flag)",
        ),
        (
            "item.kind == 1 and item.ready()",
            "(item.kind == 3 and flag) or item.kind == 4",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        for nested_first in [false, true] {
            let mut source = String::from(
                "async def sample(items, manager, flag):\n    try:\n        for item in items:\n            if ",
            );
            source.push_str(first);
            source.push_str(":\n                assert item.ready\n");
            if nested_first {
                source.push_str(
                    "                if item.side == 1:\n                    assert item.left\n                else:\n                    assert item.right\n",
                );
            }
            source.push_str("            elif ");
            source.push_str(condition);
            source.push_str(
                ":\n                assert item.ready\n                if item.side == 2:\n                    assert item.left\n                else:\n                    assert item.right\n            else:\n                assert item.done\n    finally:\n        manager.finish()\n        await manager.wait()\n",
            );
            assert_matches_cpython(
                &source,
                &format!("erased_nested_coroutine_finalizer_{case}_{nested_first}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn erased_finalizer_after_awaited_alternative_keeps_exit_location() {
    let source = "async def sample(mode, provider, path):\n    with acquire() as context:\n        try:\n            if mode:\n                async with provider.snapshot():\n                    await body()\n            else:\n                await provider.add(path)\n        finally:\n            assert not context.pending\n";
    assert_matches_cpython(source, "erased_awaited_alternative.py", &LEVELS);
}

#[test]
fn nested_suspended_manager_jump_keeps_owner() {
    let source = "async def sample(owner, done):\n    try:\n        yield owner.item()\n    except (Cancelled, GeneratorExit):\n        raise\n    except Error as error:\n        yield owner.event(error)\n    finally:\n        try:\n            with owner.shield():\n                try:\n                    await owner.terminate()\n                finally:\n                    if not done:\n                        await owner.finish()\n        finally:\n            owner.release()\n";
    for source in [
        source.to_owned(),
        source.replacen("    try:\n", "    try:\n        async with owner.open() as values:\n            async for value in values:\n                yield owner.event(value)\n", 1),
        source.replace("if not done:", "if owner.pending:"),
        source.replacen(
            "        raise\n",
            "        with owner.shield():\n            await owner.note()\n        raise\n",
            1,
        ),
    ] {
        assert_matches_cpython(&source, "suspended_manager_jump.py", &LEVELS);
    }
}

#[test]
fn nested_suspended_guard_preserves_false_edge_location() {
    let source = "async def sample(owner):\n    try:\n        await owner.run()\n    finally:\n        with owner.shield():\n            try:\n                if owner.active == owner.ident:\n                    await owner.stream.close()\n            finally:\n                if owner.active == owner.ident:\n                    try:\n                        owner.result = owner.client.result\n                    finally:\n                        try:\n                            await owner.finish()\n                        finally:\n                            owner.release()\n";
    for source in [
        source.to_owned(),
        source.replace("owner.active == owner.ident", "owner.active"),
        source.replace("owner.result = owner.client.result", "await owner.cancel()\n                        owner.status = owner.client.status\n                        owner.result = owner.client.result"),
    ] {
        assert_matches_cpython(&source, "suspended_cleanup_guard.py", &LEVELS);
    }
}

#[test]
fn async_generator_while_exit_reaches_managed_cleanup() {
    let source = "async def sample(owner, active):\n    try:\n        while active:\n            if owner.failed:\n                break\n    except Cancelled:\n        raise\n    except Error as error:\n        yield owner.event(error)\n    finally:\n        with owner.shield():\n            await owner.finish()\n";
    for source in [
        source.to_owned(),
        source.replace("        while active:\n", "        while active:\n            item = await owner.next()\n            yield item\n"),
        source.replace("    finally:\n", "    finally:\n        for worker in owner.workers:\n            if not worker.done():\n                worker.cancel()\n"),
        source.replace("        yield owner.event(error)", "        owner.warning(error)\n        await owner.record(error)\n        yield owner.event(error)")
            .replace("            await owner.finish()", "            with owner.suppress():\n                await owner.wait()\n            await owner.finish()"),
    ] {
        assert_matches_cpython(&source, "async_generator_while_cleanup.py", &LEVELS);
    }
}

#[test]
fn nested_coroutine_else_stores_reach_outer_continuation() {
    let source = "async def sample(owner, flag):\n    try:\n        try:\n            result = await owner.run()\n        except First as error:\n            result = owner.recover(error)\n        except Second as error:\n            result, flag = owner.other(error)\n        else:\n            if flag:\n                result = owner.left()\n            else:\n                result = owner.right()\n    except BaseException as error:\n        raise\n    return owner.finish(result)\n";
    for source in [
        source.to_owned(),
        source.replacen(
            "        else:\n",
            "        else:\n            owner.prepare(result)\n",
            1,
        ),
        source
            .replace(
                "result = owner.recover(error)",
                "result = {\"error\": {\"kind\": \"invalid\", **owner.recover(error)}}",
            )
            .replace(
                "await owner.run()",
                "await owner.run(retry=lambda delay: clock.now() + delay < deadline)",
            ),
        source.replace("if flag:", "if owner.ready:"),
    ] {
        assert_matches_cpython(&source, "nested_coroutine_else_store.py", &LEVELS);
    }
}

#[test]
fn group_iterator_break_reaches_guarded_cleanup() {
    let source = "async def sample(owner, source):\n    async with owner.manager():\n        try:\n            for item in source:\n                break\n            else:\n                raise Stop()\n        except* Error as group:\n            if owner.recover:\n                await owner.note(group)\n        finally:\n            if owner.active:\n                await owner.save()\n";
    for source in [
        source.to_owned(),
        source.replace("for item in source:", "for item in range(owner.limit):"),
        source.replace("            for item", "            owner.prepare()\n            for item")
            .replace("                break", "                if owner.retry(item):\n                    continue\n                owner.process(item)\n                break"),
        source.replace("async with owner.manager():", "async with owner.manager() as manager:")
            .replace("                await owner.save()", "                await owner.save()\n            if owner.file is not None:\n                assert manager is not None\n                manager.release(owner.file)"),
    ] {
        assert_matches_cpython(&source, "group_iterator_cleanup.py", &LEVELS);
    }
}

#[test]
fn nested_raising_repair_keeps_iterator_cleanup_owned() {
    let source = "def sample(owner):\n    for item in owner.items:\n        try:\n            try:\n                result = owner.prepare(item)\n            except Error as error:\n                try:\n                    result, output = owner.repair(error)\n                except Error as other:\n                    raise Unsupported(other) from None\n                if output:\n                    owner.note(output)\n        finally:\n            owner.restore()\n        owner.finish(result)\n";
    for source in [
        source.to_owned(),
        source.replacen("        try:\n", "        try:\n            owner.start()\n            if owner.invalid or owner.closed:\n                raise ValueError()\n", 1),
        source.replace("except Error as other:", "except (Error, ValueError) as other:")
            .replace("                if output:", "                if owner.errors:\n                    owner.report(error)\n                if output:"),
        source.replace("            owner.restore()", "            owner.restore()\n            owner.enabled = item"),
    ] {
        assert_matches_cpython(&source, "nested_repair_cleanup.py", &LEVELS);
    }
}

#[test]
fn guarded_recovery_work_reaches_stored_result_cleanup() {
    let source = "def sample(owner, query, telemetry):\n    token = telemetry.set([])\n    try:\n        row = owner.grade(query)\n    except Error as error:\n        if owner.fatal(error):\n            raise\n        failure = {\"error\": type(error).__name__}\n        causes = [type(cause).__name__ for cause in owner.causes(error) if cause is not error]\n        if causes:\n            failure[\"causes\"] = causes\n        row = {\"status\": \"failed\", \"failure\": failure}\n    finally:\n        telemetry.reset(token)\n    return row\n";
    for source in [
        source.to_owned(),
        source.replace("[type(cause).__name__ for cause in owner.causes(error) if cause is not error]", "owner.causes(error)"),
        source.replace("        failure = ", "        role = next((str(cause.tag) for cause in owner.causes(error) if cause.tag), 'unknown')\n        failure = "),
        source.replace("        if owner.fatal(error):", "        if owner.ready(error):\n            owner.note(error)\n        if owner.fatal(error):"),
        source.replace("def sample", "async def sample"),
    ] {
        assert_matches_cpython(&source, "guarded_store_cleanup.py", &LEVELS);
    }
}

#[test]
fn comprehension_subscript_store_preserves_guarded_cleanup_entry() {
    let source = "async def sample(owner, report):\n    try:\n        report[\"state\"] = [{\"kind\": \"control\" if key is None else \"data\", \"code\": stream.code if stream.ready else None} for key, stream in owner.streams]\n    except BaseException as error:\n        if owner.cancelled(error):\n            raise\n    finally:\n        report[\"final\"] = [{\"kind\": \"control\" if key is None else \"data\", \"code\": stream.code if stream.ready else None} for key, stream in owner.streams]\n";
    for source in [
        source.to_owned(),
        source.replacen("    try:\n", "    try:\n        async with owner.manager():\n            await owner.prepare()\n", 1),
        source.replace("        if owner.cancelled(error):", "        report[\"failed\"] = True\n        if owner.retry:\n            try:\n                value = await owner.recover()\n                report[\"recovered\"] = value\n                ready = value.active and value.done\n                if ready:\n                    owner.note(value)\n            except BaseException as recovery_error:\n                report[\"recovery_error\"] = recovery_error\n                if owner.cancelled(recovery_error):\n                    raise\n        if owner.cancelled(error):"),
        source.replace("    finally:\n", "    finally:\n        if owner.active:\n            owner.cancel()\n        await owner.finish()\n"),
    ] {
        assert_matches_cpython(&source, "comprehension_store_cleanup.py", &LEVELS);
    }
}

#[test]
fn managed_loop_break_bypasses_erased_recovery_exit() {
    let source = "def sample(owner, active, cleanup):\n    count = 0\n    while active:\n        claimed = False\n        try:\n            with owner.pop() as item:\n                claimed = item is not None\n                if item is None:\n                    break\n                count += 1\n            if cleanup and item is not None:\n                try:\n                    if item.exists():\n                        owner.remove(item)\n                except Error as error:\n                    owner.warning(error)\n        except Error as error:\n            if claimed:\n                owner.failed()\n            owner.warning(error)\n            if not claimed:\n                if not owner.retryable(error):\n                    owner.close()\n                    raise\n    owner.close()\n    owner.report(count)\n";
    for source in [
        source.to_owned(),
        source.replace("while active:", "while active and count < limit:"),
    ] {
        assert_matches_cpython(&source, "managed_loop_break.py", &LEVELS);
    }
}

#[test]
fn conditional_iterator_arm_keeps_nested_condition_borrowed() {
    let source = "def sample():\n    with resource.temporary_directory() as folder:\n        try:\n            for mode in [2, 3] if alternate else [3]:\n                for query in [[\"LOOKUP\", \"alpha\"], [\"HELP\"], [b\"bad\\x00tail\"], [b\"x\" * 1000]]:\n                    if query[0] == \"LOOKUP\":\n                        check(mode, describe(actual), describe(expected))\n                    elif b\"\\x00\" in query[0] if isinstance(query[0], bytes) else False:\n                        if query[0].startswith(b\"help\"):\n                            output[\"differences\"].append({\"actual\": actual, \"expected\": expected})\n        finally:\n            if client:\n                client.close()\n";
    let with_else = source.replacen(
        "                    if query[0]",
        "                    actual = left.request(query)\n                    expected = right.request(query)\n                    if query[0]",
        1,
    ).replace(
        "        finally:",
        "                        else:\n                            check(actual, expected)\n                    else:\n                        check(actual, expected)\n        finally:",
    );
    for source in [
        source.to_owned(),
        source.replace("[2, 3] if alternate else [3]", "modes"),
        source.replace("else False", "else selected"),
        source.replace("isinstance(query[0], bytes)", "active"),
        with_else.clone(),
        with_else.replace("else False", "else selected"),
    ] {
        assert_matches_cpython(&source, "managed_conditional_iterator.py", &LEVELS);
    }
}

#[test]
fn coroutine_guarded_recovery_reaches_nested_cleanup() {
    let source = "async def sample(client, owner, capacity, count, registered, done, cancelled):\n    try:\n        owner.ready()\n    except (Error, Cancelled) as error:\n        if registered:\n            try:\n                await client.update()\n            except ClientError:\n                owner.warning()\n        if isinstance(error, Cancelled):\n            raise\n    finally:\n        try:\n            if not done and not cancelled:\n                await owner.cancel()\n        finally:\n            capacity.release(count)\n";
    for source in [
        source.to_owned(),
        source.replace("not done and not cancelled", "not done"),
        source.replace(
            "        if registered:",
            "        if not done:\n            cancelled = await owner.cancel()\n        code = owner.describe(error)\n        owner.note(code)\n        if registered:",
        ),
        source.replace(
            "        owner.ready()",
            "        async with owner.timeout():\n            await owner.prepare()\n        owner.ready()",
        ),
    ] {
        assert_matches_cpython(&source, "guarded_recovery_nested_cleanup.py", &LEVELS);
    }
}

#[test]
fn conditional_stored_reraise_copies_local_join() {
    let source = "async def sample(owner, target, cleanup, started, observed):\n    try:\n        try:\n            outcome, reason = 'accepted', 'acknowledged'\n        except AlreadyStarted:\n            return\n        except RemoteError as error:\n            raise Unavailable() from None\n        except TimeoutError as error:\n            raise Unavailable() from None\n    except BaseException as error:\n        if isinstance(error, Cancelled):\n            reason = 'during_dispatch' if started else 'before_dispatch'\n        elif isinstance(error, Disabled):\n            reason = 'disabled'\n        elif started:\n            if isinstance(observed, RemoteError) and observed.status in {Invalid, Unauthorized, Denied, Failed}:\n                reason = 'rejected'\n            else:\n                reason = 'transport' if isinstance(observed, (RemoteError, TimeoutError)) else 'other'\n        raise\n    finally:\n        if not cleanup:\n            owner.record(target, outcome, reason, observed)\n";
    for source in [
        source.to_owned(),
        source.replacen("async def", "def", 1),
        source
            .replacen("async def", "def", 1)
            .replacen("    try:", "    yield None\n    try:", 1),
        source.replace(
            "            reason = 'during_dispatch'",
            "            outcome = 'cancelled'\n            reason = 'during_dispatch'",
        ),
    ] {
        assert_matches_cpython(&source, "conditional_stored_reraise.py", &LEVELS);
    }
}

#[test]
fn nested_async_recovery_keeps_awaited_cleanup_entry_owned() {
    let source = "async def sample(owner, observation):\n    client = owner.client()\n    try:\n        try:\n            await owner.index(\n                validate=owner.validate if owner.allowed else None,\n                ready=observation.ready if observation is not None else None,\n            )\n        except BaseException as error:\n            try:\n                async with owner.timeout():\n                    await owner.delete()\n            except BaseException as cleanup_error:\n                if not isinstance(error, Exception) or isinstance(cleanup_error, (Denied, Conflict)):\n                    raise error from None\n                raise\n            if not isinstance(error, Exception):\n                raise error from None\n            if observation is not None:\n                observation.obsolete = True\n    finally:\n        await client.close()\n";
    for source in [
        source.to_owned(),
        source.replace("observation.obsolete = True", "observation.obsolete = False"),
        source.replace(
            "                if not isinstance(error, Exception) or isinstance(cleanup_error, (Denied, Conflict)):\n                    raise error from None\n",
            "",
        ),
        source.replace(
            "            if not isinstance(error, Exception):\n                raise error from None\n",
            "",
        ),
    ] {
        assert_matches_cpython(&source, "nested_async_cleanup_entry.py", &LEVELS);
    }
}

#[test]
fn coroutine_manager_store_reaches_cleanup_past_returns() {
    let source = "async def sample(owner, kind, choice, payload, tags):\n    status = None\n    try:\n        if kind in owner.local:\n            with owner.track():\n                result = await owner.fetch(payload)\n        elif kind in owner.remote():\n            if choice and choice.ready():\n                return owner.saved\n        else:\n            raise Failed(kind)\n    except FetchError as error:\n        raise cause from error\n    except Error as error:\n        status = owner.status(error)\n        raise\n    finally:\n        flavor = 'cleanup'\n        if status is not None:\n            tags.append(status)\n        owner.record(flavor)\n";
    for source in [
        source.to_owned(),
        source.replace("return owner.saved", "return ([], owner.saved)"),
        source.replace("return owner.saved", "return"),
        source.replace("return owner.saved", "result = owner.saved"),
        source.replace(
            "            with owner.track():",
            "            from math import pi\n            with owner.track():",
        ),
        source.replace(
            "            with owner.track():",
            "            location = None\n            owner.prepare()\n            with owner.track():",
        ),
    ] {
        assert_matches_cpython(&source, "coroutine_manager_store_return.py", &LEVELS);
    }
}

#[test]
fn coroutine_asserted_success_keeps_call_cleanup_reachable() {
    let source = "async def sample(owner, outcome, tags, metrics):\n    try:\n        async with owner.timeout():\n            runtime = owner.create()\n    except Timeout as error:\n        owner.warning()\n    except Error as error:\n        owner.emit(\n            first=runtime.backing if runtime is not None else None,\n            second=str(runtime.model) if runtime is not None else owner.model,\n            third=runtime.virtual() if runtime is not None else owner.virtual,\n            error=type(error).__name__,\n        )\n    else:\n        assert runtime is not None\n        owner.update({\n            'first': str(runtime.model),\n            'second': runtime.virtual(),\n            'elapsed': owner.elapsed(),\n            **metrics,\n        })\n    finally:\n        owner.record(owner.elapsed(), tags=[f'state:{outcome}', *tags])\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("runtime = owner.create()", "runtime = await owner.create()"),
        source.replace("    try:\n", "    runtime = None\n    try:\n"),
        source.replace(
            "tags=[f'state:{outcome}', *tags])\n",
            "tags=[f'state:{outcome}', *tags])\n        try:\n            owner.clear()\n        except Error:\n            owner.failed()\n        owner.destroy()\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("coroutine_asserted_success_{index}.py"), &LEVELS);
    }
}

#[test]
fn coroutine_retry_false_exit_reaches_conditional_cleanup() {
    let source = "async def sample(owner, adapter, payload, accepted, logging):\n    try:\n        while adapter is None or adapter.open:\n            async with owner.authorize():\n                while adapter.open:\n                    if owner.ready(payload):\n                        break\n    except Error as error:\n        if accepted:\n            await owner.close()\n        else:\n            await owner.deny()\n    except Other:\n        if adapter is None or adapter.open:\n            raise\n    finally:\n        if logging():\n            owner.log(payload)\n";
    for source in [
        source.to_owned(),
        source.replace(
            "while adapter is None or adapter.open:",
            "while adapter.open:",
        ),
        source.replace("    try:\n", "    try:\n        owner.prepare()\n"),
        source
            .replace(
                "    except Error as error:\n",
                "    except Error as error:\n        if logging():\n            owner.log(error)\n",
            )
            .replace(
                "    finally:\n",
                "    finally:\n        if adapter is not None:\n            await owner.disconnect(adapter)\n",
            ),
    ] {
        assert_matches_cpython(&source, "coroutine_retry_cleanup.py", &LEVELS);
    }
}

#[test]
fn outer_awaited_finalizers_preserve_continuation_borrowing() {
    let base = |prefix: bool, chain: bool, binding: bool| {
        let mut source = String::from(
            "async def probe(worker, cleanup, guard, task, flag, other):\n    await worker.prepare()\n    try:\n        await worker.run()\n    finally:\n",
        );
        if prefix {
            source.push_str("        worker.release()\n");
        }
        source.push_str("        if flag:\n            with guard()");
        source.push_str(if binding { " as result:\n" } else { ":\n" });
        source.push_str("                await task\n");
        if chain {
            source.push_str(
                "        elif other:\n            with guard():\n                await task\n",
            );
        }
        source.push_str(
            "        else:\n            await task\n        await cleanup.close()\n    worker.finish()\n",
        );
        source
    };
    for prefix in [false, true] {
        for chain in [false, true] {
            for binding in [false, true] {
                assert_matches_cpython(
                    &base(prefix, chain, binding),
                    &format!("outer_awaited_cleanup_{prefix}_{chain}_{binding}.py"),
                    &LEVELS,
                );
            }
        }
    }
    let source = base(true, true, false);
    for (index, (before, after)) in [
        ("    worker.finish()\n", "    assert worker.ready()\n"),
        ("    worker.finish()\n", "    assert worker\n"),
        (
            "    worker.finish()\n",
            "    assert worker.ready()\n    assert cleanup.ready()\n",
        ),
        ("    worker.finish()\n", "    pass\n"),
        ("        if flag:\n", "        if True:\n"),
        (
            "        elif other:\n            with guard():\n                await task\n",
            "        elif other:\n            with guard() as result:\n                await task\n            assert result.ready\n",
        ),
        (
            "        elif other:\n            with guard():\n                await task\n",
            "        elif other:\n            with guard() as result:\n                await task\n            if flag:\n                assert result.ready\n",
        ),
        (
            "        elif other:\n            with guard():\n                await task\n",
            "        elif other:\n            with guard():\n                await task\n        elif cleanup:\n            with guard() as result:\n                await task\n            if flag:\n                assert result.ready\n",
        ),
        (
            "        if flag:\n            with guard():\n                await task\n",
            "        if flag:\n            with guard():\n                await task\n            assert task.ready\n",
        ),
        (
            "        else:\n            await task\n",
            "        else:\n            assert (await task).ready\n",
        ),
        (
            "        else:\n            await task\n",
            "        else:\n            assert (await task).ready == 1\n",
        ),
        (
            "        else:\n            await task\n",
            "        else:\n            assert (await worker.consume(task)).ready == flag\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source.replace(before, after),
            &format!("outer_awaited_cleanup_boundary_{index}.py"),
            &LEVELS,
        );
    }
    for (index, assertion) in [
        "assert task.ready",
        "assert (await worker.consume(await task)).ready == flag",
        "assert (await (task if flag else other)).ready == flag",
    ]
    .into_iter()
    .enumerate()
    {
        let source = source.replace(
            "        else:\n            await task\n",
            &format!("        else:\n            {assertion}\n"),
        );
        assert_matches_cpython(
            &source,
            &format!("outer_awaited_erased_assertion_{index}.py"),
            &[OptimizationLevel::One, OptimizationLevel::Two],
        );
    }
}

#[test]
fn asserted_awaited_finalizers_preserve_continuation_borrowing() {
    for protected in ["pass", "await task"] {
        for managed in ["pass", "await task"] {
            for prefix in ["", "        resource.release()\n"] {
                let source = format!(
                    "async def probe(flag, task, resource):\n    try:\n        {protected}\n    finally:\n{prefix}        if flag:\n            with guard():\n                {managed}\n        else:\n            await task\n        assert await resource.finish()\n    assert resource.ready()\n"
                );
                assert_matches_cpython(&source, "asserted_awaited_finalizer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn conditional_awaited_finalizers_preserve_outer_cleanup_borrowing() {
    for managed in [false, true] {
        for body_guard in [false, true] {
            let mut source = String::from(
                "async def probe(value, manager, guard, worker, cleanup, task, flag):\n    await worker.prepare(value)\n    try:\n",
            );
            let pad = if managed {
                source.push_str("        async with manager():\n");
                "            "
            } else {
                "        "
            };
            let mut push = |line: &str| {
                source.push_str(pad);
                source.push_str(line);
                source.push('\n');
            };
            push("try:");
            push("    await worker.run(value)");
            if body_guard {
                push("    with guard():");
                push("        await worker.fetch(value)");
            } else {
                push("    await worker.fetch(value)");
            }
            for line in [
                "    assert worker.ready(value)",
                "finally:",
                "    if flag:",
                "        task.cancel()",
                "        with guard():",
                "            await task",
                "    else:",
                "        worker.stop()",
                "        await task",
                "    await cleanup.close()",
            ] {
                push(line);
            }
            source.push_str("    finally:\n        await worker.finish()\n");
            assert_matches_cpython(
                &source,
                &format!("conditional_awaited_cleanup_{managed}_{body_guard}.py"),
                &LEVELS,
            );
            if !managed && !body_guard {
                for (index, (old, new)) in [
                    ("    await worker.prepare(value)\n", ""),
                    ("                task.cancel()\n", ""),
                    ("                worker.stop()\n", ""),
                    (
                        "            await cleanup.close()\n",
                        "            await cleanup\n",
                    ),
                    (
                        "                    await task\n",
                        "                    await task.finish()\n",
                    ),
                    (
                        "        await worker.finish()\n",
                        "        worker.finish()\n",
                    ),
                    (
                        "            await worker.run(value)\n            await worker.fetch(value)\n            assert worker.ready(value)\n",
                        "            pass\n",
                    ),
                    (
                        "            await worker.run(value)\n            await worker.fetch(value)\n            assert worker.ready(value)\n",
                        "            worker.run(value)\n",
                    ),
                    ("            if flag:\n", "            if True:\n"),
                    (
                        "                task.cancel()\n                with guard():\n                    await task\n            else:\n                worker.stop()\n                await task\n",
                        "                task.cancel()\n                await task\n            else:\n                worker.stop()\n                with guard():\n                    await task\n",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let source = source.replace(old, new);
                    assert_matches_cpython(
                        &source,
                        &format!("conditional_awaited_cleanup_boundary_{index}.py"),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn conditional_manager_callback_store_keeps_cleanup_exit_owned() {
    let source = "async def sample(owner, client, task, cancel, failed, detach):\n    try:\n        replacement = owner.create()\n    finally:\n        if cancel:\n            with owner.expect(Cancelled):\n                await owner.wait(task)\n        elif failed and not detach:\n            with owner.expect(Failed):\n                await owner.wait(task)\n        else:\n            await owner.wait(task)\n        client.close = lambda: None\n    assert owner.active.session is replacement\n";
    for source in [
        source.to_owned(),
        source.replace("failed and not detach", "failed"),
        source.replace("    finally:\n", "    finally:\n        owner.prepare()\n"),
    ] {
        assert_matches_cpython(&source, "conditional_manager_callback.py", &LEVELS);
    }
}

#[test]
fn awaited_store_before_asserted_else_keeps_continuation_reachable() {
    let source = "async def sample(owner, status, handled):\n    try:\n        response = await owner.update()\n    except Error as error:\n        response = None\n    else:\n        assert status is None\n    if handled:\n        owner.first()\n    else:\n        owner.second()\n";
    for source in [
        source.to_owned(),
        source.replace(
            "        response = None",
            "        assert error.status == status\n        response = None",
        ),
        source.replace(
            "owner.update()",
            "owner.update(payload=owner.payload, source=owner.source)",
        ),
    ] {
        assert_matches_cpython(&source, "awaited_store_asserted_else.py", &LEVELS);
    }
}

#[test]
fn managed_guarded_iterator_cleanup_keeps_normal_entry_reachable() {
    let source = "async def sample(owner, runs, capture, retry, error):\n    async with await owner.start() as environment:\n        async with owner.worker(environment, handlers=[owner.first, owner.failed if error is not None else owner.recover]), owner.worker(environment):\n            for index in range(runs):\n                try:\n                    owner.results.append(result)\n                except Failed as failure:\n                    if not retry or index + 1 >= runs:\n                        raise\n                finally:\n                    if capture:\n                        for event in events:\n                            if not event.has('task'):\n                                continue\n                            if event.kind != 'read':\n                                continue\n";
    for source in [
        source.to_owned(),
        source.replace(
            "                        for event in events:",
            "                        history = await owner.history()\n                        owner.record(history)\n                        for event in events:",
        ),
        source.replace(
            "                            if event.kind != 'read':\n                                continue",
            "                            if event.kind != 'read':\n                                continue\n                            assert len(event.values) == 1\n                            owner.record(event)",
        ),
    ] {
        assert_matches_cpython(&source, "managed_guarded_iterator_cleanup.py", &LEVELS);
    }
}

#[test]
fn managed_nested_await_store_keeps_continuation_reachable() {
    let source = "async def sample(owner, groups, mode, flag, pending):\n    with owner.reporter() as reporter:\n        async with await owner.connect() as client:\n            for group in groups:\n                try:\n                    try:\n                        if mode == 'first':\n                            handle = await client.start(owner.first, group, key=key, reuse=owner.allow if flag else owner.reject)\n                        else:\n                            handle = await client.start(owner.second, group, key=key, reuse=owner.allow if flag else owner.reject)\n                    except AlreadyStarted:\n                        handle = client.lookup(key)\n                except Exception as error:\n                    continue\n                if len(pending) >= limit:\n                    await owner.collect(reporter, pending)\n";
    for source in [
        source.to_owned(),
        source.replace("owner.allow if flag else owner.reject", "owner.allow"),
    ] {
        assert_matches_cpython(&source, "managed_nested_await_store.py", &LEVELS);
    }
}

#[test]
fn prefixed_copied_recovery_keeps_normal_continuation_owned() {
    let source = "async def sample(owner, value, flag, needed):\n    try:\n        try:\n            owner.report(value)\n        except Exception as error:\n            if flag:\n                return None\n            result = None\n        return owner.result, value\n    except CancelledError:\n        raise\n    except Exception as error:\n        return None\n    finally:\n        if owner.helper is not None:\n            owner.helper.close()\n        if needed and owner is not None:\n            try:\n                await owner.close(value)\n                owner.finished()\n            except Exception as error:\n                owner.problem(error)\n                raise\n";
    for source in [
        source.to_owned(),
        source.replace("await owner.close(value)", "owner.close(value)"),
        source.replace(
            "        if owner.helper is not None:\n            owner.helper.close()",
            "        owner.before()",
        ),
    ] {
        assert_matches_cpython(&source, "prefixed_copied_recovery.py", &LEVELS);
    }
}

#[test]
fn coroutine_nested_else_call_keeps_continuation_reachable() {
    let source = "async def sample(owner):\n    try:\n        owner.prepare()\n    except Error:\n        owner.fail()\n    else:\n        try:\n            result = await owner.work()\n        except Error:\n            owner.fail()\n        else:\n            if owner.invalid(result):\n                owner.fail()\n            else:\n                try:\n                    owner.parse(result)\n                except Error:\n                    owner.fail()\n                else:\n                    owner.store(result)\n    owner.done()\n";
    for source in [
        source.to_owned(),
        source.replace("except Error:", "except Error as error:"),
        source.replace(
            "            else:\n                try:",
            "            elif owner.incomplete(result):\n                owner.fail()\n            else:\n                try:",
        ),
        source.replace(
            "                    owner.store(result)",
            "                    owner.update(result, status='partial' if result else 'complete')\n                    owner.output['result'] = result",
        ),
    ] {
        assert_matches_cpython(&source, "coroutine_nested_else_call.py", &LEVELS);
    }
}

#[test]
fn copied_chained_raise_keeps_normal_continuation_owned() {
    let source = "async def sample(owner, values, problem, flag):\n    for value in values:\n        try:\n            result = await owner.read(value)\n        except Error as error:\n            if 0 < value < 10:\n                raise problem from error\n        else:\n            result = owner.convert(value if flag else None)\n        owner.record(result)\n";
    for source in [
        source.to_owned(),
        source.replace(
            "        except Error as error:\n",
            "        except Error as error:\n            problem = owner.error()\n",
        ),
        source.replace("0 < value < 10", "value < 10"),
        source.replace(
            "raise problem from error",
            "raise owner.error(problem) from error",
        ),
    ] {
        assert_matches_cpython(&source, "copied_chained_raise.py", &LEVELS);
    }
}

#[test]
fn awaited_else_break_keeps_normal_cleanup_reachable() {
    let source = "async def sample(owner, prepare, requested, observations, recovered, resolver, binding, context, streaming, emission):\n    while True:\n        try:\n            result, fallback, data, entries, tree, download, entities = owner.compile(\n                names=resolver.names if resolver is not None else None,\n                prefix=resolver.prefix if resolver is not None else None,\n                resolve=owner.resolve if binding is not None else None,\n                context=(context, recovered) if prepare is not None else context,\n                diagnostics=observations if not streaming else None,\n                counts=observations if not streaming else None,\n                output=emission,\n            )\n        except (ValueError, TypeError):\n            if recovered or prepare is None or not await prepare(tuple(requested)):\n                for observation in observations:\n                    owner.record(observation)\n        else:\n            if recovered or prepare is None or not await prepare(tuple(requested)):\n                break\n        if emission is not None:\n            emission.summary = None\n";
    for source in [
        source.to_owned(),
        source
            .replace(
                "        else:\n",
                "        else:\n            if prepare is not None:\n                requested.update(owner.references(entries))\n",
            )
            .replace(
                "                    owner.record(observation)\n",
                "                    owner.record(observation)\n                raise\n",
            ),
    ] {
        assert_matches_cpython(&source, "awaited_else_break.py", &LEVELS);
    }
}

#[test]
fn guarded_break_preserves_normal_cleanup_ownership() {
    let source = "def sample(owner, values):\n    try:\n        while owner.ready():\n            try:\n                value = owner.next()\n                if (value or {}).get('ready', 0) > 0:\n                    owner.record(value)\n                    break\n            except Error as error:\n                owner.problem(error)\n            finally:\n                if not owner.done():\n                    owner.cancel()\n            owner.pause()\n        else:\n            raise Error()\n        owner.finish()\n    finally:\n        owner.close()\n";
    for source in [
        source.to_owned(),
        source.replace("(value or {}).get('ready', 0) > 0", "values"),
        source.replace(
            "    finally:\n        owner.close()",
            "    except Error as error:\n        owner.record(error)\n        raise\n    finally:\n        try:\n            owner.close()\n        finally:\n            owner.finish()",
        ),
    ] {
        assert_matches_cpython(&source, "guarded_break_cleanup.py", &LEVELS);
    }
}

#[test]
fn stored_call_before_recovery_keeps_iterator_cleanup_reachable() {
    let source = "def sample(owner, prepared, paths, opened, failure):\n    try:\n        owner.prepare(paths)\n        for path in paths:\n            owner.register(path)\n        while owner.active() or owner.poll() is None:\n            if owner.expired() and owner.active():\n                continue\n            try:\n                events = owner.select()\n            except BaseException as error:\n                continue\n            for key, item in events:\n                if not isinstance(item, Entry):\n                    continue\n                try:\n                    data = owner.read(item.name, key.fd)\n                except RetryError:\n                    continue\n                except BaseException as error:\n                    continue\n                if not data:\n                    continue\n                if item.failed:\n                    continue\n        returncode = owner.wait(timeout=0)\n    except BaseException as error:\n        owner.record(error)\n        if owner.process is not None and owner.process.poll() is None:\n            owner.terminate()\n            try:\n                owner.process.wait(timeout=1)\n            except TimeoutError:\n                owner.process.wait()\n        returncode = None if owner.process is None else owner.returncode\n    finally:\n        owner.release()\n        owner.close()\n        if not prepared:\n            for path in paths:\n                if path in opened:\n                    continue\n    complete(0 if failure is None else 1)\n";
    assert_matches_cpython(source, "poll_recovery_cleanup.py", &LEVELS);

    let dispatch = "def sample(owner, value, values, flag):\n    try:\n        for item in values:\n            owner.process(item)\n        result = owner.wait(value)\n    except Error as error:\n        owner.before(error)\n        if owner.available():\n            try:\n                owner.recover(error)\n            except RecoverError:\n                owner.fix()\n        else:\n            owner.skip()\n        result = None if flag else value\n    finally:\n        owner.release()\n    owner.after(value)\n";
    for source in [
        dispatch.to_owned(),
        dispatch
            .replace("                owner.fix()", "                return")
            .replace("        else:\n            owner.skip()\n", ""),
    ] {
        assert_matches_cpython(&source, "conditional_recovery_cleanup.py", &LEVELS);
    }
}

#[test]
fn runtime_while_recovery_keeps_finalizer_borrow_entry() {
    for named in [false, true] {
        for nested in [false, true] {
            for none_test in [false, true] {
                let work = if nested {
                    "            for item in items:\n                result = advance(item)\n"
                } else {
                    "            result = advance(items)\n"
                };
                let handler = if named {
                    "ValueError as error"
                } else {
                    "ValueError"
                };
                let recovered = if named { "error" } else { "items" };
                let condition = if none_test {
                    "result is not None"
                } else {
                    "ready(result)"
                };
                let source = format!(
                    "def sample(items, flag):
    engine = setup(items)
    result = None
    try:
        while flag:
{work}            if {condition}:
                break
    except {handler}:
        result = {recovered}
        for item in items:
            cancel(item)
    finally:
        engine.close()
    return result
"
                );
                assert_matches_cpython(&source, "runtime_while_recovery.py", &LEVELS);
                if named && !nested && none_test {
                    for (before, after) in [
                        ("    finally:\n        engine.close()\n", ""),
                        (
                            "                break\n",
                            "                finish(result)\n",
                        ),
                        (
                            "        for item in items:\n            cancel(item)\n",
                            "        cancel(items)\n",
                        ),
                        ("        while flag:\n", "        for flag in items:\n"),
                    ] {
                        assert_matches_cpython(
                            &source.replace(before, after),
                            "runtime_while_recovery_control.py",
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_recovery_preserves_loop_cleanup_entry() {
    for leading in ["", "        if ready:\n            resource.prepare()\n"] {
        for guard in [false, true] {
            for recovery in ["pass", "report(private)"] {
                for cleanup in [
                    "            handle.close()\n",
                    "            if handle is not None:\n                handle.close()\n",
                ] {
                    let mut source = format!(
                        "def sample(resource, private, public, ready):\n    try:\n{leading}        while not resource.stopped():\n            if ready:\n                resource.poll()\n            resource.wait()\n    except Error as error:\n"
                    );
                    if guard {
                        source.push_str("        if private is not None:\n");
                    }
                    let indent = if guard { "            " } else { "        " };
                    write!(source, "{indent}try:\n{indent}    write(private, resource)\n{indent}except OSError:\n{indent}    {recovery}\n    finally:\n        for handle in (private, public):\n{cleanup}").unwrap();
                    assert_matches_cpython(&source, "guarded_loop_cleanup.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn runtime_while_break_reaches_finalizer_iteration() {
    for manager in ["", "with acquire():", "with acquire(), acquire():"] {
        for nested in [false, true] {
            for tail in ["", "            tick()\n"] {
                let guard = if nested {
                    "            if not waiting:\n                if closed or cancelled():\n                    break\n"
                } else {
                    "            if closed or cancelled():\n                break\n"
                };
                let suite = format!(
                    "    try:\n        while waiting or not closed:\n{guard}{tail}    except BaseException as error:\n        pass\n    finally:\n        for item in groups.values():\n            finish(item, state)\n"
                );
                let source = if manager.is_empty() {
                    format!("def probe():\n{suite}")
                } else {
                    let mut indented = String::new();
                    for line in suite.lines() {
                        writeln!(indented, "    {line}").unwrap();
                    }
                    format!("def probe():\n    {manager}\n{indented}")
                };
                assert_matches_cpython(&source, "runtime_while_finalizer_iteration.py", &LEVELS);
            }
        }
    }
}

#[test]
fn managed_guarded_runtime_break_keeps_iterator_cleanup_reachable() {
    let source = "def sample(owner, required, deadline, runner):\n    with owner.temporary() as directory:\n        with owner.log(directory) as stream:\n            try:\n                if required:\n                    while pending:\n                        if clock() >= deadline:\n                            break\n                        pause(0.01)\n            except TimeoutError:\n                timed_out = True\n            finally:\n                for signal, timeout in ((1, 3), (2, 1)):\n                    if runner.poll() is not None:\n                        break\n                for signal in (1, 2):\n                    while remaining:\n                        remaining = alive\n                        if not remaining or clock() >= deadline:\n                            break\n";
    for source in [
        source.to_owned(),
        source
            .replace("                if required:", "                runner.wait(timeout=20)\n                if required:")
            .replace("                    while pending:", "                    pending = {item.key for item in owner.items()}\n                    deadline = clock() + 5\n                    while pending:")
            .replace("                        if clock()", "                        for item in tuple(pending):\n                            try:\n                                owner.check(item)\n                            except Missing:\n                                pending.remove(item)\n                        if clock()"),
        "def sample(owner, ready, stopped, values):\n    try:\n        if ready:\n            while owner.active():\n                owner.step()\n                if stopped:\n                    break\n    except Error:\n        failed = True\n    finally:\n        for token in values:\n            if owner.done(token):\n                break\n            owner.record(token)\n".to_owned(),
    ] {
        assert_matches_cpython(&source, "managed_guarded_watch.py", &LEVELS);
    }
}

#[test]
fn managed_loop_conditional_call_store_keeps_cleanup_reachable() {
    let source = "def sample(owner, pending, remaining, checked, filtered, complete, offsets):\n    try:\n        with owner.stage() as staged, owner.filtered() as stream, owner.workers() as workers:\n            try:\n                while True:\n                    if not pending:\n                        selected = owner.select(remaining, version=owner.version)\n                        break\n                    for outcome in owner.check(pending, workers=workers):\n                        if isinstance(outcome, Accepted):\n                            staged.write(outcome.encode() + '\\n')\n                        else:\n                            stream.flush()\n            finally:\n                report = summarize(checked=checked, filtered=filtered, complete=complete, missing=len(offsets) - len(selected) if complete else None)\n    finally:\n        owner.close()\n";
    for source in [
        source.to_owned(),
        source.replace(
            "    finally:\n        owner.close()",
            "                owner.record(report.encode())\n    finally:\n        owner.close()",
        ),
    ] {
        assert_matches_cpython(&source, "managed_conditional_report.py", &LEVELS);
    }
}

#[test]
fn managed_iterator_inner_break_keeps_cleanup_reachable() {
    let source = "def sample(owner, protected):\n    with owner.temporary() as directory:\n        with owner.open(directory) as stream:\n            for member in stream:\n                if member.name == 'index':\n                    continue\n                target = owner.output(member.path) if member.kind == 'data' else None\n                try:\n                    while True:\n                        if not chunk:\n                            break\n                        if target is not None:\n                            if protected:\n                                tail = searchable[-overlap:] if overlap else b''\n                finally:\n                    if target is not None:\n                        target.close()\n";
    assert_matches_cpython(source, "managed_iterator_cleanup.py", &LEVELS);
}

#[test]
fn coroutine_raising_guard_keeps_nested_cleanup_reachable() {
    let source = "async def sample(owner, completed, state, output, validated):\n    try:\n        while True:\n            if completed:\n                break\n            await owner.step(timeout=60)\n        if state.strip():\n            raise RuntimeError('Unexpected state: ' + state.decode(errors='replace')[-100:])\n    except Exception as error:\n        failure = error\n    finally:\n        try:\n            inventory = await owner.capture(timeout=60)\n        except Exception:\n            inventory = b''\n        await owner.close()\n    save(output, (encode(validated, indent=2, sort_keys=True) + '\\n').encode(), label='result')\n";
    assert_matches_cpython(source, "coroutine_raise_cleanup.py", &LEVELS);
}

#[test]
fn runtime_break_keeps_guarded_cleanup_reachable() {
    let source = "def sample(owner):\n    try:\n        while not owner.stop.is_set() or completed:\n            if drain:\n                break\n            if waiting and reservation is None:\n                try:\n                    if owner.external:\n                        if owner.stop.is_set():\n                            break\n                    else:\n                        reservation, placement = reserve(native, owner.limits, owner.stop)\n                except BaseException:\n                    raise\n            for unused in range(len(waiting)):\n                try:\n                    if key in cancelled:\n                        continue\n                    if owner.paused() or (placement is not None and len(placement.busy) == len(placement.nodes)):\n                        continue\n                    if bundle is None:\n                        continue\n                finally:\n                    del manifest, payload\n            for receipt in ready:\n                try:\n                    if receipt in retiring or key[0] in retired:\n                        continue\n                    try:\n                        value = runtime.get(receipt)\n                    except Exception as error:\n                        failure = error\n                    else:\n                        continue\n                finally:\n                    del result, receipt\n    except BaseException as error:\n        try:\n            runtime.get(owner.actor.fail.remote())\n        except Exception:\n            pass\n    finally:\n        if reservation is not None and not owner.external:\n            owner.report()\n";
    for source in [
        source.to_owned(),
        source.replace("def sample(owner):", "def sample(owner, completed, drain, waiting, reservation, placement, native, cancelled, key, bundle, manifest, payload, ready, retiring, retired, runtime, result):"),
    ] {
        assert_matches_cpython(&source, "runtime_guarded_cleanup.py", &LEVELS);
    }
}

#[test]
fn nested_async_iterator_else_break_keeps_continuation_cold() {
    let source = "async def sample(owner, params, limit, content):\n    try:\n        async with owner.session() as session:\n            async with session.request(params=dict(params or {})) as response:\n                async for chunk in response.chunks():\n                    if limit is None:\n                        content.extend(chunk)\n                    else:\n                        content.extend(chunk[:limit])\n                        if len(content) > limit:\n                            break\n                return Result(response.status, dict(response.headers), response.charset, bytes(content))\n    except TimeoutError:\n        raise RequestTimeout('request timed out') from None\n    except RequestError as error:\n        raise WrappedError(error) from None\n    finally:\n        await owner.close()\n";
    for source in [
        source.to_owned(),
        source
            .replace("                async for chunk", "                status = response.status\n                async for chunk")
            .replace("content.extend(chunk[:limit])", "remaining = limit + 1 - len(content)\n                        content.extend(chunk[:remaining])")
            .replace("return Result(response.status,", "return Result(status,"),
    ] {
        assert_matches_cpython(&source, "nested_async_iterator_else.py", &LEVELS);
    }
}

#[test]
fn runtime_loop_nested_recovery_keeps_cleanup_reachable() {
    let source = "async def sample(owner, kind, stage, payload, partial, name):\n    try:\n        while owner.state['next'] != 'done':\n            try:\n                if kind == 'target':\n                    owner.complete(stage, payload)\n                elif kind == 'extract':\n                    owner.extract(stage)\n                elif kind in {'merge', 'initial', 'revision'}:\n                    owner.finish(stage, 'initial' if kind == 'merge' else 'validate')\n                elif kind == 'validate':\n                    owner.finish(stage, 'review')\n                elif kind == 'review':\n                    owner.review(stage, payload)\n                else:\n                    raise ValueError(kind)\n            except Exception as error:\n                if kind == 'target' and 'recovery' in owner.state and 'hash' in stage:\n                    if owner.invalid(payload):\n                        continue\n                continue\n    except (Exception, Cancelled) as error:\n        if owner.workspace is not None and not isinstance(error, Unresolved):\n            if partial is not None:\n                try:\n                    owner.state['partial'] = {'path': name, 'hash': save(owner.directory / name, partial)}\n                except Exception as failure:\n                    owner.state['error'] = str(failure)\n    finally:\n        if owner.workspace is not None:\n            owner.save()\n";
    for source in [
        source.to_owned(),
        source
            .replace("    except (Exception, Cancelled) as error:\n", "        owner.state['status'] = 'complete'\n        owner.state['reason'] = 'ok' if owner.accepted else 'retry'\n        owner.save()\n    except (Exception, Cancelled) as error:\n")
            .replace("            if partial is not None:\n", "            try:\n                partial = await owner.capture()\n            except CaptureError as failure:\n                partial = failure.partial\n            except Exception as failure:\n                partial = None\n            if partial is not None:\n")
            .replace("    finally:\n        if owner.workspace is not None:\n            owner.save()\n", "    finally:\n        if owner.workspace is not None:\n            try:\n                owner.state['cleanup'] = await owner.close()\n            except (Exception, Cancelled) as failure:\n                owner.state['cleanup'] = {'error': str(failure)}\n            owner.save()\n"),
    ] {
        assert_matches_cpython(&source, "runtime_loop_nested_recovery.py", &LEVELS);
    }
}

#[test]
fn conditional_value_cleanup_keeps_live_loop_break() {
    let source = "async def sample(owner, stopped, pending, deadline, now):\n    try:\n        while True:\n            if stopped and not pending:\n                break\n            if stopped:\n                if deadline is None:\n                    deadline = now + owner.timeout\n                elif now >= deadline:\n                    break\n    except Cancelled:\n        raise\n    except Exception:\n        raise\n    finally:\n        payload = {'started': owner.started.isoformat() if owner.started is not None else None, 'elapsed': clock() - now}\n";
    for source in [
        source.to_owned(),
        source.replace("payload = {'started': owner.started.isoformat() if owner.started is not None else None, 'elapsed': clock() - now}", "budget = owner.timeout if deadline is None else max(1, deadline - clock())"),
    ] {
        assert_matches_cpython(&source, "conditional_loop_cleanup.py", &LEVELS);
    }
}

#[test]
fn managed_async_retry_break_keeps_continuation_cold() {
    let source = "async def sample(owner, kind, item, created):\n    try:\n        changed = True\n    except Conflict:\n        if kind == 'primary':\n            async for attempt in owner.retry():\n                with attempt:\n                    if item is None or item.removed():\n                        break\n                    if item.kind == 'primary':\n                        break\n                    break\n        else:\n            owner.record({'kind': kind, **details(item)})\n    return owner.result(item, created)\n";
    for source in [
        source.to_owned(),
        source
            .replace("        changed = True", "        await owner.save(item)\n        changed = True")
            .replace("                with attempt:\n", "                with attempt:\n                    item = await owner.get()\n")
            .replace("                    if item.kind == 'primary':\n                        break", "                    if item.kind == 'primary':\n                        current = item\n                        break")
            .replace("                    break\n        else:", "                    item.kind = 'primary'\n                    await owner.save(item)\n                    current = item\n                    break\n        else:"),
    ] {
        assert_matches_cpython(&source, "managed_async_retry_break.py", &LEVELS);
    }
}

#[test]
fn guarded_continue_recovery_keeps_continuation_cold() {
    for (scope, suspend) in [("def", ""), ("async def", "await ")] {
        let source = format!(
            "{scope} sample(owner, names, pending):\n    known = {{}}\n    try:\n        response = {suspend}owner.fetch(names)\n    except FetchError as error:\n        if error.code() != owner.missing:\n            raise\n        if len(names) == 1:\n            owner.warn(names[0])\n            return []\n        for name in names:\n            try:\n                response = {suspend}owner.fetch([name])\n            except FetchError as failure:\n                if failure.code() != owner.missing:\n                    raise\n                owner.warn(name)\n                continue\n            known.update({{item.name: item for item in response.items}})\n    else:\n        known = {{item.name: item for item in response.items}}\n    for name in pending:\n        if owner.find(name) is None:\n            owner.warn(name)\n            continue\n"
        );
        assert_matches_cpython(&source, "guarded_continue_recovery.py", &LEVELS);
    }
}

#[test]
fn attribute_key_recovery_loop_keeps_continuation_cold() {
    for (scope, suspend) in [("def", ""), ("async def", "await ")] {
        let source = format!(
            "{scope} sample(owner, names, pending):\n    known = {{}}\n    try:\n        response = {suspend}owner.fetch(names)\n    except FetchError as error:\n        for name in names:\n            try:\n                response = {suspend}owner.fetch([name])\n            except FetchError as failure:\n                continue\n            known.update({{item.name: item for item in response.items}})\n    else:\n        known = {{item.name: item for item in response.items}}\n    for name in pending:\n        if owner.find(name) is None:\n            owner.warn(name)\n            continue\n"
        );
        for source in [
            source.clone(),
            source.replace("    known = {}\n", "    known = {}\n    for entry in pending:\n        try:\n            owner.inspect(entry)\n        except FetchError as error:\n            continue\n        if owner.reject(entry):\n            continue\n"),
        ] {
            assert_matches_cpython(&source, "attribute_key_recovery.py", &LEVELS);
        }
    }
}

#[test]
fn conditional_retry_else_keeps_cleanup_cold() {
    let source = "async def sample(owner, merged, token):\n    try:\n        owner.verify()\n    except (FirstError, SecondError) as error:\n        if merged and not (isinstance(error, RemoteError) and error.code == 'unavailable'):\n            try:\n                owner.verify()\n            except (FirstError, SecondError) as retry_error:\n                saved_error = retry_error\n            else:\n                saved_error = None\n        else:\n            saved_error = error\n    else:\n        saved_error = None\n    finally:\n        owner.contracts.reset(token)\n    return Result(error=saved_error)\n";
    for source in [
        source.to_owned(),
        source
            .replacen("        owner.verify()\n", "        value = await owner.prepare()\n        owner.verify()\n", 1)
            .replace("                owner.verify()\n", "                value = await owner.retry(value)\n                owner.verify()\n"),
    ] {
        assert_matches_cpython(&source, "conditional_retry_else.py", &LEVELS);
    }
}

#[test]
fn coroutine_nested_breaks_reach_located_cleanup() {
    let source = "async def sample(owner, ready, running, abort, failure, scheduled, item, executed):\n    try:\n        while ready or running:\n            if abort is not None or failure is not None:\n                if abort is not None and abort.ready():\n                    break\n                if failure is not None and failure.ready():\n                    break\n            while ready:\n                if item in scheduled:\n                    continue\n            if not running:\n                break\n    except BaseException:\n        raise\n    finally:\n        try:\n            owner.slowest = compute(owner, executed)\n        finally:\n            owner.clear()\n";
    for source in [
        source.to_owned(),
        source
            .replace("            if abort is not None or failure is not None:\n", "            if abort is not None or failure is not None:\n                await owner.yield_control()\n")
            .replace("            while ready:\n", "            while ready:\n                item = ready.pop()\n")
            .replace("                    continue\n", "                    continue\n                owner.schedule(item)\n")
            .replace("    except BaseException:\n", "    except BaseException:\n        await owner.cancel()\n"),
    ] {
        assert_matches_cpython(&source, "coroutine_nested_breaks.py", &LEVELS);
    }
}

#[test]
fn reraised_async_manager_keeps_recovery_continuation_cold() {
    let source = "async def sample(owner, lock, records, value, rounds):\n    for iteration in range(rounds):\n        try:\n            value['status'] = 'rejected'\n        except OwnerError as error:\n            if not owner.recoverable(error):\n                async with lock:\n                    records.append(value)\n                raise\n        except (ValueError, TypeError) as error:\n            value['error'] = error\n        async with lock:\n            records.append(value)\n";
    for source in [
        source.to_owned(),
        source
            .replace("            value['status'] = 'rejected'", "            proposal = await owner.propose(value)\n            if owner.approved(proposal):\n                async with lock:\n                    records.append(value)\n                return owner.result(proposal)\n            value['status'] = 'rejected'")
            .replace("            if not owner.recoverable(error):\n", "            if not owner.recoverable(error):\n                value['status'] = 'failed'\n                value['error'] = str(error)\n"),
    ] {
        assert_matches_cpython(&source, "reraised_async_manager.py", &LEVELS);
    }
}

#[test]
fn nested_raising_cleanup_keeps_enclosing_pop_location() {
    let source = "def sample(owner, handle, value, before, after):\n    try:\n        try:\n            with owner.open(handle) as source:\n                try:\n                    yield value\n                finally:\n                    value.close()\n        finally:\n            if after != before:\n                raise ValueError('resource changed')\n    finally:\n        owner.release(handle)\n";
    for source in [
        source.to_owned(),
        source.replace(
            "        finally:\n            if after",
            "        finally:\n            after = owner.state(handle)\n            if after",
        ),
        source.replace("after != before", "before < value < after"),
        source.replace("    finally:\n        owner.release", "    except Exception:\n        owner.report(value)\n    finally:\n        owner.release"),
    ] {
        assert_matches_cpython(&source, "nested_raising_cleanup.py", &LEVELS);
    }
}

#[test]
fn recovery_loop_comprehension_keeps_continuation_cold() {
    for (scope, suspend) in [("def", ""), ("async def", "await ")] {
        let source = format!(
            "{scope} sample(owner, groups, optional):\n    try:\n        value = {suspend}owner.query()\n    except (LoadError, ValueError):\n        value = project(owner.default, ())\n        for group in groups:\n            try:\n                extra = {suspend}owner.query(group)\n            except (LoadError, ValueError):\n                continue\n            value.update({{field: extra.get(field) for field in group}})\n    return project(value, optional)\n"
        );
        for source in [
            source.clone(),
            source
                .replace("        value = project(owner.default, ())", &format!("        if not optional:\n            raise\n        value = {suspend}owner.query()\n        validate(value)\n        value = project(value, ())"))
                .replace("            value.update(", "            validate(extra)\n            value.update("),
            source.replace("            except (LoadError, ValueError):\n                continue", "            except (LoadError, ValueError) as error:\n                record(error)\n                continue"),
            source.replace("{field: extra.get(field) for field in group}", "{field for field in group}"),
        ] {
            assert_matches_cpython(&source, "recovery_loop_comprehension.py", &LEVELS);
        }
    }
}

#[test]
fn conditional_deletion_keeps_cleanup_reachable() {
    for scope in ["def", "async def"] {
        for target in ["pending[item]", "owner.slot", "started"] {
            let source = format!(
                "{scope} sample(owner, pending, item, response, attempt, started):\n    try:\n        if response.status == 200:\n            if owner.ready(pending[item], owner.view(response)):\n                del {target}\n        else:\n            ensure(response.status in {{None, 503}}, 'unexpected response')\n    except Cancelled:\n        if attempt.started is not None and attempt.value is None:\n            attempt.elapsed = (clock() - started) * 1000\n    finally:\n        record(owner.path, {{'kind': 'attempt', **attempt.summary()}})\n"
            );
            assert_matches_cpython(&source, "conditional_delete_cleanup.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_recovery_keeps_cleanup_cold() {
    let source = "async def sample(owner, stream, payload, kind, close):\n    try:\n        async for event in stream:\n            if not isinstance(payload, dict):\n                continue\n            if kind in {'finished', 'failed', 'incomplete'}:\n                break\n    except Exception as error:\n        result = {'type': type(error).__name__, 'message': str(error), 'identifier': getattr(error, 'identifier', None)}\n    finally:\n        if callable(close):\n            maybe = close()\n            if maybe is not None:\n                await maybe\n";
    for source in [
        source.to_owned(),
        source
            .replace("        async for event in stream:", "        stream = await owner.open()\n        async for event in stream:\n            payload = owner.convert(event)")
            .replace("    finally:\n", "    finally:\n        close = getattr(stream, 'close', None)\n")
            + "    return owner.result(payload)\n",
    ] {
        assert_matches_cpython(&source, "async_iterator_cleanup.py", &LEVELS);
    }
}

#[test]
fn managed_guarded_recovery_preserves_finalizer_borrowing() {
    for named in [false, true] {
        for raising in 0..3 {
            for cleanup in 0..3 {
                let mut source = String::from(
                    "def probe(items, manager, first, second, worker, guard):\n    with manager():\n        try:\n            worker.run(items)\n        except LookupError",
                );
                if named {
                    source.push_str(" as error");
                }
                let value = if named { "error" } else { "items" };
                source.push_str(":\n            if guard(");
                source.push_str(value);
                source.push_str("):\n                raise");
                if raising != 0 {
                    let cause = if raising == 1 { "None" } else { value };
                    source.push_str(" Failure(");
                    source.push_str(value);
                    source.push_str(") from ");
                    source.push_str(cause);
                }
                source.push_str("\n        finally:\n");
                if cleanup == 2 {
                    source.push_str("            first.value = None\n");
                } else {
                    source.push_str("            first.close()\n");
                    if cleanup != 0 {
                        source.push_str("            second.finish()\n");
                    }
                }
                source.push_str("    observe(items)\n");
                assert_matches_cpython(&source, "managed_guarded_recovery.py", &LEVELS);
            }
        }
    }
}

#[test]
fn managed_loop_recovery_preserves_finalizer_borrowing() {
    let source = |loop_header: &str, prefix: &str, named: bool, recovery: bool| {
        let mut source = String::from(
            "def probe(items, manager, cleanup, worker, guard, running, advance, clock, started):\n    with manager():\n        ",
        );
        source.push_str(loop_header);
        source.push_str("\n            try:\n");
        for line in prefix.lines() {
            source.push_str("                ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("                worker.run(items)\n            except LookupError");
        if named {
            source.push_str(" as error");
        }
        source.push_str(":\n");
        if recovery {
            source.push_str("                items = []\n                worker.note(items)\n");
        }
        source.push_str("                if guard(");
        source.push_str(if named { "error" } else { "items" });
        source.push_str("):\n                    raise\n            finally:\n                cleanup.observe(clock.now() - started)\n            items = advance(items)\n    observe(items)\n");
        source
    };
    for loop_header in ["while running():", "for item in items:"] {
        for named in [false, true] {
            for recovery in [false, true] {
                assert_matches_cpython(
                    &source(loop_header, "", named, recovery),
                    "managed_loop_recovery.py",
                    &LEVELS,
                );
            }
        }
        for prefix in [
            "worker.prepare(items)\nworker.refresh(items)\n",
            "worker.value = items\n",
            "first, second = items\n",
            "prepared = [item.value for item in items]\n",
            "prepared = {item.key: item.value for item in items}\n",
            "prepared = {item.value for item in items}\n",
            "prepared = collect(item.value for item in items)\n",
            "worker.note(items if running() else None)\n",
            "for item in items:\n    worker.note(item)\n",
            "for item in items:\n    if guard(item):\n        worker.value = item\n",
            "prepared = {item.key: item for item in items}\nworker.note(item for item in prepared)\nfor item in items:\n    if guard(item) and running():\n        prepared[item.key] = item\nworker.note(items if running() else None)\n",
        ] {
            assert_matches_cpython(
                &source(loop_header, prefix, false, true),
                "managed_loop_completed_recovery.py",
                &LEVELS,
            );
        }
    }
}

#[test]
fn managed_recovery_preserves_finalizer_store_borrowing() {
    for early_return in [false, true] {
        for named in [false, true] {
            for rebound in [false, true] {
                for success in [false, true] {
                    let mut source =
                        String::from("def sample(owner, work):\n    with owner.lock:\n");
                    if early_return {
                        source.push_str(
                            "        if owner.closed or work.done():\n            return\n",
                        );
                    }
                    source.push_str("        owner.active = 1\n        try:\n            value = produce(work)\n        except Error");
                    if named {
                        source.push_str(" as error");
                    }
                    source.push_str(":\n            if not owner.ready():\n                owner.failed = True\n                report(work)\n                raise Stop()\n");
                    if rebound {
                        source.push_str("            if invalid(work):\n                code = first() if conflict(work) else second()\n                error = wrap(code)\n");
                    }
                    source.push_str(if named || rebound {
                        "            finish(work, error)\n"
                    } else {
                        "            finish(work, None)\n"
                    });
                    if success {
                        source.push_str("        else:\n            work.update({\"mode\": \"fast\" if owner.fast else \"slow\"})\n            finish(work, value)\n");
                    }
                    source.push_str("        finally:\n            owner.active = 0\n");
                    assert_matches_cpython(&source, "managed_recovery_store.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn managed_coroutine_recovery_keeps_guarded_cleanup_reachable() {
    let source = "async def sample(owner, item, outcome, persisted):\n    with owner.context():\n        try:\n            outcome = await owner.route()\n        except Exception as error:\n            if item is None or item.status != Status.PENDING:\n                raise\n        finally:\n            if outcome is not None:\n                if not persisted:\n                    await owner.persist()\n";
    for source in [
        source.to_owned(),
        source
            .replace("        try:\n            outcome", "        try:\n            await owner.persist()\n            persisted = True\n            outcome")
            .replace("        except Exception as error:\n", "        except Exception as error:\n            if owner.cancelled(error):\n                raise\n            outcome = 'failed'\n            with owner.refresh():\n                item = await owner.reload()\n")
            .replace("                raise\n        finally:", "                raise\n            outcome = await owner.recover(item)\n        finally:")
            .replace("            if outcome is not None:\n", "            if outcome is not None:\n                await owner.record(outcome)\n"),
    ] {
        assert_matches_cpython(&source, "managed_coroutine_cleanup.py", &LEVELS);
    }
}

#[test]
fn returning_recovery_keeps_linear_else_cleanup_reachable() {
    for (scope, suspend) in [("def", ""), ("async def", "await ")] {
        for suffix in ["", "        progress.failed += 1\n"] {
            let source = format!(
                "{scope} sample(owner, progress, identifier, tags):\n    try:\n        {suspend}owner.remove(identifier)\n    except Missing as error:\n        if error.status == Status.ABSENT:\n            return\n{suffix}    except Exception as error:\n        owner.log(error)\n    else:\n        owner.record('removed', tags=tags + ['success'])\n    finally:\n        progress.completed += 1\n"
            );
            assert_matches_cpython(&source, "returning_recovery_else.py", &LEVELS);
        }
    }
}

#[test]
fn awaited_branch_keeps_nested_cleanup_reachable() {
    let source = "async def sample(owner, token, checkpoint, error):\n    try:\n        if owner.cancelled:\n            await owner.abort(token)\n        elif checkpoint is None:\n            raise RuntimeError('missing checkpoint')\n    except BaseException as failure:\n        if token is not None:\n            try:\n                await owner.abort(token)\n            except Exception:\n                log('abort failed', extra={'token': token})\n    finally:\n        try:\n            await owner.close()\n        except BaseException as failure:\n            log('close failed', extra={'token': token})\n        finally:\n            if not owner.result.done():\n                if error is None:\n                    owner.result.set_result(None)\n                else:\n                    owner.result.set_exception(error)\n";
    for source in [
        source.to_owned(),
        source
            .replace("    try:\n        if owner.cancelled:", "    try:\n        if token is None:\n            raise RuntimeError('missing token')\n        await owner.ready.wait()\n        if owner.cancelled:")
            .replace("        elif checkpoint is None:\n            raise RuntimeError('missing checkpoint')", "        else:\n            checkpoint = owner.checkpoint\n            if checkpoint is None:\n                raise RuntimeError('missing checkpoint')\n            await owner.transfer(checkpoint)"),
    ] {
        assert_matches_cpython(&source, "awaited_nested_cleanup.py", &LEVELS);
    }
}

#[test]
fn nested_recovery_else_keeps_loop_cleanup_reachable() {
    let source = "async def sample(owner, entry, queue):\n    while True:\n        try:\n            if entry.observed == entry.subscribed:\n                continue\n            if entry.observed:\n                try:\n                    result = await owner.request('resume', entry.identifier, empty=True)\n                except Exception:\n                    raise\n                else:\n                    owner.publish()\n                finally:\n                    entry.resumed.set()\n            else:\n                entry.subscribed = False\n        except Exception:\n            owner.logger.error('unable to update', details=owner.details(entry))\n        finally:\n            queue.task_done()\n";
    for source in [
        source.to_owned(),
        source
            .replace("                except Exception:\n                    raise", "                except Exception:\n                    entry.observed = False\n                    raise")
            .replace("            else:\n                entry.subscribed = False", "            else:\n                await owner.request('stop', entry.identifier)\n                entry.subscribed = False"),
    ] {
        assert_matches_cpython(&source, "nested_recovery_else.py", &LEVELS);
    }
}

#[test]
fn nested_loop_recovery_keeps_cleanup_entry_cold() {
    let source = "def sample(owner, outcome, errors):\n    for identifier, session in reversed(tuple(owner.sessions.items())):\n        try:\n            if outcome.handle != session.process or outcome.status != 0:\n                raise TransportError('unexpected process exit')\n        except Exception as error:\n            try:\n                owner.transport.wait(session.process, timeout=owner.timeout)\n            except Exception as recovery_error:\n                errors.append(recovery_error)\n        finally:\n            owner.sessions.pop(identifier, None)\n";
    assert_matches_cpython(source, "nested_loop_recovery.py", &LEVELS);
}

#[test]
fn conditional_call_before_reraise_keeps_cleanup_reachable() {
    let source = "async def sample(items, values, logger, metrics, optional):\n    for item in items:\n        try:\n            values[item.name] = item.value\n        except BaseException as error:\n            logger.warning('optional item unavailable' if optional else 'item unavailable', extra=details(item=item, name=item.name))\n            if not optional:\n                raise\n        finally:\n            metrics.record('duration', elapsed, tags=tags)\n";
    for source in [
        source.to_owned(),
        source.replace(
            "            logger.warning",
            "            status = 'optional' if optional else 'required'\n            logger.warning",
        ),
        source.replace(
            "            logger.warning",
            "            optional = item.optional and isinstance(error, ValueError)\n            status = 'optional' if optional else 'required'\n            logger.warning",
        ),
    ] {
        assert_matches_cpython(&source, "conditional_handler_call.py", &LEVELS);
        assert_matches_cpython(
            &source.replace("async def", "def"),
            "ordinary_handler_call.py",
            &LEVELS,
        );
    }
}

#[test]
fn asserted_async_iterator_break_keeps_cleanup_cold() {
    let source = "async def sample(processor, owner):\n    stream = processor.open(owner=owner, events=events())\n    try:\n        async with timeout(1):\n            async for item in stream:\n                if item.channel == 'final':\n                    assert item.parts == ['ready']\n                    break\n    finally:\n        await stream.close()\n";
    for source in [
        source.to_owned(),
        source.replace("item.parts == ['ready']", "ready(item)"),
        source.replace("item.parts == ['ready']", "item is not None"),
    ] {
        assert_matches_cpython(&source, "asserted_async_iterator_break.py", &LEVELS);
    }
}

#[test]
fn nested_reraise_cleanup_keeps_awaited_store_reachable() {
    let source = "async def sample():\n    try:\n        if native:\n            await session.close()\n        else:\n            await session.detach()\n    except CancelledError as error:\n        raise\n    except OwnershipError as error:\n        recovered = error\n    finally:\n        try:\n            if not native:\n                if session.reusable:\n                    await pool.release(lease)\n                else:\n                    retired = await replace(pool, name=lease.name, expected=lease.backend, lease=lease)\n                    if retired and isinstance(pool, Pool):\n                        disposition = 'retired'\n        except BaseException as failure:\n            raise\n";
    assert_matches_cpython(source, "nested_reraise_cleanup.py", &LEVELS);
}

#[test]
fn nested_manager_constants_keep_exception_ownership() {
    let body = "with manager:\n    if condition:\n        work()\n        'comment'\n";
    for prefix in ["", "def sample():\n", "async def sample():\n"] {
        let mut source = String::from(prefix);
        let indent = if prefix.is_empty() { "" } else { "    " };
        for line in body.lines() {
            writeln!(source, "{indent}{line}").unwrap();
        }
        assert_matches_cpython(&source, "nested_manager_constant.py", &LEVELS);
    }
}

#[test]
fn erased_nested_cleanup_continuations_keep_markers() {
    for source in [
        "async def sample(followup, mode, resources):\n    async with session():\n        try:\n            if followup:\n                if mode == 'quiet':\n                    assert ready()\n                else:\n                    assert expected()\n                return\n        finally:\n            for identifier in resources:\n                if state.get('status') not in {'failed', 'cancelled', 'completed'}:\n                    await client.cancel(identifier)\n",
        "async def sample(state, status, clients):\n    try:\n        if state != 'none':\n            if status is None:\n                assert missing()\n            else:\n                assert valid()\n            assert stable()\n        if status is not None:\n            assert cleaned()\n        elif state != 'none':\n            assert expected()\n    finally:\n        for client in clients:\n            await client.close()\n",
        "async def sample(mode, denied, trusted, cause, clients):\n    try:\n        with first(), second(), third():\n            if mode == 'hit':\n                assert ready()\n            elif denied:\n                assert expected()\n            else:\n                if trusted and cause is not None:\n                    assert valid()\n                else:\n                    assert absent()\n                assert stable()\n    finally:\n        for client in clients:\n            await client.close()\n",
        "async def sample(mode, trusted, clients):\n    try:\n        with first(), second(), third():\n            if mode:\n                assert ready()\n            else:\n                if trusted:\n                    assert valid()\n                else:\n                    assert absent()\n                detail = encode(errors)\n    finally:\n        for client in clients:\n            await client.close()\n",
    ] {
        assert_matches_cpython(source, "nested_cleanup_continuation.py", &LEVELS);
    }
}

#[test]
fn erased_nested_cleanup_joins_keep_markers() {
    for source in [
        "async def sample(followup, mode, resources):\n    async with session():\n        try:\n            if followup:\n                if mode == 'quiet':\n                    assert output.items == ['result']\n                else:\n                    assert output.text or output.questions\n        finally:\n            for identifier in resources:\n                state = await client.fetch(identifier)\n",
        "async def sample(mode, denied, clients):\n    try:\n        with first(), second(), third():\n            if mode == 'hit':\n                assert len(calls) == 2\n            elif denied:\n                assert len(calls) == 1\n            else:\n                assert all(call.content for call in calls)\n    finally:\n        for client in clients:\n            await client.close()\n",
        "async def sample(state, status, clients):\n    try:\n        if state != 'none':\n            if status is None:\n                assert missing()\n            else:\n                assert valid()\n        if status is not None:\n            assert cleaned()\n        elif state != 'none':\n            assert expected()\n    finally:\n        for client in clients:\n            await client.close()\n",
    ] {
        assert_matches_cpython(source, "nested_cleanup_join.py", &LEVELS);
    }
}

#[test]
fn erased_conditional_before_cleanup_iterator_keeps_join_marker() {
    for scope in ["def", "async def"] {
        for cleanup in [
            "task.cancel()",
            "if not task.done():\n                task.cancel()",
        ] {
            let source = format!(
                "{scope} sample(kind, captured, failure, tasks):\n    try:\n        if kind == 'cancelled':\n            assert isinstance(captured, (CancelledError, TransportError))\n        else:\n            assert captured.__cause__ is failure\n    finally:\n        for task in tasks:\n            {cleanup}\n"
            );
            assert_matches_cpython(&source, "erased_cleanup_iterator.py", &LEVELS);
        }
    }
}

#[test]
fn erased_coroutine_joins_before_live_successors_keep_markers() {
    for source in [
        "async def sample(grant, clock):\n    for index in range(6):\n        if grant.lane is Lane.MAIN:\n            assert await store.finish(grant, outcome='complete')\n        else:\n            assert await store.finish(grant, retry_after=1)\n        clock[0] += 1\n",
        "async def sample(grant, moving, clock):\n    for turn in range(8):\n        if grant.identifier == moving.identifier and grant.lane is Lane.WAIT:\n            assert await store.finish(grant, handoff=next_stage)\n        elif grant.identifier == moving.identifier and turn == 5:\n            assert await store.finish(grant, retry_after=1)\n        else:\n            assert await store.finish(grant, outcome='complete')\n        clock[0] += 1\n",
        "async def sample(mode, task, blocker):\n    try:\n        if mode == 'readonly':\n            assert all(item.valid for item in warning.calls)\n        else:\n            assert warning.kind == 'timeout'\n    finally:\n        if task is not None:\n            await gather(task, return_exceptions=True)\n        if blocker is not None:\n            await blocker.close()\n",
        "async def sample(mode, stream):\n    with travel(now, tick=False):\n        try:\n            if mode.startswith('invalid'):\n                assert str(operation.error) == 'invalid'\n            else:\n                assert operation.error is original_error\n        finally:\n            await stream.close(identifier, replacement)\n",
        "async def sample(owner, actor, client):\n    try:\n        async with intercept() as mocked:\n            if owner is not None:\n                assert request['account'] == expected_account\n            else:\n                assert 'owner' not in request\n            if actor is not None:\n                assert 'project' not in request\n            else:\n                assert 'actor' not in request\n    finally:\n        await client.close()\n",
    ] {
        assert_matches_cpython(source, "erased_coroutine_successor.py", &LEVELS);
    }
}

#[test]
fn erased_terminal_async_manager_join_keeps_marker() {
    for binding in ["", " as data"] {
        for condition in ["mode", "not mode", "mode is None"] {
            let source = format!(
                "async def sample(mode, writer):\n    async with runtime() as server:\n        async with connect(server) as control:\n            try:\n                async with channel(){binding}:\n                    if {condition}:\n                        assert ready()\n                    else:\n                        assert expected()\n                while True:\n                    if cancelled:\n                        break\n            finally:\n                await close(writer)\n"
            );
            assert_matches_cpython(&source, "erased_terminal_async_join.py", &LEVELS);
        }
    }
}

#[test]
fn erased_finalizer_keeps_managed_return_copy() {
    let source = "async def sample(owner):\n    try:\n        with prepare_scope(owner, search=False):\n            return await prepare(owner, Context())\n    finally:\n        assert enabled()\n";
    assert_matches_cpython(source, "erased_managed_return.py", &LEVELS);
}

#[test]
fn pass_recovery_in_handler_iterator_keeps_cold_exit() {
    let source = r#"def sample(storage, root, label, owner, prefix=None, prompt=None, replacement=None, *, register=None):
    with archive.open(fileobj=stream, mode="w:gz") as bundle:
        for path in sorted(root.walk("*")):
            if not path.is_file() or not (name == "manifest.json" or name.startswith(("assets/", "jobs/", "static/"))):
                continue
    try:
        storage.upload(name, encode(entry), overwrite=False)
    except AlreadyExists:
        if existing != entry:
            for revision in (existing, entry):
                try:
                    storage.upload(history, encode(revision), overwrite=False)
                except AlreadyExists:
                    pass
    logger.info("Recorded %s", label)
"#;
    for source in [
        source.to_owned(),
        source
            .replace("                try:\n", "                history = location(revision)\n                try:\n")
            .replace("                    pass\n", "                    pass\n            storage.upload(name, encode(entry), overwrite=True)\n"),
    ] {
        assert_matches_cpython(&source, "iterator_handler_recovery.py", &LEVELS);
        assert_matches_cpython(
            &source.replace("def sample", "async def sample"),
            "coroutine_iterator_handler_recovery.py",
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditional_join_inside_async_manager_loop_keeps_marker() {
    for iterator in ["for", "async for"] {
        for binding in ["", " as (channel, identity)"] {
            for cleanup in ["release.set()", "record()\nrelease.set()"] {
                let cleanup = cleanup.replace('\n', "\n                ");
                let source = format!(
                    "async def sample(flag, items):\n    {iterator} gate in items:\n        async with acquire(){binding}:\n            try:\n                if flag:\n                    assert ready()\n                else:\n                    assert complete()\n            finally:\n                {cleanup}\n"
                );
                assert_matches_cpython(&source, "async_loop_conditional_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_loop_break_keeps_async_manager_finally_boundary() {
    for setup in ["", "                event = await next_event(channel)\n"] {
        for condition in [
            "event['kind'].startswith('done')",
            "event['kind'] in {'done', 'failed'}",
        ] {
            let source = format!(
                "async def sample():\n    async with acquire() as (channel, identity):\n        try:\n            while True:\n{setup}                if {condition}:\n                    break\n                assert waiting()\n            assert complete()\n        finally:\n            release.set()\n        events = await receive(channel)\n"
            );
            assert_matches_cpython(&source, "async_loop_finally_boundary.py", &LEVELS);
        }
    }
}

#[test]
fn tuple_unpack_prefix_keeps_retained_setup_marker_owner() {
    let source = r#"async def sample(healthy):
    group = attach({**originals, **({"peer": peer} if healthy else {})})
    waiting, release = event(), event()
    started, resume, finished = event(), event(), event()
    failed, retry = event(), event()

    async def stage():
        await resume.wait()
        await retry.wait()

    async def close():
        await release.wait()

    initial = create_task(stage(inputs=[bundle()]))
    try:
        if healthy:
            assert ready()
    finally:
        await pool.close()
"#;
    assert_matches_cpython(source, "tuple_unpack_cleanup_owner.py", &LEVELS);
}

#[test]
fn erased_coroutine_join_preserves_linear_await_successors() {
    for awaited in ["action(state)", "state", "state.finish(value)"] {
        for managers in [1, 2] {
            for prefix in ["fetch(items)", "next(part for part in items if part.ready)"] {
                let mut source = String::from("async def sample(value, state, items):\n");
                let mut indent = String::from("    ");
                for _ in 0..managers {
                    writeln!(source, "{indent}async with manager(state):").unwrap();
                    indent.push_str("    ");
                }
                write!(
                    source,
                    "{indent}try:
{indent}    result = {prefix}
{indent}    if value == state:
{indent}        assert result
{indent}    else:
{indent}        assert result is None
{indent}    await {awaited}
{indent}finally:
{indent}    await finish(state)
"
                )
                .unwrap();
                assert_matches_cpython(&source, "erased_join_await.py", &LEVELS);
            }
        }
    }
}

#[test]
fn managed_erased_coroutine_join_preserves_linear_await_successors() {
    for outer in ["with", "async with"] {
        for inner in ["with", "async with"] {
            for awaited in [
                "action(state)",
                "state",
                "state.finish(value)",
                "action(await prepare(state))",
                "state.finish(await prepare(value))",
                "manager(state).finish(await prepare(value))",
            ] {
                let source = format!(
                    "async def sample(value, state):
    {outer} manager(value):
        try:
            {inner} guard(state):
                with capture_error() as error:
                    await prepare(value)
                if value:
                    assert ready(state)
                else:
                    assert accepted(error)
                assert retained(state)
                await {awaited}
        finally:
            finish(state)
"
                );
                assert_matches_cpython(&source, "managed_erased_join_await.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_async_iterator_tails_preserve_predating_manager_exits() {
    let source = r"async def probe(value):
    OWNER outer():
        try:
            with guard():
                async for value in left():
                    BODY
            assert accepted()
            async for value in right():
                BODY
            assert settled()
        finally:
            CLEANUP
";
    for owner in ["with", "async with"] {
        for body in ["pass", "consume(value)"] {
            for cleanup in ["finish()", "await finish()"] {
                let source = source
                    .replace("OWNER", owner)
                    .replace("BODY", body)
                    .replace("CLEANUP", cleanup);
                for (name, source) in [
                    ("erased_tail", source.clone()),
                    ("live_tail", source.replace("assert settled()", "advance()")),
                ] {
                    assert_matches_cpython(
                        &source,
                        &format!("managed_async_iterator_{name}.py"),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn erased_coroutine_join_preserves_predating_async_managers() {
    let source = r"async def sample(value, state, items):
    async with manager(value):
        async with manager(state):
            try:
                result = next(part for part in items if part.ready)
                if value == state:
                    assert result
                else:
                    assert result is None
                action(state)
                if state:
                    action(value)
                await finish(value)
            finally:
                await finish(state)
";
    for (name, source) in [
        ("generator_prefix", source.to_owned()),
        (
            "plain_prefix",
            source.replace("next(part for part in items if part.ready)", "fetch(items)"),
        ),
        ("truth_test", source.replace("value == state", "value")),
        (
            "terminal_join",
            source.replace(
                "                action(state)\n                if state:\n                    action(value)\n                await finish(value)\n",
                "",
            ),
        ),
        (
            "later_finalizer_manager",
            source.replace(
                "                await finish(state)\n",
                "                await finish(state)\n                with manager(value):\n                    await finish(value)\n",
            ),
        ),
        (
            "authored_pass",
            source
                .replace("assert result is None", "pass")
                .replace("assert result", "pass"),
        ),
        (
            "live_true_arm",
            source.replace("assert result\n", "action(result)\n"),
        ),
        (
            "live_false_arm",
            source.replace("assert result is None", "action(state)"),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("erased_coroutine_predating_managers_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn folded_while_breaks_copy_unnamed_handler_cleanup() {
    let source = r"def sample(values, limit):
    try:
        rows = fetch(values)
    except ValueError:
        while True:
            rows = fetch(values)
            if len(rows) > limit:
                raise RuntimeError('too many rows') from None
            consume(rows)
            if len(rows) < limit:
                break
    return rows
";
    for (name, source) in [
        ("plain", source.to_owned()),
        (
            "try_else",
            source.replace(
                "    return rows",
                "    else:\n        consume(rows)\n    return rows",
            ),
        ),
        (
            "prefix",
            source.replace(
                "        while True:",
                "        total = len(values)\n        while True:",
            ),
        ),
        (
            "followup",
            source.replace("    return rows", "    finish(rows)\n    return rows"),
        ),
    ] {
        assert_matches_cpython(&source, &format!("folded_handler_break_{name}.py"), &LEVELS);
    }
}

#[test]
fn named_handler_none_while_break_copies_cleanup() {
    for prefix in ["", "async "] {
        for condition in ["value", "value is None", "value is not None"] {
            for work in ["", "                consume(value)\n"] {
                let operation = if prefix.is_empty() {
                    "action(value)"
                } else {
                    "await action(value)"
                };
                let source = format!(
                    "{prefix}def sample(value):
    try:
        {operation}
    except ValueError as error:
        while {condition}:
            if check(value):
{work}                break
            value = advance(value)
    return finish(value)
"
                );
                assert_matches_cpython(&source, "named_handler_none_while.py", &LEVELS);
            }
        }
    }
}

#[test]
fn retry_breaks_copy_named_handler_interrupt_checks() {
    let source = r"def probe(values, token, item):
    records = []
    saved = None
    def transform(item):
        try:
            output = normalize(item)
        except RuntimeError as error:
            record(error)
            raise
        publish(output)
        return output
    try:
        for item in values:
            token = reserve(item)
            records.append((item, token, None))
            schedule(item, token)
    except RuntimeError as error:
        saved = error
        while True:
            try:
                release()
            except RuntimeError:
                continue
            break
    if saved is not None:
        raise saved
";
    assert_matches_cpython(source, "named_handler_retry_break.py", &LEVELS);
}

#[test]
fn erased_loop_exit_precedes_manager_cleanup() {
    for manager in ["acquire()", "outer(), middle(), inner(), acquire()"] {
        for setup in ["", "            work()\n"] {
            for condition in ["ready(cursor)", "cursor.poll() == (True,)"] {
                let source = format!(
                    "def sample(store, dispatch):\n    with {manager} as cursor:\n        while True:\n{setup}            if {condition}:\n                break\n            assert waiting()\n    persisted = store.load(dispatch.id)\n"
                );
                assert_matches_cpython(&source, "manager_loop_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_loop_exit_stays_inside_managed_finally() {
    for manager in ["acquire()", "outer(), acquire()"] {
        for setup in ["", "                    work()\n"] {
            for condition in ["ready()", "cursor.poll() == (True,)"] {
                let source = format!(
                    "def sample(cursor):\n    try:\n        with {manager} as executor:\n            future = executor.submit(work)\n            try:\n                while True:\n{setup}                    if {condition}:\n                        break\n                    assert waiting()\n                with expecting() as error:\n                    future.result()\n            finally:\n                rollback()\n    finally:\n        close()\n"
                );
                assert_matches_cpython(&source, "managed_finally_loop_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn copied_return_through_nested_loops_keeps_none_guard_owner() {
    let source = r#"async def sample(client, response):
    for attempt in range(client.attempts):
        for redirect in range(limit):
            try:
                if response.status in redirects:
                    continue
                if response.status < 200 or response.status >= 300:
                    if response.status in retryable:
                        if attempt < client.attempts:
                            break
                return bytes(content)
            except TransportError as error:
                break
            finally:
                if response is not None:
                    await response.close()
        else:
            raise AssertionError("unreachable")
"#;
    for (outer, inner) in [
        (
            "for attempt in range(client.attempts):",
            "for redirect in range(limit):",
        ),
        (
            "for attempt in range(client.attempts):",
            "while redirected():",
        ),
        ("while active():", "while redirected():"),
    ] {
        let source = source
            .replace("for attempt in range(client.attempts):", outer)
            .replace("for redirect in range(limit):", inner);
        assert_matches_cpython(&source, "nested_loop_return_owner.py", &LEVELS);
    }
}

#[test]
fn erased_outer_finalizer_keeps_completed_cleanup_exit() {
    for source in [
        "def sample(staged, engine):\n    settings = get_settings()\n    with holder():\n        try:\n            try:\n                status = cleanup(settings, stage=make_stage() if staged else None)\n            finally:\n                remove_listener(engine, 'event', listener)\n        finally:\n            assert release(settings)\n    inspect_settings(settings)\n",
        "def sample(staged, engine):\n    settings = get_settings()\n    with holder():\n        try:\n            prepare(settings)\n            with first_session():\n                assert ready()\n            register_listener(engine)\n            try:\n                status = cleanup(settings, stage=make_stage() if staged else None)\n            finally:\n                remove_listener(engine, 'event', listener)\n            assert complete(status)\n            with last_session():\n                state = snapshot()\n                assert stable(state)\n        finally:\n            assert release(settings)\n    inspect_settings(settings)\n",
    ] {
        assert_matches_cpython(source, "erased_outer_cleanup.py", &LEVELS);
    }
}

#[test]
fn returning_else_prefix_keeps_live_cleanup_entry() {
    let source = r#"async def sample(client, path):
    for attempt in range(client.attempts):
        try:
            if response.status in retryable_statuses:
                retry_after = response.headers.get("retry")
            else:
                complete = True
                return result(attempt, size, response.headers.get("tag"), digest)
        except (TransportError, ValidationError) as error:
            last_error = error
        finally:
            if not complete:
                path.unlink(missing_ok=True)
"#;
    for scope in ["def", "async def"] {
        for prefix in [
            "complete = True",
            "record(path)\n                complete = True",
        ] {
            let source = source
                .replace("async def", scope)
                .replace("complete = True", prefix);
            assert_matches_cpython(&source, "returning_else_prefix.py", &LEVELS);
        }
    }
}

#[test]
fn erased_manager_prefix_keeps_empty_finalizer_exit() {
    for prefix in [
        "",
        "            assert held()\n",
        "            assert held()\n            assert stable()\n",
    ] {
        let source = format!(
            "def sample(settings):\n    with acquire() as holder:\n        try:\n{prefix}            with session() as connection, connection.begin():\n                assert ready()\n        finally:\n            assert release(holder)\n    inspect(settings)\n"
        );
        assert_matches_cpython(&source, "erased_manager_prefix.py", &LEVELS);
    }
}

#[test]
fn copied_named_iterator_break_keeps_none_guard_owner() {
    let source = r#"async def sample(client, response):
    for attempt in range(client.attempts):
        for redirect in range(limit):
            try:
                if response.status in redirects:
                    continue
                if response.status < 200 or response.status >= 300:
                    if response.status in retryable:
                        if attempt < client.attempts:
                            break
            except TransportError as error:
                if attempt < client.attempts:
                    with phase():
                        await client.sleep(client.delay(attempt))
                break
            finally:
                if response is not None:
                    await response.close()
        else:
            raise AssertionError("unreachable")
"#;
    for (index, source) in [
        source.to_owned(),
        source.replace("await response.close()", "response.close()"),
        source.replace("await client.sleep", "client.sleep"),
        source.replace("async def", "def").replace("await ", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("iterator_handler_break_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn returning_else_keeps_live_cleanup_entry() {
    for scope in ["def", "async def"] {
        for (binding, recovery) in [("", "report()"), (" as error", "last_error = error")] {
            for condition in ["not complete", "path is not None"] {
                for work in ["response.headers.get('retry')", "client.wait()"] {
                    let suspend = if scope == "async def" && work == "client.wait()" {
                        "await "
                    } else {
                        ""
                    };
                    let source = format!(
                        "{scope} sample(client, path):\n    for attempt in range(client.attempts):\n        try:\n            if response.status in retryable_statuses:\n                retry_after = {suspend}{work}\n            else:\n                return result(attempt, size, response.headers.get('tag'), digest)\n        except (TransportError, ValidationError){binding}:\n            {recovery}\n        finally:\n            if {condition}:\n                path.unlink(missing_ok=True)\n"
                    );
                    assert_matches_cpython(&source, "returning_else_cleanup.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn nested_handler_raise_keeps_empty_outer_exit() {
    for scope in ["def", "async def"] {
        for binding in ["", " as parse_error"] {
            for cause in ["", " from None"] {
                let suspend = if scope == "async def" { "await " } else { "" };
                let source = format!(
                    "{scope} sample(client, resource):\n    try:\n        loaded = {suspend}client.load(resource)\n    except StatusError as error:\n        try:\n            client.ensure(response)\n        except StatusError as registration_error:\n            try:\n                payload = decode(registration_error.body or '{{}}')\n            except ValueError{binding}:\n                raise registration_error{cause}\n            created = False\n        else:\n            created = True\n        loaded = {suspend}client.load(resource)\n    else:\n        created = False\n    if created and refresh.status != 'skipped':\n        return output(resource, refresh.version)\n"
                );
                assert_matches_cpython(&source, "nested_handler_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn loop_raise_comparison_chain_reaches_cleanup() {
    for scope in ["def", "async def"] {
        for condition in [
            "300 <= error.status < 400",
            "error.status in {401, 403} or 300 <= error.status < 400",
            "not (300 <= error.status < 400)",
        ] {
            for handler in [
                "",
                "        except (ClientError, TimeoutError) as error:\n            retryable = True\n",
            ] {
                let suspend = if scope == "async def" { "await " } else { "" };
                let source = format!(
                    "{scope} sample(row, path, manifest):\n    while row.attempts < limit:\n        try:\n            retryable = result.outcome == 'failed'\n        except RequestError as error:\n            if {condition}:\n                raise\n{handler}        finally:\n            save(path, manifest)\n        {suspend}sleep(2 ** (row.attempts - 1))\n"
                );
                assert_matches_cpython(&source, "loop_comparison_recovery.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_async_cleanup_after_comprehension_keeps_empty_exit() {
    for work in [
        "tasks = build()",
        "tasks = [requests.create_task(request(index)) for index in range(count)]",
    ] {
        for cleanup in ["assert healthy()", "assert healthy()\nassert complete()"] {
            let cleanup = cleanup.replace('\n', "\n        ");
            let source = format!(
                "async def sample(count):\n    try:\n        async with acquire() as requests:\n            {work}\n    finally:\n        {cleanup}\n    for task in tasks:\n        for subject in payload['history']:\n            result = int(subject['id']) if subject['id'] else None\n"
            );
            assert_matches_cpython(&source, "erased_async_comprehension_cleanup.py", &LEVELS);
        }
    }
}

#[test]
fn completed_while_prefixes_preserve_erased_exit_borrow_boundaries() {
    for coroutine in [false, true] {
        for generator in [false, true] {
            for prefix_assert in [false, true] {
                for suffix_assert in [false, true] {
                    for following in [false, true] {
                        let header = if coroutine { "async def" } else { "def" };
                        let operation = if coroutine {
                            "await obtain(cursor)"
                        } else {
                            "obtain(cursor)"
                        };
                        let mut source = format!(
                            "{header} sample(seed):\n    collected = []\n    cursor = seed\n    while True:\n        chunk = {operation}\n"
                        );
                        if prefix_assert {
                            source.push_str("        assert chunk is not None\n");
                        }
                        let payload = if generator {
                            "(element for element in chunk)"
                        } else {
                            "chunk"
                        };
                        writeln!(source, "        collected.extend({payload})").unwrap();
                        source.push_str(
                            "        cursor = next_cursor(chunk)\n        if cursor is None:\n            break\n",
                        );
                        if suffix_assert {
                            source.push_str("        assert chunk\n");
                        }
                        source.push_str("    assert collected\n");
                        if following {
                            source.push_str(if coroutine {
                                "    outcome = await finish(collected)\n"
                            } else {
                                "    outcome = finish(collected)\n"
                            });
                            source.push_str("    assert outcome\n");
                        }
                        assert_matches_cpython(&source, "completed_while_prefix.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_sync_manager_finalizer_keeps_empty_exit() {
    for source in [
        "def sample():\n    try:\n        with expecting(Failure) as error:\n            second.start()\n    finally:\n        assert first.close(1)\n    restarted = factory(lambda: store)\n    restarted.start()\n",
        "def sample():\n    try:\n        with expecting(Failure) as error:\n            second.start()\n        assert 'expected' in str(error.value)\n    finally:\n        assert first.close(1)\n    restarted = factory(lambda: store)\n    restarted.start()\n",
        "def sample(environment):\n    try:\n        with listen() as listener, pool() as executor:\n            try:\n                with publish():\n                    server.start()\n            finally:\n                assert server.stop().wait()\n    finally:\n        for key, value in environment.items():\n            if value is None:\n                reset(key)\n            else:\n                set_value(key, value)\n",
        "def sample(environment):\n    try:\n        with listen() as listener, pool() as executor:\n            try:\n                with publish():\n                    assert server.start() > 0\n            finally:\n                assert server.stop().wait()\n    finally:\n        for key, value in environment.items():\n            if value is None:\n                reset(key)\n            else:\n                set_value(key, value)\n",
        "def sample(environment):\n    try:\n        with listen() as listener, pool() as executor:\n            try:\n                with publish():\n                    assert server.start() > 0\n                    server.run()\n            finally:\n                assert server.stop().wait()\n    finally:\n        for key, value in environment.items():\n            if value is None:\n                reset(key)\n            else:\n                set_value(key, value)\n",
        "def sample(invalid, original, retained, policy):\n    with client() as connection:\n        with session(connection) as service:\n            try:\n                with expecting():\n                    release(invalid)\n            finally:\n                assert invalid == original\n            repaired = redact(retained, policy=policy)\n            with expecting():\n                validate(repaired)\n",
    ] {
        assert_matches_cpython(source, "erased_sync_manager_finalizer.py", &LEVELS);
    }
}

#[test]
fn guarded_iterator_break_reaches_recovery_cleanup() {
    for iterator in ["(1, 2)", "items"] {
        for setup in [
            "",
            "            value = run(attempt)\n            attempts.append(value)\n            assert ready(value)\n",
        ] {
            let source = format!(
                "def sample(result, items):\n    attempts = []\n    failure = None\n    try:\n        for attempt in {iterator}:\n{setup}            if not result.ready():\n                break\n            if attempt == 1:\n                report(result)\n        assert ready(attempts)\n    except BaseException as error:\n        failure = f'{{type(error).__name__}}: {{error}}'\n        raise\n    finally:\n        try:\n            upload(payload(attempts, failure=failure))\n        except Error:\n            report()\n"
            );
            assert_matches_cpython(&source, "guarded_iterator_cleanup.py", &LEVELS);
        }
    }
}

#[test]
fn erased_assertions_preserve_iterator_cleanup_marker() {
    for prefix in ["", "async "] {
        for iterator in ["(1, 2)", "items"] {
            for binding in ["", " as error"] {
                for cleanup in [
                    "finish()",
                    "try:\n    finish()\nexcept Error:\n    recover()",
                ] {
                    let cleanup = cleanup.replace('\n', "\n        ");
                    let source = format!(
                        "{prefix}def sample(result, attempts, items):\n    try:\n        for attempt in {iterator}:\n            if not result.ready():\n                break\n        assert not attempts[-1].ready(), 'failed'\n    except BaseException{binding}:\n        raise\n    finally:\n        {cleanup}\n"
                    );
                    assert_matches_cpython(&source, "erased_iterator_cleanup.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn erased_loop_keeps_manager_cleanup_join_unlocated() {
    for binding in ["", " as executor"] {
        for body in ["assert ready()", "assert ready()\nassert pending()"] {
            for cleanup in ["writer.rollback()", "record()\nwriter.rollback()"] {
                let body = body.replace('\n', "\n                    ");
                let cleanup = cleanup.replace('\n', "\n                ");
                let source = format!(
                    "def sample(writer):\n    try:\n        with acquire(){binding}:\n            try:\n                while True:\n                    {body}\n            finally:\n                {cleanup}\n    finally:\n        writer.close()\n"
                );
                assert_matches_cpython(&source, "erased_loop_manager_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_async_finalizer_keeps_empty_borrow_exit() {
    for binding in ["", " as entered"] {
        for work in ["await wait()", "value = await receive()\nconsume(value)"] {
            for cleanup in [
                "assert cleanup() is state",
                "assert state\nassert cleanup() is state",
            ] {
                let work = work.replace('\n', "\n                    ");
                let cleanup = cleanup.replace('\n', "\n                ");
                let source = format!(
                    "async def sample(state):\n    token = store.set(state)\n    try:\n        async with owner():\n            try:\n                async with connection(){binding}:\n                    {work}\n            finally:\n                {cleanup}\n    finally:\n        store.reset(token)\n"
                );
                assert_matches_cpython(&source, "erased_async_finalizer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn optimized_try_iterator_break_inherits_location() {
    for prefix in ["", "async "] {
        for iterator in ["(1, 2)", "items"] {
            for binding in ["", " as error"] {
                let source = format!(
                    "{prefix}def sample(result, attempts, items):\n    try:\n        for attempt in {iterator}:\n            if not result.ready():\n                break\n        assert not attempts[-1].ready(), 'failed'\n    except BaseException{binding}:\n        raise\n"
                );
                assert_matches_cpython(&source, "optimized_try_iterator.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_recovery_alternative_keeps_cleanup_live() {
    for class in [false, true] {
        for condition in ["outer", "ready(state)"] {
            for binding in ["", " as error"] {
                for setup in ["", "                pending = prepare(state)\n"] {
                    for prefix in [
                        "",
                        "            try:\n                active = current(state)\n            except Error:\n                active = None\n",
                    ] {
                        let cleanup = if class {
                            "super().close()"
                        } else {
                            "cleanup(state)"
                        };
                        let source = format!(
                            "def sample(state):\n    try:\n        if {condition}:\n{prefix}            if inner:\n                work(state)\n            else:\n{setup}                try:\n                    finish(state)\n                except RecoveryError:\n                    recover(state)\n        else:\n            finish(state)\n    except Error{binding}:\n        report(state)\n    finally:\n        {cleanup}\n"
                        );
                        let source = if class {
                            format!(
                                "class Worker(Parent):\n    {}\n",
                                source.trim_end().replace('\n', "\n    ")
                            )
                        } else {
                            source
                        };
                        assert_matches_cpython(&source, "nested_recovery_alternative.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn asserted_manager_else_reaches_awaited_cleanup() {
    for body in [
        "        async with client as entered:\n            if failed:\n                raise failure\n",
        "        async with acquire():\n            async with client as entered:\n                if failed:\n                    raise failure\n",
    ] {
        for condition in ["not failed", "failed is False"] {
            for prefix in ["", "        finish()\n"] {
                let source = format!(
                    "async def sample(client, backends, failed):\n    try:\n{body}    except RuntimeError as error:\n        assert failed and error is failure\n    else:\n        assert {condition}\n    finally:\n{prefix}        await client.close()\n    for backend in backends.values():\n        backend.close()\n"
                );
                assert_matches_cpython(&source, "asserted_manager_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn awaited_else_assertion_keeps_following_loads_live() {
    for operator in ["is", "is not"] {
        for arguments in ["index.id", "index.id, bypass=True"] {
            let source = format!(
                "async def sample(index, account, created):\n    try:\n        await cleanup()\n    except RetryError:\n        pass\n    else:\n        assert await get_index({arguments}) {operator} None\n    replacement = await load(account)\n    assert await exists(replacement.id, created.id)\n"
            );
            assert_matches_cpython(&source, "awaited_else_assertion.py", &LEVELS);
        }
    }
}

#[test]
fn completed_loop_work_reaches_boolean_cleanup() {
    for (prefix, awaited) in [("", false), ("async ", false), ("async ", true)] {
        for work in [
            "pending = state.request(payload)",
            "if enabled:\n                pending = state.request(payload)",
            "if enabled:\n                pending = state.request(payload)\n            else:\n                pending = state.request(other)",
        ] {
            for guard in [
                "pending is not None",
                "pending is not None and not pending.done()",
                "pending is None or not pending.done()",
            ] {
                let completion = if awaited {
                    "response = await pending.wait()\n            counts.misses += misses"
                } else {
                    "counts.misses += misses"
                };
                let source = format!(
                    "{prefix}def sample(state):\n    while clock.time() < deadline:\n        try:\n            {work}\n            {completion}\n        except ServerError as error:\n            if code in ('DENIED', 'INVALID'):\n                raise RuntimeError(code) from None\n        finally:\n            if {guard}:\n                pending.cancel()\n"
                );
                assert_matches_cpython(&source, "loop_boolean_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn async_iterator_early_break_survives_later_continues() {
    for setup in ["", "            assert channel is not None\n"] {
        for alternate in ["continue", "return value"] {
            let source = format!(
                "async def sample(channel, notify):\n    failure = None\n    try:\n        while True:\n{setup}            async for message in channel:\n                if message.closed:\n                    break\n                value = decode(message)\n                if value.skip:\n                    {alternate}\n                await deliver(value)\n                if value.failed:\n                    break\n            if not channel.active:\n                break\n            await channel.reconnect()\n    except Cancelled:\n        raise\n    except Error as error:\n        failure = error\n    finally:\n        if not channel.stopping:\n            await notify(failure or RuntimeError('disconnected'))\n"
            );
            assert_matches_cpython(&source, "async_iterator_early_break_cleanup.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_break_reaches_outer_loop_cleanup() {
    for guarded in [false, true] {
        for reconnect in ["channel.reconnect()", "await channel.reconnect()"] {
            for argument in ["failure", "failure or RuntimeError('disconnected')"] {
                let inner =
                    "if not await channel.acknowledge(channel.position):\n    break\ncontinue\n";
                let inner = if guarded {
                    format!(
                        "if channel.active and position <= channel.position:\n    {}\n",
                        inner.trim_end().replace('\n', "\n    ")
                    )
                } else {
                    inner.to_string()
                };
                let source = format!(
                    "async def sample(channel, notify):\n    try:\n        while True:\n            async for message in channel:\n                {}\n            if not channel.active:\n                break\n            {reconnect}\n    except Cancelled:\n        raise\n    except Error as error:\n        failure = error\n    finally:\n        if not channel.stopping:\n            await notify({argument})\n",
                    inner.trim_end().replace('\n', "\n                ")
                );
                assert_matches_cpython(&source, "async_iterator_outer_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn guarded_break_keeps_outer_finalizer_boundary() {
    for depth in [1, 2] {
        for binding in ["", " as error"] {
            let mut body = format!(
                "while enabled(runtime) or state.index < runtime['limit']:\n    try:\n        completed = work(**options)\n    except (Error, RuntimeError){binding}:\n        if not ready():\n            break\n    finally:\n        if ready():\n            copy(state, runtime)\n"
            );
            for _ in 0..depth {
                body = format!(
                    "try:\n    {}\nfinally:\n    state.restore()\n",
                    body.trim_end().replace('\n', "\n    ")
                );
            }
            let source = format!(
                "def sample(runtime, state, **options):\n    {}\n",
                body.trim_end().replace('\n', "\n    ")
            );
            assert_matches_cpython(&source, "guarded_break_outer_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn loop_recovery_reaches_managed_cleanup_under_finalizers() {
    for depth in [0, 1, 2] {
        for managers in [
            "left() as source",
            "left() as source, right() as destination",
        ] {
            for binding in ["", " as error"] {
                let mut body = format!(
                    "while enabled(runtime) or state.index < runtime['limit']:\n    try:\n        completed = work(**options)\n    except (Error, RuntimeError){binding}:\n        if not ready():\n            raise\n    finally:\n        if ready():\n            with {managers}:\n                copy(source, state)\n"
                );
                for _ in 0..depth {
                    body = format!(
                        "try:\n    {}\nfinally:\n    state.restore()\n",
                        body.trim_end().replace('\n', "\n    ")
                    );
                }
                let source = format!(
                    "def sample(runtime, state, **options):\n    {}\n",
                    body.trim_end().replace('\n', "\n    ")
                );
                assert_matches_cpython(&source, "loop_managed_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_loop_cleanup_recovery_keeps_following_loads_live() {
    for depth in [1, 2, 3] {
        for binding in ["", " as error"] {
            let mut body = format!(
                "try:\n    result['ready'] = True\nexcept (Error, Interrupt) as error:\n    if isinstance(error, Interrupt):\n        raise\nfinally:\n    try:\n        result['released'] = resource.release()\n    except Error{binding}:\n        resource.report()\n    record(item, result)\n"
            );
            for _ in 0..depth {
                body = format!(
                    "for item in resource.items:\n    {}",
                    body.replace('\n', "\n    ")
                );
            }
            let source = format!(
                "def sample(resource, result):\n    {}\n",
                body.replace('\n', "\n    ")
            );
            assert_matches_cpython(&source, "nested_loop_cleanup_recovery.py", &LEVELS);
        }
    }
}

#[test]
fn nested_loop_cleanup_store_keeps_following_loads_live() {
    for depth in [1, 2, 3] {
        for value in [
            "resource.ready and resource.enabled",
            "all(item['passed'] for item in report['cases'])",
            "len(report['cases']) == len(resource.items) and all(item['passed'] for item in report['cases'])",
        ] {
            let mut body = format!(
                "try:\n    result['passed'] = True\nexcept (Error, Interrupt) as error:\n    if isinstance(error, Interrupt):\n        raise\nfinally:\n    report['passed'] = {value}\n    record(item, result)\n"
            );
            for _ in 0..depth {
                body = format!(
                    "for item in resource.items:\n    {}",
                    body.replace('\n', "\n    ")
                );
            }
            let source = format!(
                "def sample(resource, result, report):\n    {}\n",
                body.replace('\n', "\n    ")
            );
            assert_matches_cpython(&source, "nested_loop_cleanup_store.py", &LEVELS);
        }
    }
}

#[test]
fn pre_iterator_return_keeps_normal_cleanup_entry() {
    for prefix in ["", "async "] {
        let awaited = if prefix.is_empty() { "" } else { "await " };
        for value in ["", " resource"] {
            for cleanup in [
                "samples = (item for group in resource.groups for item in group.items)\n        all_samples = list(samples)".to_owned(),
                format!("if not resource.dry_run:\n            if resource.ready:\n                {awaited}resource.release(state)"),
            ] {
                let source = format!(
                    "{prefix}def sample(resource, kinds, state):\n    try:\n        if resource.cleanup_only:\n            return{value}\n        for kind in (None, *kinds):\n            if kind is not None and resource.enabled(kind):\n                del state\n    except Exception as error:\n        if resource.cancelled(error):\n            raise\n    finally:\n        {cleanup}\n"
                );
                assert_matches_cpython(&source, "pre_iterator_return_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn exhausted_iterator_reaches_guarded_cleanup() {
    for prefix in ["", "async "] {
        let awaited = if prefix.is_empty() { "" } else { "await " };
        for condition in ["enabled", "not resource.dry_run"] {
            for nested in [false, true] {
                let cleanup = if nested {
                    format!("if resource.ready:\n                {awaited}resource.release(state)")
                } else {
                    format!("{awaited}resource.release(state)")
                };
                let source = format!(
                    "{prefix}def sample(resource, kinds, state, enabled):\n    try:\n        for kind in (None, *kinds):\n            if kind is not None and resource.enabled(kind):\n                del state\n    except Exception as error:\n        if resource.cancelled(error):\n            raise\n    finally:\n        if {condition}:\n            {cleanup}\n"
                );
                assert_matches_cpython(&source, "iterator_guarded_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn exhausted_iterator_reaches_linear_cleanup_stores() {
    for prefix in ["", "async "] {
        for value in [
            "resource.samples()",
            "(sample for group in resource.groups for sample in group.samples)",
        ] {
            for handler in ["pass", "if resource.cancelled(error):\n            raise"] {
                for tail in [
                    "",
                    "        resource.total = sum(item.count for item in all_samples)\n",
                ] {
                    let source = format!(
                        "{prefix}def sample(resource, kinds, state, metadata):\n    try:\n        for kind in (None, *kinds):\n            if kind is not None and resource.enabled(kind):\n                del state, metadata\n    except Exception as error:\n        {handler}\n    finally:\n        samples = {value}\n        all_samples = list(samples)\n{tail}"
                    );
                    assert_matches_cpython(&source, "iterator_cleanup_stores.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn awaited_while_reaches_nested_cleanup_setup() {
    for condition in [
        "await resource.ready(state)",
        "not await resource.ready(state)",
    ] {
        for setup in ["", "state = await resource.obtain()\n"] {
            for tail in ["", "outcome = 'ready'\n"] {
                for handler in [
                    "resource.warn(error)\n",
                    "if resource.cancelled(error):\n    raise\nif not isinstance(error, Exception):\n    raise\nresource.warn(error)\n",
                ] {
                    for cleanup in [
                        "try:\n    await resource.release(state)\nexcept Error:\n    pass\n",
                        "try:\n    if state is not None:\n        task = resource.create_task(resource.release(state))\n        try:\n            await resource.shield(task)\n        except CancelledError:\n            raise\nexcept Exception:\n    resource.warn(state)\n",
                    ] {
                        let body = format!(
                            "{setup}while {condition}:\n    await resource.pause()\n{tail}"
                        );
                        let source = format!(
                            "async def sample(resource, state):\n    try:\n        {}\n    except BaseException as error:\n        {}\n    finally:\n        {}\n",
                            body.replace('\n', "\n        "),
                            handler.replace('\n', "\n        "),
                            cleanup.replace('\n', "\n        "),
                        );
                        assert_matches_cpython(&source, "awaited_loop_cleanup.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn awaited_reraise_keeps_normal_cleanup_entry() {
    for condition in ["state is not None", "enabled"] {
        for handler in ["except:", "except CancelledError:"] {
            for setup in [
                "",
                "task = resource.create_task(resource.release(state))\n                ",
            ] {
                for cleanup in [
                    "resource.record({'state': state.value if state else None, 'failed': failed})",
                    "resource.record(state, failed)",
                ] {
                    for work in [
                        "if resource.enabled() and resource.ready():\n            await resource.work(state)",
                        "if enabled:\n            return",
                    ] {
                        let source = format!(
                            "async def sample(resource, state, task, enabled, failed):\n    try:\n        {work}\n    finally:\n        try:\n            if {condition}:\n                {setup}try:\n                    await resource.shield(task)\n                {handler}\n                    raise\n        except Exception:\n            raise\n        finally:\n            {cleanup}\n"
                        );
                        assert_matches_cpython(&source, "awaited_recovery_entry.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_raise_reaches_nested_cleanup_setup() {
    for prefix in ["", "async "] {
        for recovery in ["raise", "return state"] {
            let awaited = if prefix.is_empty() { "" } else { "await " };
            for cleanup in [
                format!("try:\n    {awaited}resource.release(state)\nexcept Error:\n    pass\n"),
                format!(
                    "try:\n    if state is not None:\n        cleanup = resource.create_task(resource.release(state))\n        try:\n            {awaited}resource.shield(cleanup)\n        except CancelledError:\n            raise\nexcept Exception:\n    resource.warn(state)\n"
                ),
                "try:\n    resource.release(state)\nfinally:\n    resource.record(state)\n"
                    .to_owned(),
            ] {
                let source = format!(
                    "{prefix}def sample(resource, state):\n    try:\n        pass\n    except BaseException as error:\n        if not isinstance(error, Exception):\n            {recovery}\n    finally:\n        {}\n",
                    cleanup.replace('\n', "\n        ")
                );
                assert_matches_cpython(&source, "nested_cleanup_setup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn branching_finally_call_keeps_following_return_boundary() {
    for (prefix, work) in [
        ("", "resource.refresh(key)"),
        ("", "(yield resource.refresh(key))"),
        ("async ", "await resource.refresh(key)"),
    ] {
        for recovery in [false, true] {
            for cleanup in [
                "resource.record(f\"status:{'good' if not failed else 'bad'}\", f\"error:{failed}\")",
                "resource.record(failed or key, cached)",
                "(resource.record if failed else resource.other)(key, cached)",
                "resource.record(*(cached if failed else items))",
            ] {
                let mut body = format!(
                    "try:\n    fresh = {work}\nexcept TimeoutError:\n    return cached.value\nexcept Error:\n    return cached.value\nreturn fresh\n"
                );
                if recovery {
                    body = format!(
                        "try:\n    {}\nexcept Error as error:\n    resource.report(error)\n",
                        body.replace('\n', "\n    ")
                    );
                }
                let source = format!(
                    "{prefix}def sample(resource, key, failed, cached, items):\n    try:\n        {}\n    finally:\n        {cleanup}\n",
                    body.replace('\n', "\n        ")
                );
                assert_matches_cpython(&source, "branching_call_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn completed_comparison_keeps_async_break_boundary() {
    for manager in ["with", "async with"] {
        for prefix in [false, true] {
            for condition in [
                "str(item.get('status') or '').lower() != 'ready'",
                "'ready' != normalize(item.flag or item.backup)",
                "normalize(item.flag or item.backup) == transform(item.kind or item.other)",
                "normalize(await item.check() or item.flag) == value",
                "normalize(item.flag if active else item.other) is None",
                "normalize(item.flag or item.backup) in values",
                "(item.flag or item.backup) == item.ready",
                "await normalize(item.flag or item.backup) != value",
            ] {
                let body = if prefix {
                    format!(
                        "if item.ready():\n    if {condition}:\n        raise Error()\n    break\n"
                    )
                } else {
                    format!("if {condition}:\n    break\n")
                };
                let source = format!(
                    "async def sample(resource, items):\n    try:\n        {manager} resource.open():\n            async for item in items:\n                {}\n            result = resource.value()\n            return build(result)\n    except TimeoutError as error:\n        raise Error() from error\n",
                    body.replace('\n', "\n                ")
                );
                assert_matches_cpython(&source, "completed_comparison_break.py", &LEVELS);
            }
        }
    }
}

#[test]
fn raising_async_iterator_else_keeps_empty_break_exit() {
    for generator in [false, true] {
        for depth in [0, 1, 3] {
            for otherwise in [
                "raise Error()\n",
                "raise Error() from cause\n",
                "resource.record()\nraise Error()\n",
                "await resource.record()\nraise Error()\n",
            ] {
                let work = if generator {
                    "yield item"
                } else {
                    "await resource.consume(item)"
                };
                let mut suite = format!(
                    "async with resource.timer():\n    async for item in items:\n        if ready:\n            break\n    else:\n        {}\nasync for item in items:\n    if active:\n        {work}\n",
                    otherwise.replace('\n', "\n        ")
                );
                for _ in 0..depth {
                    suite = format!(
                        "async with resource.open():\n    {}",
                        suite.replace('\n', "\n    ")
                    );
                }
                let source = format!(
                    "async def sample(resource, items, ready, active):\n    try:\n        {}\n    except Error:\n        raise\n",
                    suite.replace('\n', "\n        ")
                );
                assert_matches_cpython(&source, "raising_async_iterator_else.py", &LEVELS);
            }
        }
    }
}

#[test]
fn async_for_else_returns_preserve_cleanup_borrow_boundaries() {
    let source = r"async def sample(source, task):
    try:
        async for item in source:
            yield item
            if ready():
                task = make_task()
                break
        else:
            return
    finally:
        await task
";
    for yielding in [true, false] {
        let source = if yielding {
            source.to_owned()
        } else {
            source.replace("yield item", "consume(item)")
        };
        for cleanup in [
            "await task",
            "finish(task)",
            "with guard():\n            await task",
            "try:\n            await task\n        finally:\n            finish(task)",
        ] {
            assert_matches_cpython(
                &source.replace("await task", cleanup),
                "async_for_else_cleanup.py",
                &LEVELS,
            );
        }
    }
    for source in [
        source.replace("        else:\n            return\n", ""),
        source.replace("async for", "for"),
        source
            .replace("    try:\n", "")
            .replace("    finally:\n        await task\n", "")
            .replace("\n    ", "\n"),
    ] {
        assert_matches_cpython(&source, "async_for_else_boundary.py", &LEVELS);
    }
}

#[test]
fn conditional_retry_else_keeps_live_continuation() {
    for source in [
        "def sample(resource, items, enabled, ready):\n    try:\n        if enabled:\n            try:\n                result = False\n            except Error as error:\n                if any(items) and ready:\n                    try:\n                        value = resource.read(items)\n                    except LookupError:\n                        pass\n                    else:\n                        result = True\n            resource.record(result, items, ready)\n    except Error as error:\n        raise\n",
        "def sample(resource, items, ready):\n    with resource.open():\n        for item in items:\n            try:\n                result = resource.read(item, setting=ready if items else None)\n            except Error as error:\n                if ready and items:\n                    try:\n                        result = resource.read(item)\n                    except LookupError as error:\n                        failure = error\n                    else:\n                        failure = None\n            resource.record(result, item, ready)\n",
    ] {
        assert_matches_cpython(source, "conditional_retry_else.py", &LEVELS);
    }
}

#[test]
fn manager_setup_tail_keeps_none_marker_unprotected() {
    for managers in [
        ["resource.open()", "resource.open()"],
        ["resource.open() as first", "resource.open() as second"],
        [
            "resource.open() as (first, other)",
            "resource.open() as (second, extra)",
        ],
    ] {
        for comma in [false, true] {
            for condition in ["payload is None", "payload is not None"] {
                let mut suite = format!(
                    "try:\n    if {condition}:\n        resource.close()\nexcept Error as error:\n    if resource.poll() is None:\n        try:\n            resource.wait()\n        except LookupError:\n            resource.wait()\nfinally:\n    with resource.lock:\n        resource.release()\n"
                );
                let managers = if comma {
                    vec![managers.join(", ")]
                } else {
                    managers.map(str::to_owned).to_vec()
                };
                for manager in managers.iter().rev() {
                    suite = format!("with {manager}:\n    {}", suite.replace('\n', "\n    "));
                }
                let source = format!(
                    "def sample(resource, payload):\n    {}",
                    suite.replace('\n', "\n    ")
                );
                assert_matches_cpython(&source, "manager_setup_tail.py", &LEVELS);
            }
        }
    }
}

#[test]
fn linear_retry_else_stops_borrowed_continuation() {
    for coroutine in ["", "async "] {
        for handler in ["Error", "Error as error"] {
            for recovery in ["pass", "upper = middle - 1"] {
                for otherwise in ["lower = middle + 1", "resource.advance()"] {
                    for nested in [false, true] {
                        let mut body = format!(
                            "try:\n    resource.read(item, setting=configure(resource))\nexcept {handler}:\n    while lower <= upper:\n        try:\n            resource.read(item)\n        except LookupError:\n            {recovery}\n        else:\n            {otherwise}\nif upper - lower <= resource.minimum:\n    raise Error('stop')\nlower = upper - resource.overlap\n"
                        );
                        if nested {
                            body = format!(
                                "for index, item in enumerate(items):\n    while lower < len(item):\n        {}",
                                body.replace('\n', "\n        ")
                            );
                        }
                        let source = format!(
                            "{coroutine}def sample(resource, items, item, lower, upper):\n    {}",
                            body.replace('\n', "\n    ")
                        );
                        assert_matches_cpython(&source, "linear_retry_else.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn surviving_break_reaches_cleanup_inside_nested_managers() {
    for (header, manager, work) in [
        ("def", "with", "resource.run()"),
        ("def", "with", "yield resource.value"),
        ("async def", "async with", "await resource.run()"),
        ("async def", "async with", "yield resource.value"),
    ] {
        for depth in 1..=3 {
            for cleanup in [
                "for item in items:\n    item.close()\n",
                "if items:\n    resource.close()\n",
            ] {
                let mut suite = format!(
                    "try:\n    while True:\n        if not items:\n            break\n        try:\n            {work}\n        finally:\n            resource.count -= 1\nfinally:\n    {}",
                    cleanup.replace('\n', "\n    ")
                );
                for _ in 0..depth {
                    suite = format!(
                        "{manager} resource.open():\n    {}",
                        suite.replace('\n', "\n    ")
                    );
                }
                let source = format!(
                    "{header} sample(resource, items):\n    {}",
                    suite.replace('\n', "\n    ")
                );
                assert_matches_cpython(&source, "nested_manager_break.py", &LEVELS);
            }
        }
    }
}

#[test]
fn guarded_recovery_inside_normal_finally_keeps_its_continuation() {
    for coroutine in ["", "async "] {
        for manager in [false, true] {
            for value in [
                "failure or resource.error",
                "failure and resource.error",
                "resource.error if failure is None else failure",
                "resource.error",
            ] {
                for work in ["True", "resource.run()"] {
                    let suite = format!(
                        "try:\n    done = {work}\nexcept Error as error:\n    failure = {value}\n    if ready:\n        try:\n            done = {work}\n        except Error as inner:\n            record(inner)\ntry:\n    if done:\n        resource.release()\nexcept Error as error:\n    if error is not None:\n        record(error)\n"
                    );
                    let cleanup = if manager {
                        format!(
                            "with resource.open():\n    {}",
                            suite.replace('\n', "\n    ")
                        )
                    } else {
                        suite
                    };
                    let source = format!(
                        "{coroutine}def sample(resource, ready, failure):\n    try:\n        ready = True\n    except Error as error:\n        record(error)\n    finally:\n        {}",
                        cleanup.replace('\n', "\n        ")
                    );
                    assert_matches_cpython(&source, "normal_finally_recovery.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn completed_nested_guard_reaches_recovery_cleanup() {
    for coroutine in ["", "async "] {
        for prefix in ["", "            resource.prepare()\n"] {
            for condition in [
                "any(item.key not in resource.seen for item in items)",
                "all(item.key in resource.seen for item in items)",
                "not any(item.key in resource.seen for item in items)",
            ] {
                for handler in ["Error", "Error as error"] {
                    let source = format!(
                        "{coroutine}def sample(resource, items):\n    try:\n        if items:\n{prefix}            if {condition}:\n                resource.ready = True\n        else:\n            resource.wait()\n    except LookupError:\n        record(resource)\n    except {handler}:\n        record(resource)\n    finally:\n        resource.pending.pop(items, None)\n"
                    );
                    assert_matches_cpython(&source, "nested_guard_cleanup.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn terminal_recovery_scopes_preserve_normal_exits() {
    for header in ["", "class Example:\n"] {
        for handler in ["Error", "Error as error"] {
            for suffix in ["", "record()\n"] {
                let body = format!(
                    "try:\n    pass\nexcept {handler}:\n    try:\n        raise Error()\n    except Other:\n        pass\n{suffix}"
                );
                let source = if header.is_empty() {
                    body
                } else {
                    format!("{header}    {}", body.replace('\n', "\n    "))
                };
                assert_matches_cpython(&source, "terminal_recovery_scope.py", &LEVELS);
            }
        }
    }
}

#[test]
fn terminal_recovery_body_stops_borrowed_continuation() {
    for prefix in ["", "            resource.prepare()\n"] {
        for terminal in ["return", "return resource", "raise Error(resource)"] {
            for outer in ["Error", "Error as error"] {
                for inner in ["Error", "Error as inner"] {
                    let source = format!(
                        "def sample(resource):\n    try:\n        value = resource.read()\n    except {outer}:\n        try:\n{prefix}            {terminal}\n        except {inner}:\n            if resource.failed:\n                raise\n        value = resource.read()\n    resource.write(value)\n"
                    );
                    assert_matches_cpython(&source, "terminal_recovery_body.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn unlabeled_try_entry_reaches_named_cleanup() {
    for work in ["result = 0", "result = resource.run()"] {
        for condition in ["ready", "ready and other is not None"] {
            for continuation in ["return result", "consume(result)\n    return result"] {
                let source = format!(
                    "def sample(resource, ready, other):\n    try:\n        {work}\n    except Error as error:\n        if {condition}:\n            if other is None:\n                report(resource)\n        elif resource.active:\n            try:\n                resource.save()\n            except Error as failure:\n                report(failure)\n    finally:\n        try:\n            resource.close()\n        except Error as failure:\n            result = 1\n    {continuation}\n"
                );
                assert_matches_cpython(&source, "unlabeled_try_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn recovery_break_cleanup_keeps_its_empty_exit() {
    for handler in ["Empty", "errors.Empty", "errors.Empty as error"] {
        for suffix in [
            "",
            "                        item.cancel()\n                        resource.release(item)\n",
        ] {
            for work in [
                "                resource.run()\n",
                "                for worker in workers:\n                    worker.result()\n",
            ] {
                for iterator in ["queues", "(resource.queue, *queues.values())"] {
                    let source = format!(
                        "def sample(resource, workers, queues, previous):\n    try:\n        with resource.open():\n            try:\n{work}            finally:\n                for queue in {iterator}:\n                    while True:\n                        try:\n                            item = queue.get()\n                        except {handler}:\n                            break\n{suffix}    finally:\n        for key, value in previous.items():\n            restore(key, value)\n"
                    );
                    assert_matches_cpython(&source, "recovery_break_cleanup.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn runtime_nonterminal_break_preserves_cleanup_entry() {
    for condition in [
        "ready and other",
        "not resource.stopped() and clock() < deadline",
    ] {
        for prefix in [
            "",
            "            for item in resource.items:\n                try:\n                    item.poll()\n                except LookupError:\n                    item.cancel()\n",
        ] {
            for guard in [
                "            if ready:\n                break\n",
                "            if ready and other:\n                if resource.complete:\n                    break\n                for item in resource.items:\n                    item.cancel()\n            elif resource.active:\n                resource.poll()\n",
            ] {
                for handler in [
                    "    except Error as error:\n        report(error)\n",
                    "    except (Error, Stop):\n        return 1\n",
                ] {
                    for cleanup in [
                        "        resource.finish()\n        for item in resource.items:\n            item.close()\n",
                        "        if resource.pool is not None:\n            resource.close()\n        resource.finish(missing_ok=True)\n",
                    ] {
                        let source = format!(
                            "def sample(resource, deadline, ready, other):\n    try:\n        while {condition}:\n{prefix}{guard}            resource.wait()\n{handler}    finally:\n{cleanup}"
                        );
                        assert_matches_cpython(&source, "runtime_nonterminal_cleanup.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn guarded_runtime_break_retains_its_line_before_cleanup() {
    for prefix in ["", "        resource.prepare()\n"] {
        for condition in [
            "ready and other",
            "not resource.stopped() and clock() < deadline",
        ] {
            for handler in ["Exception", "Exception as error"] {
                let source = format!(
                    "def sample(resource, deadline, ready, other):\n    try:\n{prefix}        while {condition}:\n            if ready and (other or resource.done()):\n                if resource.disabled or resource.complete:\n                    break\n    except {handler}:\n        report(resource)\n    finally:\n        resource.finish()\n"
                );
                assert_matches_cpython_versions(
                    &source,
                    "guarded_runtime_break.py",
                    &LEVELS,
                    &[
                        PythonVersion::Py312,
                        PythonVersion::Py313,
                        PythonVersion::Py314,
                    ],
                );
            }
        }
    }
}

#[test]
fn ordinary_guarded_recovery_preserves_continuation_borrowing() {
    let source = "def probe(value, receiver, worker, flag):\n    try:\n        result = worker.decode(value)\n    except Failure:\n        if flag:\n            try:\n                result = worker.decode(worker.unpack(value))\n            except Failure:\n                receiver.problem(value)\n    if worker.accepts(result):\n        receiver.store(result.get(\"first\"))\n    receiver.store(value)\n    receiver.finish()\n";
    for conditional in [false, true] {
        for outer_name in [false, true] {
            for inner_name in [false, true] {
                let mut source = source.to_owned();
                if conditional {
                    source = source.replacen(
                        "result = worker.decode(value)",
                        "result = worker.decode(value) if flag else value",
                        1,
                    );
                }
                if outer_name {
                    source = source.replacen(
                        "\n    except Failure:\n",
                        "\n    except Failure as outer_error:\n",
                        1,
                    );
                }
                if inner_name {
                    source = source.replacen(
                        "\n            except Failure:\n",
                        "\n            except Failure as inner_error:\n",
                        1,
                    );
                }
                assert_matches_cpython(&source, "ordinary_guarded_recovery.py", &LEVELS);
            }
        }
    }
    for recovery in ["pass", "raise", "return value"] {
        let source = source.replace("receiver.problem(value)", recovery);
        assert_matches_cpython(&source, "abrupt_guarded_recovery.py", &LEVELS);
    }
    let source = source.replace(
        "        if flag:\n            try:\n                result = worker.decode(worker.unpack(value))\n            except Failure:\n                receiver.problem(value)\n",
        "        receiver.problem(value)\n",
    );
    assert_matches_cpython(&source, "linear_recovery.py", &LEVELS);
}

#[test]
fn guarded_recovery_suffix_keeps_normal_continuation_live() {
    for preceding in [
        "",
        "    except AdmissionError:\n        result = recover(request)\n    except OtherError as other:\n        record(other)\n",
        "    except AdmissionError:\n        from helpers import READY, recover\n    except OtherError as other:\n        record(other)\n",
        "    except AdmissionError:\n        if started:\n            raise\n        from helpers import READY, recover\n        result = recover(request)\n    except OtherError as other:\n        record(other)\n",
    ] {
        for status in ["'pending'", "'pending' if started else 'blocked'"] {
            for suffix in [
                "",
                "        if uncertain:\n            stopped = error\n",
                "        if uncertain:\n            stopped = error\n        if not started and recovered is None:\n            if ready():\n                result = recover(request)\n",
                "        if uncertain:\n            stopped = error\n        if not started and recovered is None:\n            import helpers as imported\n",
                "        if uncertain:\n            stopped = error\n        if not started and recovered is None:\n            from helpers import READY, recover\n",
                "        if uncertain:\n            stopped = error\n        result = recovered or Result(state='pending' if uncertain else 'blocked', details={'ready': True} if uncertain else {}, error=f'Failed: {type(error).__name__}')\n",
                "        if uncertain:\n            stopped = error\n        result = Result(state='pending' if uncertain else 'blocked', details={'ready': True} if uncertain else {})\n",
            ] {
                for setup in ["", "        from helpers import work\n"] {
                    let source = format!(
                        "def sample(request, state, started, uncertain, stopped):\n    try:\n{setup}        result = work(request, state)\n{preceding}    except Error as error:\n        if isinstance(error, ProcessError):\n            try:\n                recovered = recover(state, request)\n            except Error as nested:\n                record(nested, status={status})\n{suffix}    if result.ready and stopped is not None:\n        finish(state, request, stopped)\n"
                    );
                    assert_matches_cpython(&source, "guarded_recovery_continuation.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn guarded_recovery_type_expressions_retain_borrowed_continuation() {
    for kind in [
        "Error",
        "errors.Error",
        "(Error, errors.Error)",
        "(Error, choose_error())",
    ] {
        for prefix in [
            "",
            "            if ready:\n                resource.close()\n",
            "            if ready:\n                resource.close()\n            else:\n                resource.retry()\n",
        ] {
            for continuation in [
                "        consume(resource)\n",
                "        for item in (resource.first, resource.second):\n            consume(item)\n",
            ] {
                let source = format!(
                    "def sample(resource, ready):\n    if ready:\n        try:\n{prefix}            resource.wait()\n        except {kind}:\n            if resource.ready():\n                try:\n                    resource.wait()\n                except RuntimeError:\n                    resource.retry()\n{continuation}"
                );
                assert_matches_cpython(&source, "guarded_type_expression_recovery.py", &LEVELS);
            }
        }
    }
}

#[test]
fn guarded_nested_recovery_keeps_coroutine_continuation_reachable() {
    let source = "async def sample(owner, ready):\n    try:\n        origin = 'query'\n    except Exception:\n        if ready and owner is not None:\n            try:\n                origin = 'result'\n            except Exception:\n                log('result unavailable', extra={'identifier': owner.identifier})\n    return Result(identifier=owner.identifier, available=owner.state is not None, origin=origin)\n";
    for source in [
        source.to_owned(),
        source
            .replace("        origin = 'query'", "        value = await owner.query()\n        if value.owner != owner.identifier or not owner.accepts(value):\n            return None\n        origin = 'query'")
            .replace("                origin = 'result'", "                value = await owner.result()\n                if value.owner != owner.identifier or not owner.accepts(value):\n                    return None\n                origin = 'result'"),
    ] {
        assert_matches_cpython(&source, "guarded_nested_recovery.py", &LEVELS);
    }
}

#[test]
fn final_recovery_dispatch_arm_keeps_continuation_reachable() {
    for scope in ["def", "async def"] {
        let source = format!(
            "{scope} sample(owner, mode, allowed, force, required, registry, witness):\n    try:\n        entries = owner.entries()\n    except OSError:\n        reset = True\n    except ValueError as error:\n        if not allowed:\n            if mode == 'first':\n                required = True\n            elif mode == 'second':\n                needed = True\n            elif mode == 'third':\n                witness = owner.seed()\n            elif mode == 'fourth':\n                witness = owner.seed()\n            else:\n                try:\n                    witness = owner.seed()\n                except ValueError:\n                    owner.record(witness)\n    try:\n        repaired = owner.repair()\n    except (OSError, ValueError):\n        for path in ('first', 'second'):\n            try:\n                owner.clear(path)\n            except OSError:\n                continue\n    if not (force or required or (registry is None and not allowed) or (registry is not None and registry.bad)):\n        try:\n            release = owner.release(blocked=frozenset() if registry is None else registry.blocked)\n        except Incomplete:\n            force = True\n        except ValueError:\n            for path in ('first', 'second'):\n                try:\n                    owner.clear(path)\n                except OSError:\n                    continue\n"
        );
        assert_matches_cpython(&source, "final_recovery_dispatch.py", &LEVELS);
    }
}

#[test]
fn guarded_recovery_preserves_named_cleanup_entry() {
    for prefix in [
        "        if ready:\n            resource.prepare()\n",
        "        try:\n            resource.prepare()\n        except LookupError as problem:\n            report(problem)\n",
    ] {
        for work in ["result = 0", "result = resource.run()"] {
            for condition in ["ready", "ready and other is not None"] {
                let source = format!(
                    "def sample(resource, ready, other):\n    result = None\n    try:\n{prefix}        {work}\n    except Error as error:\n        if {condition}:\n            if other is None:\n                report(resource)\n        elif resource.active:\n            try:\n                resource.save()\n            except Error as failure:\n                report(failure)\n    finally:\n        try:\n            resource.close()\n        except Error as failure:\n            result = 1\n    return result\n"
                );
                assert_matches_cpython(&source, "guarded_named_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn guarded_reraise_retains_normal_cleanup_entry() {
    for prefix in [
        "",
        "        resource.prepare()\n",
        "        if ready:\n            resource.prepare()\n        else:\n            resource.wait()\n",
        "        try:\n            resource.prepare()\n        except LookupError:\n            pass\n",
    ] {
        for recovery in ["", "                report(resource)\n"] {
            for exit in ["raise Failure(resource.code) from error", "report(error)"] {
                let source = format!(
                    "def sample(resource, callback, ready):\n    try:\n{prefix}        if callback is not None:\n            try:\n                callback(resource)\n            except:\n{recovery}                raise\n    except Exception as error:\n        {exit}\n    finally:\n        if resource.pool is not None:\n            resource.stop()\n"
                );
                assert_matches_cpython(&source, "guarded_reraise_cleanup.py", &LEVELS);
            }
        }
    }
}

#[test]
fn wrapped_terminal_recovery_stops_borrowed_continuation() {
    let indent = |body: &str| {
        body.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    };
    for (handler, terminal) in [
        ("OSError", "raise Failure(value) from None"),
        ("OSError as error", "raise Failure(value) from None"),
        ("OSError as error", "return"),
    ] {
        for wrapper in [
            "with resource.capture():",
            "with resource.capture(), acquire():",
            "try:",
        ] {
            for context in ["", "with acquire():", "for item in values:"] {
                let recovery = format!(
                    "try:\n    resource.kill()\nexcept {handler}:\n    {terminal}\nresource.record(value)\n"
                );
                let mut recovery = format!("{wrapper}\n{}", indent(&recovery));
                if wrapper == "try:" {
                    recovery.push_str("finally:\n    resource.close()\n");
                }
                let mut body = format!(
                    "try:\n    resource.wait(value)\nexcept TimeoutError:\n{}    resource.wait(value)\nconsume(value)\n",
                    indent(&recovery)
                );
                if !context.is_empty() {
                    body = format!("{context}\n{}", indent(&body));
                }
                let source = format!(
                    "def sample(resource, value, acquire, values):\n{}",
                    indent(&body)
                );
                assert_matches_cpython(&source, "wrapped_terminal_recovery.py", &LEVELS);
            }
        }
    }
}

#[test]
fn asserted_recovery_retains_enclosing_manager_successor() {
    for managers in 1..=3 {
        for context in ["acquire()", "acquire() as holder"] {
            let contexts = vec![context; managers].join(", ");
            let source = format!(
                "def sample(acquire, resource, failure):\n    with {contexts}:\n        try:\n            with resource.open():\n                if failure is not None:\n                    raise failure\n        except Error as error:\n            assert error is failure\n        else:\n            assert failure is None\n    consume(resource)\n"
            );
            assert_matches_cpython(&source, "asserted_manager_successor.py", &LEVELS);
        }
    }
}

#[test]
fn runtime_asserted_recovery_retains_borrowed_continuation() {
    for managed in [false, true] {
        for handler in ["Error", "Error as error"] {
            for assertion in [
                "str(failure) == 'expected'",
                "failure != resource",
                "ready(failure)",
            ] {
                let suite = format!(
                    "try:\n    result = acquire(resource)\nexcept {handler}:\n    assert {assertion}\nelse:\n    assert result is None\nconsume(resource)\n"
                );
                let mut source = String::from("def sample(acquire, resource, failure):\n");
                if managed {
                    source.push_str("    with acquire():\n");
                }
                let indent = if managed { "        " } else { "    " };
                for line in suite.lines() {
                    writeln!(source, "{indent}{line}").unwrap();
                }
                assert_matches_cpython(&source, "runtime_asserted_recovery.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_assertion_try_exit_excludes_enclosing_manager() {
    for (handler, recovery) in [
        ("Error", "failure is not None"),
        ("Error as error", "error is failure"),
    ] {
        let source = format!(
            "def sample(acquire, resource, failure):\n    with acquire():\n        try:\n            assert resource.state is current()\n        except {handler}:\n            assert {recovery}\n        else:\n            assert failure is None\n        saved = resource.state\n        consume(saved)\n"
        );
        assert_matches_cpython(&source, "erased_assertion_try_exit.py", &LEVELS);
    }
}

#[test]
fn terminal_nested_recovery_else_stops_borrowed_continuation() {
    let indent = |body: &str| {
        body.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    };
    for (header, operation, continuation) in [
        ("def", "worker.wait()", "consume(value)"),
        ("async def", "await worker.wait()", "await worker"),
    ] {
        for context in ["", "for index in range(3):", "with acquire():"] {
            for terminal in ["return", "raise RuntimeError(value) from None"] {
                let mut body = format!(
                    "try:\n    value = {operation}\nexcept TimeoutError:\n    try:\n        {operation}\n    except LookupError:\n        pass\n    except RuntimeError as caught:\n        if not allowed(caught):\n            raise\n    else:\n        {terminal}\n    value = recover(value)\n{continuation}\n"
                );
                if !context.is_empty() {
                    body = format!("{context}\n{}", indent(&body));
                }
                let source = format!("{header} sample(worker, value):\n{}", indent(&body));
                assert_matches_cpython(&source, "terminal_recovery_else.py", &LEVELS);
            }
        }
    }
}

#[test]
fn asserted_try_else_retains_borrowed_continuation() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            writeln!(result, "    {line}").unwrap();
        }
        result
    };
    for managed in [false, true] {
        for assignment in [false, true] {
            for named in [false, true] {
                for raises in [false, true] {
                    let mut work = String::new();
                    if assignment {
                        work.push_str("holder.value = \"left\" if flag else \"right\"\n");
                    }
                    work.push_str(if raises {
                        "if failure is not None:\n    raise failure\n"
                    } else {
                        "consume(holder)\n"
                    });
                    if managed {
                        work = format!("with acquire() as holder:\n{}", indent(&work));
                    }
                    let mut body = format!("try:\n{}", indent(&work));
                    body.push_str(if named {
                        "except Error as caught:\n    assert caught is failure\n"
                    } else {
                        "except Error:\n    assert failure is not None\n"
                    });
                    body.push_str("else:\n    assert failure is None\nkind = first if flag else second\nlabels = [\"sample\", f\"kind:{kind}\"]\nrecord(labels)\nselected = [entry for entry in events if entry.key == \"ready\"]\nassert selected == ([build(0, labels)] if flag else [])\n");
                    let source = format!(
                        "def sample(acquire, holder, flag, failure, first, second, events):\n{}",
                        indent(&body)
                    );
                    assert_matches_cpython(&source, "asserted_recovery.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn managed_asserted_try_else_retains_borrowed_continuation() {
    for work in [
        "consume(resource)\n",
        "with resource.open():\n                consume(resource)\n",
        "with resource.open():\n                if failure is not None:\n                    raise failure\n",
        "consume(resource)\n            assert resource.state is current()\n",
        "with resource.open():\n                assert resource.state is current()\n",
    ] {
        for (handler, recovery) in [
            ("Error", "failure is not None"),
            ("Error as error", "error is failure"),
        ] {
            let source = format!(
                "def sample(acquire, resource, failure):\n    with acquire():\n        try:\n            {work}        except {handler}:\n            assert {recovery}\n        else:\n            assert failure is None\n        saved = resource.state\n        consume(saved)\n"
            );
            assert_matches_cpython(&source, "managed_asserted_recovery.py", &LEVELS);
        }
    }
}

#[test]
fn runtime_while_false_edge_reaches_guarded_finally() {
    for (prefix, read) in [("def", "read()"), ("async def", "await read()")] {
        for condition in ["not stop.is_set()", "ready or stop.is_set()"] {
            for handler in ["pass", "recover(error)"] {
                for tail in ["", "            tick(value)\n"] {
                    let source = format!(
                        "{prefix} probe(stop, value, resource, ready):\n    try:\n        while {condition}:\n            try:\n                value = {read}\n            except LookupError:\n                continue\n            if value is None:\n                break\n{tail}    except Exception as error:\n        {handler}\n    finally:\n        if resource is not None:\n            resource.close()\n        stop.wait()\n"
                    );
                    assert_matches_cpython(&source, "runtime_while_guarded_finally.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn runtime_finalizer_while_preserves_entry_and_exit_boundaries() {
    for (prefix, call) in [("def", "wait()"), ("async def", "await wait()")] {
        for protected in ["pass", call] {
            for condition in ["not done()", "ready or done()", "ready and done()"] {
                for tail in ["return saved", "finish(saved)"] {
                    let source = format!(
                        "{prefix} probe(wait, done, ready, saved):\n    try:\n        {protected}\n    except BaseException as error:\n        saved = error\n    finally:\n        while {condition}:\n            try:\n                {call}\n            except BaseException as error:\n                saved = saved or error\n    {tail}\n"
                    );
                    assert_matches_cpython(&source, "runtime_finalizer_while.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn coroutine_asserted_recovery_preserves_borrowed_continuation() {
    for call in [
        "worker.run()",
        "worker.run(value=first if ready else second)",
    ] {
        for handler in [
            "        assert error is failure\n",
            "        assert not ready\n        assert error.code == 1\n        assert error.reason == 'invalid'\n",
        ] {
            for assertion in ["ready", "failure is None"] {
                let source = format!(
                    "async def probe(worker, reader, recorder, ready, failure, first, second):\n    try:\n        await {call}\n    except ValueError as error:\n{handler}    else:\n        assert {assertion}\n    reader.consume(value=first)\n    recorder.finish()\n"
                );
                assert_matches_cpython(&source, "coroutine_asserted_recovery.py", &LEVELS);
            }
        }
    }
}

#[test]
fn finalizer_loop_empty_try_keeps_exception_owner() {
    for manager in ["", "with acquire():", "with acquire(), acquire():"] {
        for prefix in ["def", "async def"] {
            for body in ["pass", "assert ready", "tick()"] {
                for handler in ["BaseException", "BaseException as error"] {
                    let suite = format!(
                        "    try:\n        tick()\n    except {handler}:\n        pass\n    finally:\n        for item, key in groups.items():\n            try:\n                {body}\n            except {handler}:\n                pass\n"
                    );
                    let suite = if manager.is_empty() {
                        suite
                    } else {
                        let indented = suite.lines().fold(String::new(), |mut output, line| {
                            writeln!(output, "    {line}").unwrap();
                            output
                        });
                        format!("    {manager}\n{indented}")
                    };
                    let source = format!("{prefix} probe(groups, ready):\n{suite}");
                    assert_matches_cpython(&source, "finalizer_loop_empty_try.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn compound_finalizer_conditions_preserve_retired_handler_slots() {
    let source = r"def sample(value, state):
    try:
        token = work(value)
        if type(token) is not int or token != 0 or not state.first:
            raise RuntimeError()
        token = work(state)
        if type(token) is not int or token != 0 or not state.second:
            raise RuntimeError()
    except (AttributeError, ValueError) as error:
        raise RuntimeError() from error
    finally:
        try:
            if state.second and not finish(value):
                raise RuntimeError()
        finally:
            if state.first and not finish(state):
                raise RuntimeError()
";
    assert_matches_cpython(source, "compound_finalizer_condition.py", &LEVELS);
}

#[test]
fn literal_handler_returns_retire_inner_try_before_managed_iterator() {
    let source = r"def sample(items, flag):
    with manager():
        for item in items:
            try:
                try:
                    work(item)
                except ValueError as error:
                    if flag:
                        return 7
                    continue
            except TypeError:
                raise
    raise RuntimeError
";
    for context in ["ordinary", "coroutine", "async_manager"] {
        let mut source = source.to_owned();
        if context != "ordinary" {
            source = source.replace("def sample", "async def sample");
        }
        if context == "async_manager" {
            source = source.replace("with manager():", "async with manager():");
        }
        for returned in ["return 7", "return None", "return"] {
            assert_matches_cpython(
                &source.replace("return 7", returned),
                "managed_handler_return.py",
                &LEVELS,
            );
        }
    }
}

#[test]
fn coroutine_iterator_handler_returns_preserve_outer_try_owner() {
    let source = r"async def sample(items, work, finish):
    try:
        for item in items:
            try:
                try:
                    await work()
                except ValueError:
                    return item
            finally:
                await finish()
    except Exception:
        raise
";
    for named in [false, true] {
        for iterator in [false, true] {
            for returned in ["item", "None"] {
                for awaited in [false, true] {
                    let mut source = source.replace("return item", &format!("return {returned}"));
                    if named {
                        source =
                            source.replace("except ValueError:", "except ValueError as error:");
                    }
                    if !iterator {
                        source = source.replace("for item in items:", "while items:");
                    }
                    if !awaited {
                        source = source.replace("await finish()", "finish()");
                    }
                    assert_matches_cpython(&source, "iterator_handler_return_owner.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn awaited_finalizers_preserve_normal_iterator_return_locations() {
    let source = r"async def sample(items, work, finish):
    try:
        for item in items:
            try:
                try:
                    await work()
                except ValueError:
                    if item:
                        raise
                    continue
                except TypeError:
                    return item
                return item
            finally:
                await finish()
    except Exception:
        raise
    return None
";
    for named in [false, true] {
        for returned in ["item", "work(item)", "None", "7", "True", "(1, 2)"] {
            for (index, cleanup) in ["finish()", "await finish()", "await finish.close(item)"]
                .into_iter()
                .enumerate()
            {
                let mut source = source
                    .replace("return item", &format!("return {returned}"))
                    .replace("await finish()", cleanup);
                if named {
                    source = source.replace("except TypeError:", "except TypeError as error:");
                }
                assert_matches_cpython(
                    &source,
                    &format!("awaited_iterator_return_location_{named}_{returned}_{index}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn literal_finalizer_returns_preserve_completed_cleanup_locations() {
    let source = r"def sample(items, work, finish):
    try:
        for item in items:
            try:
                work(item)
                return None
            finally:
                finish()
    except ValueError:
        raise
    return None
";
    for generator in [false, true] {
        for returned in ["None", "7", "True", "(1, 2)"] {
            let mut source = source.replace("return None", &format!("return {returned}"));
            if generator {
                source = source.replacen("    try:\n", "    yield None\n    try:\n", 1);
            }
            assert_matches_cpython(
                &source,
                &format!("literal_finalizer_return_{generator}_{returned}.py"),
                &LEVELS,
            );
            if !generator {
                assert_matches_cpython(
                    &source.replace("def sample", "async def sample"),
                    &format!("coroutine_literal_finalizer_return_{returned}.py"),
                    &LEVELS,
                );
            }
        }
    }
    let asynchronous = source
        .replace("def sample", "async def sample")
        .replacen("    try:\n", "    yield None\n    try:\n", 1)
        .replace("return None", "return")
        .replace("finish()", "await finish()");
    assert_matches_cpython(
        &asynchronous,
        "async_generator_literal_finalizer_return.py",
        &LEVELS,
    );
}

#[test]
fn inner_handler_returns_restore_exception_state_before_outer_managers() {
    let source = r"def sample(manager, work, finish, value):
    with manager():
        try:
            work()
        finally:
            try:
                finish()
            except ValueError:
                return value
";
    for asynchronous in [false, true] {
        for named in [false, true] {
            for returned in ["value", "None"] {
                let mut source = source.replace("return value", &format!("return {returned}"));
                if asynchronous {
                    source = source.replace("def sample", "async def sample");
                }
                if named {
                    source = source.replace("except ValueError:", "except ValueError as error:");
                }
                for outer_try in [false, true] {
                    let source = if outer_try {
                        let (header, body) = source.split_once('\n').unwrap();
                        format!(
                            "{header}\n    try:\n{}    except RuntimeError:\n        raise\n",
                            body.lines().fold(String::new(), |mut output, line| {
                                writeln!(output, "    {line}").unwrap();
                                output
                            }),
                        )
                    } else {
                        source.clone()
                    };
                    assert_matches_cpython(&source, "handler_finally_manager_order.py", &LEVELS);
                    if asynchronous {
                        assert_matches_cpython(
                            &source.replace("with manager():", "async with manager():"),
                            "handler_finally_async_manager_order.py",
                            &LEVELS,
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn inner_handler_returns_restore_exception_state_before_outer_iterators() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    }

    let body = "try:\n    work()\nfinally:\n    try:\n        finish()\n    except ValueError:\n        return value\n";
    for loops in [
        &[][..],
        &["for item in items:"][..],
        &["while items:"][..],
        &["for item in items:", "for other in items:"][..],
    ] {
        for named in [false, true] {
            for returned in ["value", "None"] {
                let mut body = body.replace("return value", &format!("return {returned}"));
                if named {
                    body = body.replace("except ValueError:", "except ValueError as error:");
                }
                for header in loops.iter().rev() {
                    body = format!("{header}\n{}", indent(&body));
                }
                for (header, prefix) in
                    [("def", ""), ("async def", ""), ("def", "    yield None\n")]
                {
                    let source = format!(
                        "{header} sample(items, value, work, finish):\n{prefix}{}",
                        indent(&body),
                    );
                    assert_matches_cpython(&source, "handler_finally_iterator_order.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn retained_managers_own_value_return_iterator_cleanup() {
    let source = r"async def sample(items, manager, action, finish):
    try:
        async with manager():
            for item in items:
                try:
                    value = await action(item)
                    return value
                except Error as error:
                    recover(error)
    finally:
        await finish()
";
    for source in [
        source.to_owned(),
        source.replace("async with", "with"),
        source.replace("return value", "return"),
        r"async def sample(items, manager, action, finish):
    try:
        async with manager():
            for item in items:
                value = await action(item)
                return value
    finally:
        await finish()
"
        .to_owned(),
        r"async def sample(items, action, finish):
    try:
        for item in items:
            try:
                value = await action(item)
                return value
            except Error as error:
                recover(error)
    finally:
        await finish()
"
        .to_owned(),
    ] {
        assert_matches_cpython(&source, "retained_manager_iterator_return.py", &LEVELS);
    }
}

#[test]
fn coroutine_for_else_break_keeps_nonfinal_branch_order() {
    let source = r"async def sample(values, flag):
    OWNER manager():
        ITERATOR item in values:
            if flag:
                if CONDITION:
                    break
            elif accepted(item):
                break
        else:
            raise RuntimeError('exhausted')
    await complete()
";
    for owner in ["with", "async with"] {
        for iterator in ["for", "async for"] {
            for condition in ["item.state in states", "flag", "accepted(item)"] {
                let source = source
                    .replace("OWNER", owner)
                    .replace("ITERATOR", iterator)
                    .replace("CONDITION", condition);
                assert_matches_cpython(&source, "coroutine_for_else_break.py", &LEVELS);
            }
        }
    }
}

#[test]
fn coroutine_nested_finalizers_keep_iterator_return_locations() {
    let source = r"async def sample(manager, action, cleanup, values, value, flag):
    try:
        with manager:
            for outer in values:
                for middle in values:
                    try:
                        for item in values:
                            if flag:
                                value = None
                                return item.attr
                    except ValueError:
                        value = None
                        raise
                    finally:
                        value = [*value.attr, None] if value is not None else None
                        cleanup(value)
    except TypeError as error:
        value = None
        raise
    finally:
        cleanup()
";
    for source in [
        source.to_owned(),
        source.replace(
            "                        value = [*value.attr, None] if value is not None else None\n",
            "",
        ),
        source.replace(
            "                        value = [*value.attr, None] if value is not None else None",
            "                        if flag:\n                            raise ValueError",
        ),
    ] {
        assert_matches_cpython(&source, "coroutine_nested_finalizer_owner.py", &LEVELS);
    }
}

#[test]
fn return_unwinds_interleaved_managers_and_iterators() {
    let source = r"async def sample(items, manager, stream, flag, finish):
    try:
        for item in items:
            async with manager():
                async for value in stream():
                    if flag:
                        return
    finally:
        await finish()
";
    for (name, source) in [
        ("async", source.to_owned()),
        ("sync_manager", source.replace("async with", "with")),
        ("sync_iterator", source.replace("async for", "for")),
    ] {
        for (result, statement) in [
            ("bare", "return"),
            ("none", "return None"),
            ("value", "return value"),
        ] {
            assert_matches_cpython(
                &source.replace("return\n", &format!("{statement}\n")),
                &format!("interleaved_return_{name}_{result}.py"),
                &LEVELS,
            );
        }
    }
    for (name, source) in [
        (
            "no_finally",
            r"async def sample(items, manager, stream, flag):
    for item in items:
        async with manager():
            async for value in stream():
                if flag:
                    return
",
        ),
        (
            "no_outer_iterator",
            r"async def sample(manager, stream, flag, finish):
    try:
        async with manager():
            async for value in stream():
                if flag:
                    return
    finally:
        await finish()
",
        ),
    ] {
        assert_matches_cpython(source, &format!("interleaved_return_{name}.py"), &LEVELS);
    }
}

#[test]
fn handler_return_keeps_outer_finalizer_owner_through_iterator_cleanup() {
    let source = r"async def sample(items, flag):
    try:
        for item in items:
            try:
                await work(item)
            except ValueError as error:
                if flag:
                    return False
            finally:
                finish(item)
    finally:
        await finish(flag)
";
    for (index, source) in [
        source.to_owned(),
        source.replace("return False", "return flag"),
        source.replace("return False", "return"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("nested_finalizer_iterator_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn return_retires_inner_try_before_iterator_after_finalizer_and_manager() {
    let source = r"async def sample(items, manager, ready):
    for item in items:
        try:
            async with manager:
                state = None
                try:
                    result = await action()
                    state = True
                    return result
                except Exception as error:
                    state = False
                    record()
                    raise
                finally:
                    finish(item, state)
        except Exception as error:
            if ready:
                raise
            await action()
    raise RuntimeError()
";
    for source in [
        source.to_owned(),
        source.replace("async with", "with"),
        source
            .replace("async def", "def")
            .replace("async with", "with")
            .replace("await ", ""),
    ] {
        for returned in ["return result", "return", "return None"] {
            assert_matches_cpython(
                &source.replace("return result", returned),
                "finalizer_manager_iterator_return.py",
                &LEVELS,
            );
        }
    }
}

#[test]
fn conditional_raise_exits_keep_compound_finalizers_ownerless() {
    let source = r"def sample(value):
    try:
        if value:
            raise RuntimeError()
    except Exception:
        raise
    finally:
        try:
            action()
        except Exception:
            if value:
                raise
        finally:
            action()
";
    for (index, source) in [
        source.to_owned(),
        source.replacen("if value:", "if value.ready():", 1),
        source.replace("        except Exception:\n            if value:\n                raise\n", ""),
        source.replace("        finally:\n            action()\n", ""),
        source.replace("    except Exception:\n        raise\n", ""),
        source.replace("        if value:\n            raise RuntimeError()", "        raise RuntimeError()"),
        source.replace("        try:\n            action()\n        except Exception:\n            if value:\n                raise\n        finally:\n            action()", "        action()"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("conditional_raise_finalizer_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn coroutine_awaited_branches_preserve_recovery_finalizer_borrowing() {
    let source = r"async def sample(value, state, ready):
    failed = False
    try:
        await prepare()
        if value is None:
            prior = await action()
            if ready:
                check(prior)
            else:
                check(value)
            await verify(prior)
        elif value != state.key:
            async with manager():
                prior = await load(await lookup(value))
            await verify(prior)
    except Exception:
        failed = True
        await recover()
        raise
    finally:
        try:
            await finish(value=value, state=state, failed=failed)
        except Exception:
            if not failed:
                raise
            recover_cleanup()
";
    assert_matches_cpython(source, "coroutine_await_recovery.py", &LEVELS);
}

#[test]
fn coroutine_conditional_stores_preserve_recovery_finalizer_borrowing() {
    let source = r#"async def sample(value, state, ready):
    tag = "unset"
    try:
        if value is None:
            tag = "first" if ready else "second"
        elif value != state.key:
            action()
    except Exception:
        pass
    finally:
        try:
            kind = state.kind or "unset"
            finish(tag, kind)
        except Exception:
            pass
"#;
    for later_arm in [
        "action()",
        "async with manager():\n                prior = await load(value)\n            if prior is None:\n                return\n            tag = prior.tag",
    ] {
        let source = source.replace("action()", later_arm);
        let mut nonlocal = String::from("def outer():\n    tag = None\n");
        for line in source
            .replace("    tag = \"unset\"", "    nonlocal tag")
            .lines()
        {
            writeln!(nonlocal, "    {line}").unwrap();
        }
        for source in [
            source.clone(),
            source.replace("    tag = \"unset\"", "    global tag\n    tag = \"unset\""),
            source.replacen(
                "    try:\n",
                "    def read_tag():\n        return tag\n    try:\n",
                1,
            ),
            nonlocal,
        ] {
            assert_matches_cpython(&source, "coroutine_conditional_store_recovery.py", &LEVELS);
        }
    }
}

#[test]
fn elif_branches_preserve_recovery_finalizer_borrowing() {
    let source = r"def sample(value, state):
    try:
        if value is None:
            action()
        elif value != state:
            action()
    except Exception:
        pass
    finally:
        try:
            action(value, state)
        except Exception:
            pass
";
    for handler in ["pass", "recover()"] {
        assert_matches_cpython(
            &source.replace("        pass", &format!("        {handler}")),
            &format!("elif_recovery_finalizer_{handler}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn async_loop_else_preserves_finalizer_borrow_boundary() {
    let source = r#"async def sample(value):
    if mode is not None:
        request = prepare(value)
        await channel.send(request.key, timeout=1)
        response = await channel.receive(value, timeout=1)
        assert response == value
        return
    resource = await acquire(options())
    try:
        send(value.encode())
        data = b""
        async with limit(10):
            async for part in resource:
                data += part.payload
                assert len(data) < 1024
                if value.encode() in data:
                    break
            else:
                fail("missing")
    finally:
        resource.close()
"#;
    let early = "    if mode is not None:\n        request = prepare(value)\n        await channel.send(request.key, timeout=1)\n        response = await channel.receive(value, timeout=1)\n        assert response == value\n        return\n";
    let unprotected = source
        .replace("    try:\n", "")
        .replace("    finally:\n        resource.close()\n", "");
    let (prefix, body) = unprotected
        .split_once("        send(value.encode())")
        .unwrap();
    let unprotected = format!(
        "{prefix}    send(value.encode()){}",
        body.replace("\n    ", "\n")
    );
    let (prefix, body) = source
        .split_once("        async with limit(10):\n")
        .unwrap();
    let (body, suffix) = body.split_once("    finally:").unwrap();
    let unmanaged = format!(
        "{prefix}{}    finally:{suffix}",
        body.lines()
            .map(|line| &line[4..])
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    );
    for (name, source) in [
        ("plain", source.to_owned()),
        (
            "no_assert",
            source.replace("                assert len(data) < 1024\n", ""),
        ),
        ("no_early_branch", source.replace(early, "")),
        ("no_finally", unprotected),
        (
            "no_loop_else",
            source.replace("            else:\n                fail(\"missing\")\n", ""),
        ),
        ("no_manager", unmanaged),
        (
            "no_prefix_call",
            source.replace("        send(value.encode())\n", ""),
        ),
        (
            "simple_test",
            source.replace("value.encode() in data", "ready(part)"),
        ),
    ] {
        assert_matches_cpython(&source, &format!("async_loop_else_{name}.py"), &LEVELS);
    }
}

#[test]
fn copied_finalizers_preserve_live_exception_owners() {
    let nested = r"async def sample(manager, action, cleanup, value):
    try:
        try:
            async with manager:
                await action()
        except ValueError:
            value = None
            return
        finally:
            value = cleanup()
    except TypeError:
        value = None
        return
    finally:
        if value is not None:
            cleanup(value)
";
    let managed = r"def sample(manager, action, cleanup, value, flag):
    with manager:
        try:
            with manager:
                try:
                    action()
                except ValueError as error:
                    try:
                        value = cleanup(error)
                        if flag:
                            action()
                        return value
                    finally:
                        with manager:
                            cleanup()
                except TypeError as error:
                    return value
        finally:
            value = None
";
    let iterator = r"def sample(manager, action, cleanup, values, value, flag):
    try:
        with manager:
            for outer in values:
                for middle in values:
                    try:
                        for item in values:
                            if flag:
                                value = None
                                return item.attr
                    except ValueError:
                        value = None
                        raise
                    finally:
                        value = [*value.attr, None] if value is not None else None
                        cleanup(value)
    except TypeError as error:
        value = None
        raise
    finally:
        cleanup()
";
    let handler = r"def sample(manager, action, cleanup, value, flag):
    with manager:
        while flag:
            try:
                action()
            except ValueError as error:
                if flag:
                    try:
                        if not action(value):
                            return
                        value = cleanup()
                    except KeyError:
                        raise
                    except TypeError as inner:
                        value = inner
                    finally:
                        value.attr = None
            finally:
                cleanup()
";
    let synchronous = nested
        .replace("async def", "def")
        .replace("async with", "with")
        .replace("await action()", "action()");
    for source in [
        nested.to_owned(),
        nested.replace("async with manager:", "with manager:"),
        nested.replace(
            "            async with manager:\n                await action()",
            "            await action()",
        ),
        synchronous.clone(),
    ] {
        assert_matches_cpython_versions(
            &source,
            "nested_finalizer_owner.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py315,
            ],
        );
    }
    for source in [
        managed.to_owned(),
        managed.replace(
            "                        with manager:\n                            cleanup()",
            "                        cleanup()",
        ),
        managed
            .replace(" as error", "")
            .replace("cleanup(error)", "cleanup(value)"),
    ] {
        assert_matches_cpython_versions(
            &source,
            "managed_handler_finalizer_owner.py",
            &LEVELS,
            &[PythonVersion::Py312, PythonVersion::Py313],
        );
    }
    for source in [
        iterator.to_owned(),
        iterator.replace(
            "                        value = [*value.attr, None] if value is not None else None\n",
            "",
        ),
        handler.to_owned(),
        handler.replace("value.attr = None", "cleanup(value)"),
        iterator.replace(
            "                        value = [*value.attr, None] if value is not None else None",
            "                        if flag:\n                            raise ValueError",
        ),
        synchronous
            .replace("            return\n", "            return value.attr\n")
            .replace("        return\n", "        return value.attr\n"),
        synchronous
            .replace("            return\n", "            return value\n")
            .replace("        return\n", "        return value\n"),
    ] {
        assert_matches_cpython(&source, "copied_finalizer_owner.py", &LEVELS);
    }
    for source in [
        synchronous.replacen("    try:\n", "    yield value\n    try:\n", 1),
        synchronous.replacen("    try:\n", "    yield from values\n    try:\n", 1),
        nested.replacen("    try:\n", "    yield value\n    try:\n", 1),
    ] {
        assert_matches_cpython_versions(
            &source,
            "generator_finalizer_owner.py",
            &LEVELS,
            &[
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py315,
            ],
        );
    }
}

#[test]
fn raising_finalizer_preserves_handler_return_iterator_owner() {
    let source = r"async def sample(items, work, cleanup, recover, finish):
    try:
        for item in items:
            try:
                await work(item)
            except ValueError as error:
                return
            finally:
                try:
                    await cleanup(item)
                except TypeError:
                    raise
    except LookupError:
        recover()
    finally:
        finish()
";
    for (name, source) in [
        ("reraise", source.to_owned()),
        (
            "raise_value",
            source.replace(
                "                    raise",
                "                    raise RuntimeError()",
            ),
        ),
        (
            "no_raise",
            source.replace("                    raise", "                    recover()"),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("raising_finalizer_handler_return_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn copied_iterator_break_joins_guarded_handler_exits() {
    let source = r"def nested_cleanup(groups, skip, retry, stop, acquire, transform, close, done):
    for batch in groups:
        finished = False
        for item in batch:
            if item is None or skip(item):
                continue
            ticket = None
            try:
                ticket = acquire(item)
                if ticket is None or retry(ticket):
                    continue
                result = transform(ticket)
                if result is None or retry(result):
                    continue
                finished = stop(result)
                if finished:
                    break
            except ValueError:
                continue
            finally:
                if ticket is not None:
                    try:
                        close(ticket)
                    except RuntimeError:
                        pass
        if finished:
            done(batch)
            return
";
    let single_loop = source
        .lines()
        .enumerate()
        .fold(String::new(), |mut text, (index, line)| {
            if index == 1 {
                writeln!(text, "    batch = groups").unwrap();
            } else {
                let line = if index > 1 {
                    line.strip_prefix("    ").unwrap_or(line)
                } else {
                    line
                };
                writeln!(text, "{line}").unwrap();
            }
            text
        });
    for (index, source) in [
        source.to_owned(),
        source.replace("                    break", "                    continue"),
        single_loop,
        source.replace(
            "                if ticket is not None:\n                    try:\n                        close(ticket)\n                    except RuntimeError:\n                        pass",
            "                try:\n                    close(ticket)\n                except RuntimeError:\n                    pass",
        ),
        include_str!(
            "fixtures/compatibility/copied_finally_break_threading/matches_cpython_marshal_for_guarded_try_finally_iterator_breaks/guarded_body_and_handler_break_none_guard.py"
        ).to_owned(),
    ].into_iter().enumerate() {
        assert_matches_cpython(&source, &format!("guarded_handler_iterator_{index}.py"), &LEVELS);
    }
}

#[test]
fn recovery_finalizer_preserves_local_return_borrow_boundary() {
    let source = r"def sample(items, work, recover, cleanup, valid):
    for item in items:
        value = work(item)
        try:
            try:
                work(value)
            except Exception:
                if not valid(value):
                    raise
                result = recover(value)
                if valid(result):
                    return work(result)
                continue
            return value
        finally:
            try:
                cleanup()
            except Exception:
                pass
    raise ValueError()
";
    for (name, source) in [
        ("plain", source.to_owned()),
        (
            "no_guard",
            source.replace("                if not valid(value):\n                    raise\n", ""),
        ),
        (
            "no_handler_return",
            source.replace("                if valid(result):\n                    return work(result)\n", ""),
        ),
        ("named_handler", source.replace("except Exception:", "except Exception as error:")),
        (
            "plain_finally",
            source.replace("            try:\n                cleanup()\n            except Exception:\n                pass", "            cleanup()"),
        ),
        ("return_call", source.replace("            return value", "            return work(value)")),
        ("coroutine", source.replace("def sample", "async def sample")),
    ] {
        assert_matches_cpython(&source, &format!("recovery_finalizer_return_{name}.py"), &LEVELS);
    }
}

#[test]
fn chained_comparison_try_exit_preserves_borrow_fallthrough() {
    let source = r"def sample(items, action, guard, recover):
    for item in items:
        try:
            left, right = action()
            if left < item < right:
                return left, right
        except ValueError:
            pass
        if item:
            recover()
    return None
";
    for (name, condition) in [
        ("chain", "left < item < right"),
        ("and_names", "left and right"),
        ("or_names", "left or right"),
        ("and_calls", "guard(left) and guard(right)"),
        ("and_compares", "left == item and right is not None"),
        ("chain_and_head", "guard(left) and left < item < right"),
        (
            "chain_and_middle",
            "guard(left) and left < item < right and guard(right)",
        ),
        ("chain_and_tail", "left < item < right and guard(right)"),
        ("chain_or_head", "guard(left) or left < item < right"),
        ("chain_or_tail", "left < item < right or guard(right)"),
        ("chain_negated", "not (left < item < right)"),
    ] {
        assert_matches_cpython(
            &source.replace("left < item < right", condition),
            &format!("chained_comparison_recovery_{name}.py"),
            &LEVELS,
        );
    }
    let conjunction = source.replace("left < item < right", "left and right");
    for (name, source) in [
        (
            "and_prefix",
            conjunction.replace(
                "            left, right = action()\n",
                "            recover()\n            left, right = action()\n",
            ),
        ),
        (
            "and_scalar",
            conjunction.replace("return left, right", "return left"),
        ),
        (
            "and_no_tail_guard",
            conjunction.replace(
                "        if item:\n            recover()\n",
                "        recover()\n",
            ),
        ),
        (
            "chain_scalar",
            source.replace("return left, right", "return left"),
        ),
        (
            "chain_and_no_tail_guard",
            source
                .replace(
                    "left < item < right",
                    "left < item < right and guard(right)",
                )
                .replace(
                    "        if item:\n            recover()\n",
                    "        recover()\n",
                ),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("chained_comparison_recovery_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_suffix_preserves_async_iterator_branch_exits() {
    let source = r"async def sample(source, known, saved, convert, manager, check):
    async with manager():
        async for item in source:
            value = convert(item)
            if value in known:
                saved.append(value)
            assert not await check(item)
        assert saved
";
    let unmanaged = r"async def sample(source, known, saved, convert, manager, check):
    async for item in source:
        value = convert(item)
        if value in known:
            saved.append(value)
        assert not await check(item)
    assert saved
";
    for (name, source) in [
        ("managed", source.to_owned()),
        ("unmanaged", unmanaged.to_owned()),
        ("name_test", source.replace("value in known", "known")),
        (
            "plain_assert",
            source.replace("not await check", "not check"),
        ),
        (
            "no_suffix",
            source.replace("            assert not await check(item)\n", ""),
        ),
        (
            "sync_iterator",
            source.replace("async for item", "for item"),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("async_iterator_erased_suffix_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_nested_iterator_work_preserves_shared_exit() {
    let source = r"async def sample(items, flag, manager, load, check):
    with manager:
        for item in items:
            with manager:
                value = await load(item)
            assert check(value)
            if flag in value:
                graph = value[flag]
                for node in graph:
                    if node.parent is not None:
                        assert check(node.parent)
                    for child in node.children:
                        assert check(child)
            else:
                assert check(value)
";
    for (name, source) in [
        ("base", source.to_owned()),
        (
            "no_prefix",
            source.replace("            with manager:\n                value = await load(item)\n            assert check(value)\n", "            value = item\n"),
        ),
        (
            "single_inner_loop",
            source.replace("                    for child in node.children:\n                        assert check(child)\n", ""),
        ),
        ("sync", source.replace("async def", "def").replace("await load(item)", "load(item)")),
        ("tuple_target", source.replace("for node in graph:", "for key, node in graph.items():")),
        (
            "direct_call",
            source.replace("                graph = value[flag]\n                for node in graph:\n                    if node.parent is not None:\n                        assert check(node.parent)\n                    for child in node.children:\n                        assert check(child)\n", "                check(value)\n"),
        ),
        ("no_else", source.replace("            else:\n                assert check(value)\n", "")),
        ("live_else", source.replace("            else:\n                assert check(value)\n", "            else:\n                check(value)\n")),
    ] {
        assert_matches_cpython(&source, &format!("nested_erased_iterator_{name}.py"), &LEVELS);
    }
}

#[test]
fn copied_conditional_backedge_preserves_outer_handler() {
    let source = "def sample(stop, action, other, recover, outer_recover, flag):\n    try:\n        while not stop():\n            if flag:\n                return\n            try:\n                action() if flag else other()\n            except ValueError:\n                recover()\n    except Exception:\n        outer_recover()\n";
    for source in [
        source.to_owned(),
        source.replace(
            "        while not stop():",
            "        try:\n            action()\n        except KeyError:\n            recover()\n        while not stop():",
        ),
        "def sample(stop, action, other, recover, outer_recover, flag):\n    while not stop():\n        if flag:\n            return\n        try:\n            action() if flag else other()\n        except ValueError:\n            recover()\n".to_owned(),
        source.replace("            if flag:\n                return\n", ""),
        source.replace(
            "action() if flag else other()",
            "value = action() if flag else other()",
        ),
        source.replace(
            "                action() if flag else other()",
            "                if flag:\n                    action()\n                else:\n                    other()",
        ),
        source.replace("action() if flag else other()", "action()"),
        source
            .replace("outer_recover, flag):", "outer_recover, flag, manager, items):")
            .replace(
                "            if flag:\n                return",
                "            if flag:\n                with manager:\n                    values = {key: list(value) for key, value in items}\n                if values:\n                    try:\n                        action()\n                    except LookupError:\n                        recover()\n                return",
            ),
    ] {
        assert_matches_cpython(&source, "conditional_recovery.py", &LEVELS);
    }
}

#[test]
fn erased_branches_preserve_nested_finalizer_loop_exits() {
    let source = r"def sample(items, flag, check, manager, finish):
    try:
        with manager():
            try:
                for item in items:
                    if flag:
                        assert check(item)
                    else:
                        assert not check(item)
            finally:
                finish()
    finally:
        finish()
";
    let single = r"def sample(items, flag, check, manager, finish):
    with manager():
        try:
            for item in items:
                if flag:
                    assert check(item)
                else:
                    assert not check(item)
        finally:
            finish()
";
    let plain = r"def sample(items, flag, check, manager, finish):
    for item in items:
        if flag:
            assert check(item)
        else:
            assert not check(item)
";
    for (name, source) in [
        ("nested", source.to_owned()),
        ("single", single.to_owned()),
        ("plain", plain.to_owned()),
        (
            "live_else",
            source.replace("assert not check(item)", "check(item)"),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("nested_finalizer_erased_branch_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_prefix_preserves_conditional_continue_exit() {
    let source = r#"async def sample(items, selected, verify):
    for item in items:
        if selected(item):
            if item.kind != "keep":
                assert verify(item)
                continue
            assert verify(item)
"#;
    for (name, source) in [
        ("plain", source.to_owned()),
        ("synchronous", source.replace("async def", "def")),
        (
            "manager",
            source.replace("\n    ", "\n        ").replace(
                "verify):\n",
                "verify, manager):\n    async with manager():\n",
            ),
        ),
        (
            "multiple_assertions",
            source.replace(
                "                assert verify(item)\n",
                "                assert verify(item)\n                assert selected(item)\n",
            ),
        ),
        (
            "name_condition",
            source.replace("item.kind != \"keep\"", "item"),
        ),
        (
            "no_prefix",
            source.replace("\n                assert verify(item)", ""),
        ),
        (
            "no_suffix",
            source.replace("\n            assert verify(item)", ""),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("erased_continue_prefix_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_generator_guard_preserves_coroutine_loop_exits() {
    let source = r"async def sample(groups, left, right, verify):
    for items in groups:
        for item in items:
            values = item
            if left(item) or any(right(value) for value in values):
                assert verify(item)
";
    for (name, source) in [
        ("nested", source.to_owned()),
        (
            "single_iterator",
            source
                .replace(
                    "    for items in groups:\n        for item in items:",
                    "    for item in groups:",
                )
                .replace("\n            ", "\n        "),
        ),
        (
            "no_prefix",
            source
                .replace("            values = item\n", "")
                .replace(" in values)", " in item)"),
        ),
        (
            "manager",
            source
                .replace("\n    ", "\n        ")
                .replace("verify):\n", "verify, manager):\n    with manager():\n"),
        ),
        (
            "ordinary_calls",
            source.replace("any(right(value) for value in values)", "right(values)"),
        ),
        ("lone_generator", source.replace("left(item) or ", "")),
        (
            "conjunction",
            source.replace("left(item) or ", "left(item) and "),
        ),
        (
            "live_body",
            source.replace("assert verify(item)", "verify(item)"),
        ),
        (
            "explicit_else",
            format!("{source}            else:\n                assert verify(item)\n"),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("erased_generator_guard_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_nested_else_preserves_coroutine_loop_exits() {
    let source = r"async def sample(items, selected, verify):
    for item in items:
        if selected:
            assert verify(item)
        else:
            assert verify(item)
            if item is not None and item.matches():
                assert verify(item)
            else:
                assert verify(item)
            assert verify(item)
";
    for (name, source) in [
        ("nested_else", source.to_owned()),
        (
            "no_prefix",
            source.replace(
                "        else:\n            assert verify(item)\n",
                "        else:\n",
            ),
        ),
        (
            "three_leaves",
            source.replace("item.matches()", "item.matches() and selected"),
        ),
        (
            "live_else",
            source.replace(
                "            else:\n                assert verify(item)",
                "            else:\n                verify(item)",
            ),
        ),
        (
            "live_suffix",
            format!(
                "{}            verify(item)\n",
                source
                    .strip_suffix("            assert verify(item)\n")
                    .unwrap(),
            ),
        ),
    ] {
        assert_matches_cpython(&source, &format!("erased_nested_else_{name}.py"), &LEVELS);
    }
}

#[test]
fn erased_final_else_suffix_preserves_coroutine_loop_exits() {
    let source = r"async def sample(items, selected, prepare, work, verify):
    for item in items:
        if selected:
            value = await prepare(item)
            assert verify(value)
        else:
            value = None
            if item is not None:
                work(item)
                value = item
            assert await verify(value)
";
    for (name, source) in [
        ("final_else", source.to_owned()),
        (
            "no_prefix",
            source
                .replace("            value = None\n", "")
                .replace("                value = item\n", "")
                .replace("assert await verify(value)", "assert verify(item)"),
        ),
        (
            "synchronous",
            source.replace("async def ", "def ").replace("await ", ""),
        ),
        (
            "live_suffix",
            source.replace("assert await verify(value)", "await verify(value)"),
        ),
    ] {
        assert_matches_cpython(&source, &format!("erased_final_else_{name}.py"), &LEVELS);
    }
}

#[test]
fn while_else_nonfinal_condition_preserves_forward_exit() {
    for coroutine in [false, true] {
        for loop_else in [false, true] {
            for condition in [
                "left and right",
                "left or right",
                "ready(left) and ready(right)",
                "left and right and third",
            ] {
                let header = if coroutine { "async def" } else { "def" };
                let mut source = format!(
                    "{header} sample(flag, left, right, third):
    result = None
    while active():
        value = obtain()
        if flag:
            if {condition}:
                result = value
        elif complete(value):
            break
"
                );
                if loop_else {
                    source.push_str("    else:\n        raise RuntimeError(flag)\n");
                }
                source.push_str("    return result\n");
                assert_matches_cpython(
                    &source,
                    &format!("while_else_{coroutine}_{loop_else}_{condition}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn managed_while_else_preserves_nested_branch_exits() {
    let source = r"def sample(left, right, flag):
    with acquire() as worker, acquire() as other:
        result = None
        while worker.active():
            value = worker.obtain()
            if flag:
                if CONDITION:
                    result = value
            elif other.complete(value):
                break
        else:
            raise RuntimeError(flag)
    return result
";
    for condition in [
        "left",
        "left and right",
        "left or right",
        "left < value < right",
    ] {
        let source = source.replace("CONDITION", condition);
        assert_matches_cpython(&source, "managed_while_else.py", &LEVELS);
        assert_matches_cpython(
            &source.replace("def sample", "async def sample"),
            "managed_coroutine_while_else.py",
            &LEVELS,
        );
    }
}

#[test]
fn coroutine_for_else_return_keeps_shared_branch_exit() {
    let source = r"async def sample(items, step, flag, left, right, third, work):
    for item in items:
        event = await step(item)
        if event is None:
            break
        if flag:
            value = event.value
            if left == work(value) and right == value.kind and third:
                return value.result
        elif event.other:
            work(event)
    else:
        return None
    return False
";
    for (name, source) in [
        ("and", source.to_owned()),
        ("two_leaves", source.replace(" and third", "")),
        ("or", source.replace(" and ", " or ")),
        (
            "single",
            source.replace(
                "left == work(value) and right == value.kind and third",
                "left",
            ),
        ),
        (
            "no_loop_else",
            source.replace("    else:\n        return None\n", ""),
        ),
        (
            "synchronous",
            source.replace("async def", "def").replace("await ", ""),
        ),
        ("no_await", source.replace("await ", "")),
        (
            "return_call",
            source.replace("return value.result", "return work(value)"),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("coroutine_for_else_return_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_coroutine_final_elif_preserves_shared_backedge() {
    let source = r"async def sample(items, manager, verify):
    async with manager():
        for item in items:
            if item.ready():
                assert verify(item)
            elif item.kind == 2 or item.matches():
                assert verify(item)
";
    for (name, source) in [
        ("async_manager", source.to_owned()),
        (
            "nested",
            source.replace(
                "            elif item.kind == 2 or item.matches():\n                assert verify(item)",
                "            else:\n                if item.kind == 2 or item.matches():\n                    assert verify(item)",
            ),
        ),
        (
            "plain",
            source
                .replace("    async with manager():\n", "")
                .replace("\n    ", "\n"),
        ),
        (
            "live_first",
            source.replacen("assert verify(item)", "verify(item)", 1),
        ),
        ("live_suffix", format!("{source}            verify(item)\n")),
    ] {
        assert_matches_cpython(&source, &format!("erased_final_elif_{name}.py"), &LEVELS);
    }
}

#[test]
fn completed_managed_stores_preserve_iterator_backedges() {
    let source = r"async def sample(items, outer, resource, work, convert, flag):
    async with outer():
        with resource():
            for item in items:
                ready = await work(item)
                if flag is item:
                    value = convert(next(entry for entry in ready if entry.active))
                    assert value
                else:
                    assert item
";
    let generator = "convert(next(entry for entry in ready if entry.active))";
    for (name, source) in [
        ("generator_store", source.to_owned()),
        ("generator_attribute", source.replace("value", "item.value")),
        (
            "generator_sync_manager",
            source
                .replace("    async with outer():\n", "")
                .replace("\n    ", "\n"),
        ),
        (
            "list_store",
            source.replace(generator, "[entry.code for entry in ready if entry.active]"),
        ),
        (
            "set_store",
            source.replace(generator, "{entry.code for entry in ready if entry.active}"),
        ),
        (
            "dict_store",
            source.replace(
                generator,
                "{entry.code: entry.value for entry in ready if entry.active}",
            ),
        ),
        (
            "combined_work",
            r"async def sample(items, outer, resource, work, convert, flag):
    async with outer():
        with resource():
            for item in items:
                ready = await work(item)
                if flag is item:
                    chosen = ready if item.active else items
                    value = convert(next(entry for entry in chosen if entry.active))
                    assert value
                    if flag:
                        value = item.value
                        assert value
                    else:
                        assert item
                    selected = [entry for entry in ready if entry.active]
                    result = await work(selected)
                    item.value = convert(next(entry for entry in ready if entry.active))
                    item.ready = result
                else:
                    assert item
"
            .to_owned(),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("completed_managed_store_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_managed_arms_preserve_iterator_backedges() {
    let source = r"async def sample(items, outer, resource, work, convert, flag, active):
    async with outer():
        with resource():
            for item in items:
                ready = await work(item)
                if flag is item:
                    value = convert(next(entry for entry in ready if entry.active))
                    assert value
                    if active:
                        value = ready[0]
                        assert value
                    else:
                        assert ready
                    assert item
                else:
                    assert item
";
    for (name, source) in [
        ("base", source.to_owned()),
        (
            "no_generator",
            source.replace(
                "value = convert(next(entry for entry in ready if entry.active))",
                "value = ready[0]",
            ),
        ),
        (
            "no_inner_else",
            source.replace(
                "                    else:\n                        assert ready\n",
                "",
            ),
        ),
        (
            "no_trailing_assert",
            source.replace(
                "                        assert ready\n                    assert item\n",
                "                        assert ready\n",
            ),
        ),
        (
            "suspending_prefix",
            source.replace(
                "                    if active:\n",
                "                    await work(ready)\n                    if active:\n",
            ),
        ),
    ] {
        assert_matches_cpython(&source, &format!("nested_managed_arm_{name}.py"), &LEVELS);
    }
}

#[test]
fn erased_iterator_else_preserves_saved_cleanup_owners() {
    for body in [
        "assert result",
        "count = action(result)\n                assert count",
    ] {
        let sources = [
            format!(
                "def sample(items, state):
    try:
        action(state)
    except Exception:
        for item in items:
            result = action(item)
            if item == state:
                {body}
            else:
                assert result is None
"
            ),
            format!(
                "def sample(items, state):
    try:
        with manager():
            for item in items:
                result = action(item)
                if item == state:
                    {body}
                else:
                    assert result is None
    finally:
        finish(state)
"
            )
            .replace(
                "\n                assert count",
                "\n                    assert count",
            ),
            format!(
                "async def sample(items, state):
    try:
        async with manager():
            for item in items:
                result = await action(item)
                if item == state:
                    {body}
                else:
                    assert result is None
    finally:
        await finish(state)
"
            )
            .replace(
                "\n                assert count",
                "\n                    assert count",
            ),
        ];
        for source in sources {
            assert_matches_cpython(&source, "erased_iterator_saved_owners.py", &LEVELS);
        }
    }
    for body in ["break", "continue", "return value"] {
        let source = format!(
            "def sample(value, state, items):
    try:
        for item in items:
            if value:
                {body}
            else:
                assert state
    finally:
        finish(state)
"
        );
        assert_matches_cpython(&source, "erased_iterator_branch_exit.py", &LEVELS);
    }
    for source in [
        r"def sample(value, state, items):
    for item in items:
        with manager():
            if value:
                assert state
            else:
                assert value is None
",
        r"def sample(value, state, items):
    for item in items:
        try:
            action(item)
        except Exception:
            if value:
                assert state
            else:
                assert value is None
",
    ] {
        assert_matches_cpython(source, "erased_iterator_nested_owner.py", &LEVELS);
    }
}

#[test]
fn erased_iterator_else_preserves_linear_generator_creation() {
    for expression in [
        "(part for part in item)",
        "consume(part for part in item)",
        "sum(part for part in item)",
        "action(sum(part for part in item))",
        "sum((lambda: part) for part in item)",
        "sum((part or state) for part in item)",
        "sum((part if value else state) for part in item)",
        "sum(part for part in item if part and state)",
        "sum(part for group in item for part in (group if value else items))",
        "sum(part + value for part in item)",
        "sum(part for part in prepare(item))",
    ] {
        for prefix in ["", "async "] {
            let source = format!(
                "{prefix}def sample(value, state, items):
    try:
        with manager():
            for item in items:
                if value:
                    result = {expression}
                    assert result
                else:
                    assert state
    finally:
        finish(state)
"
            );
            assert_matches_cpython(&source, "linear_generator_iterator_else.py", &LEVELS);
        }
    }
}

#[test]
fn completed_manager_prefix_keeps_erased_conditional_tail() {
    let source = "async def sample(mode, expected, task):\n    try:\n        if mode == 'cancel':\n            with raises(CancelledError):\n                await task\n        else:\n            with raises(RuntimeError if expected else Failure) as error:\n                await task\n            if expected:\n                assert error.value is expected\n            else:\n                assert error.value.code == 'unavailable'\n    finally:\n        await gather(task, return_exceptions=True)\n";
    for source in [
        source.to_owned(),
        source.replace("RuntimeError if expected else Failure", "Failure"),
        source.replace("await task\n", "await wait(task)\n"),
        source.replace(
            "with raises(RuntimeError if expected else Failure) as error:\n                await task",
            "expected = (primary_error if mode in ('primary', 'both') else backup_error) if mode else None",
        ),
    ] {
        assert_matches_cpython(&source, "completed_manager_tail.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_preserve_nonfinal_while_branch_locations() {
    let source = r"PREFIXdef sample(values, state):
    count = len(values)
    if state == 0:
        consume(values)
    elif state == 1:
        while count:
            consume(count)
            count -= 1
        assert ready(values)
    else:
        consume(state)
    finish(values)
";
    for prefix in ["", "async "] {
        for (name, source) in [
            ("plain", source.to_owned()),
            (
                "call_condition",
                source.replace("while count:", "while ready(count):"),
            ),
            (
                "compound_condition",
                source.replace("while count:", "while count and state:"),
            ),
            (
                "break",
                source.replace("            count -= 1", "            break"),
            ),
            (
                "while_else",
                source.replace(
                    "        assert ready(values)",
                    "        else:\n            consume(values)\n        assert ready(values)",
                ),
            ),
            (
                "live_suffix",
                source.replace("assert ready(values)", "consume(values)"),
            ),
        ] {
            assert_matches_cpython(
                &source.replace("PREFIX", prefix),
                &format!("nonfinal_while_assert_{name}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn nested_conditional_stores_preserve_saved_iterator_owners() {
    for (index, expression) in [
        "(3 if flag else 5) if item == 1 else 7",
        "3 if item == 1 else (5 if flag else 7)",
        "(3 if flag else 5) if item == 1 else (7 if flag else 9)",
    ]
    .into_iter()
    .enumerate()
    {
        for manager in ["manager()", "manager() as owner"] {
            for prefix in ["", "async "] {
                let source = format!(
                    "{prefix}def sample(values, flag, manager):
    with {manager}:
        for item in values:
            if item > 0:
                value = {expression}
                seen = observe(value)
                assert seen
            else:
                assert item == 0
"
                );
                assert_matches_cpython(
                    &source,
                    &format!("nested_iterator_store_{index}_{manager}_{prefix}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn managed_iterator_assertions_preserve_conditional_backedges() {
    let indent = |source: &str| {
        let mut result = String::new();
        for line in source.lines() {
            result.push_str("    ");
            result.push_str(line);
            result.push('\n');
        }
        result
    };
    for managed in [false, true] {
        for in_loop in [false, true] {
            for nested in [false, true] {
                for following in [false, true] {
                    for ordinary_assert in [false, true] {
                        let mut core = String::from("value = transform(item)\n");
                        if nested {
                            core.push_str("if value is None:\n    value = replace(item)\n");
                        }
                        core.push_str(if ordinary_assert {
                            "assert valid(value)\n"
                        } else {
                            "assert value is not None, \"missing value\"\n"
                        });
                        if following {
                            core.push_str("observe(value)\n");
                        }
                        core = format!("if flag:\n{}", indent(&core));
                        core = if in_loop {
                            format!("for item in items:\n{}", indent(&core))
                        } else {
                            format!("item = items\n{core}")
                        };
                        core.push_str("return finish(items)\n");
                        if managed {
                            core = format!("with manager():\n{}", indent(&core));
                        }
                        let source =
                            format!("def sample(manager, items, flag):\n{}", indent(&core));
                        assert_matches_cpython(&source, "managed_iterator_assertion.py", &LEVELS);
                    }
                }
            }
        }
    }
}

#[test]
fn erased_elif_arms_preserve_saved_iterator_owners() {
    let indent = |body: &str| {
        let mut result = String::new();
        for line in body.lines() {
            result.push_str("    ");
            result.push_str(line);
            result.push('\n');
        }
        result
    };
    for first in [
        "assert flag\nresult = action(item)\nassert result\n",
        "assert flag\naction(item)\nassert item\n",
        "assert flag\n",
    ] {
        for condition in ["not value", "value"] {
            for finalized in [false, true] {
                let branch = format!(
                    "if flag:\n{}elif {condition}:\n    assert item\nelse:\n    assert value\n",
                    indent(first),
                );
                let body = format!(
                    "for item in items:\n    value = read(item)\n{}finish(items)\n",
                    indent(&branch),
                );
                let body = format!("with resource() as handle:\n{}", indent(&body));
                let body = if finalized {
                    format!("try:\n{}finally:\n    close(items)\n", indent(&body))
                } else {
                    body
                };
                let source = format!("def sample(items, flag):\n{}", indent(&body));
                assert_matches_cpython(&source, "erased_elif_saved_iterator_owners.py", &LEVELS);
            }
        }
    }
}

#[test]
fn completed_await_iterator_arms_preserve_saved_cleanup_owners() {
    for managers in 0..=2 {
        for guarded in [false, true] {
            for awaited_finalizer in [false, true] {
                let mut body = String::from("await action(item)\n");
                let indent = |body: &str| {
                    let mut indented = String::new();
                    for line in body.lines() {
                        indented.push_str("    ");
                        indented.push_str(line);
                        indented.push('\n');
                    }
                    indented
                };
                if guarded {
                    body = format!("with guard():\n{}", indent(&body));
                }
                body = format!("if item:\n{}else:\n    assert flag\n", indent(&body));
                body = format!("for item in values:\n{}", indent(&body));
                let finalizer = if awaited_finalizer {
                    "await finish()"
                } else {
                    "finish()"
                };
                body = format!("try:\n{}finally:\n    {finalizer}\n", indent(&body));
                for _ in 0..managers {
                    body = format!("async with gate():\n{}", indent(&body));
                }
                let source = format!("async def scenario(values, flag):\n{}", indent(&body));
                assert_matches_cpython(
                    &source,
                    &format!(
                        "completed_iterator_await_{managers}_{guarded}_{awaited_finalizer}.py"
                    ),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn managed_mixed_erased_iterator_guard_preserves_shared_backedge() {
    let source = r"def sample(items, manager, check):
    with manager():
        for item in items:
            if item == 1 or item > 2 and item < 5:
                assert check(item)
";
    for (name, source) in [
        ("synchronous", source.to_owned()),
        (
            "coroutine",
            source.replace("def sample", "async def sample"),
        ),
        (
            "async_manager",
            source
                .replace("def sample", "async def sample")
                .replace("with manager()", "async with manager()"),
        ),
        (
            "conjunction",
            source.replace(
                "item == 1 or item > 2 and item < 5",
                "item > 2 and (item == 1 or item < 5)",
            ),
        ),
        (
            "three_false_edges",
            source.replace(
                "item > 2 and item < 5",
                "item > 2 and item < 5 and item != 4",
            ),
        ),
        (
            "disjunction",
            source.replace("item > 2 and item < 5", "item > 2"),
        ),
        (
            "live_body",
            source.replace("assert check(item)", "check(item)"),
        ),
        (
            "inner_manager",
            source.replace(
                "    with manager():\n        for item in items:",
                "    for item in items:\n        with manager():",
            ),
        ),
    ] {
        assert_matches_cpython(&source, &format!("managed_mixed_guard_{name}.py"), &LEVELS);
    }
}

#[test]
fn iterator_handler_returns_preserve_conditional_finalizer_entry() {
    let source = r"def sample(values, flag):
    resource = None
    try:
        while True:
            for value in values:
                if resource is not None:
                    break
                try:
                    resource = acquire(value)
                    break
                except OSError as error:
                    if flag:
                        report(error)
                        return
                    report(error)
    finally:
        if resource is not None:
            release(resource)
";
    for asynchronous in [false, true] {
        for explicit_none in [false, true] {
            for (index, report) in [
                "report(error)",
                "logger.report(error)",
                "logger.report('message')",
            ]
            .into_iter()
            .enumerate()
            {
                let mut source = source.replacen(
                    "                        report(error)",
                    &format!("                        {report}"),
                    1,
                );
                if asynchronous {
                    source = source.replace("def sample", "async def sample");
                    for call in ["acquire(value)", "release(resource)"] {
                        source = source.replace(call, &format!("await {call}"));
                    }
                }
                if explicit_none {
                    source = source.replace(
                        "                        return\n",
                        "                        return None\n",
                    );
                }
                assert_matches_cpython(
                    &source,
                    &format!("iterator_handler_entry_{asynchronous}_{explicit_none}_{index}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn boolean_returns_preserve_enclosing_named_handler_owners() {
    let source = r"def sample(items, flag):
    for value in items:
        try:
            return work(value)
        except OSError as outer:
            if flag:
                try:
                    value = retry(value)
                except ValueError as middle:
                    if flag:
                        try:
                            value = fallback(value)
                        except RuntimeError:
                            report(value)
                            return False
                        else:
                            return value
                    return False
                else:
                    return value
            return False
    return False
";
    for asynchronous in [false, true] {
        for iterated in [false, true] {
            for prefixed in [false, true] {
                for returned in ["False", "True"] {
                    let mut source = source.replace("return False", &format!("return {returned}"));
                    if asynchronous {
                        source = source.replace("def sample", "async def sample");
                        for call in ["work(value)", "retry(value)", "fallback(value)"] {
                            source = source.replace(call, &format!("await {call}"));
                        }
                    }
                    if !prefixed {
                        source = source.replace("                            report(value)\n", "");
                    }
                    if !iterated {
                        let mut dedented = String::with_capacity(source.len());
                        for line in source.lines() {
                            if line == "    for value in items:" {
                                continue;
                            }
                            let line = if line.starts_with("        ") {
                                &line[4..]
                            } else {
                                line
                            };
                            writeln!(dedented, "{line}").unwrap();
                        }
                        source = dedented;
                    }
                    assert_matches_cpython(
                        &source,
                        &format!(
                            "named_boolean_return_{asynchronous}_{iterated}_{prefixed}_{returned}.py"
                        ),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn three_handler_returns_preserve_each_outer_cleanup_owner() {
    let source = r"def sample(value):
    try:
        work(value)
    except TypeError:
        try:
            retry(value)
        except TypeError:
            try:
                fallback(value)
            except Exception:
                return recover(value)
";
    for asynchronous in [false, true] {
        for named in [false, true] {
            for returned in ["recover(value)", "value"] {
                let mut source =
                    source.replace("return recover(value)", &format!("return {returned}"));
                if asynchronous {
                    source = source.replace("def sample", "async def sample");
                    for call in ["work(value)", "retry(value)", "fallback(value)"] {
                        source = source.replace(call, &format!("await {call}"));
                    }
                }
                if named {
                    source = source.replace("except Exception:", "except Exception as error:");
                }
                assert_matches_cpython(&source, "three_handler_return.py", &LEVELS);
            }
        }
    }
}

#[test]
fn three_handler_returns_preserve_owners_before_iterator_cleanup() {
    let source = r"def sample(values):
    for value in values:
        try:
            work(value)
        except TypeError:
            try:
                retry(value)
            except TypeError:
                try:
                    fallback(value)
                except Exception:
                    return recover(value)
";
    for asynchronous in [false, true] {
        for named in [false, true] {
            for returned in ["recover(value)", "value"] {
                let mut source =
                    source.replace("return recover(value)", &format!("return {returned}"));
                if asynchronous {
                    source = source.replace("def sample", "async def sample");
                    for call in ["work(value)", "retry(value)", "fallback(value)"] {
                        source = source.replace(call, &format!("await {call}"));
                    }
                }
                if named {
                    source = source.replace("except Exception:", "except Exception as error:");
                }
                assert_matches_cpython(&source, "three_handler_iterator_return.py", &LEVELS);
            }
        }
    }
}

#[test]
fn boolean_handler_returns_preserve_shared_finally_borrows() {
    let source = r"def sample(item):
    target = None
    try:
        target = open_value(item)
        work()
    except LookupError:
        if CONDITION:
            return
COMPANION    finally:
        if target is not None:
            try:
                close(target)
            except OSError:
                pass
    finish()
";
    for (index, (condition, companion)) in [
        ("ready(item) and valid(target)", ""),
        ("ready(item)", "    except OSError:\n        pass\n"),
        (
            "ready(item)",
            "    except OSError as error:\n        recover(error)\n",
        ),
        (
            "ready(item) and valid(target)",
            "    except OSError:\n        pass\n",
        ),
        (
            "ready(item) or valid(target)",
            "    except OSError:\n        recover()\n",
        ),
        (
            "ready(item) and (valid(target) or item == target)",
            "    except OSError:\n        pass\n",
        ),
        (
            "ready(item) and valid(target) and item is not None and check(item.value) and target.value == item.value",
            "    except OSError:\n        pass\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let source = source.replace("CONDITION", condition).replace("COMPANION", companion);
        assert_matches_cpython(&source, &format!("boolean_handler_finally_borrow_{index}.py"), &LEVELS);
    }
}

#[test]
fn conditional_handler_return_preserves_normal_finally_borrows() {
    let source = r"def sample(item):
    try:
        work()
    except LookupError:
        if ready(item):
            return
    finally:
        if item is not None:
            close(item)
    finish()
";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "            close(item)",
            "            try:\n                close(item)\n            except OSError:\n                pass",
        ),
        source.replace("            return\n", "            return\n        recover()\n"),
        source.replace("        if ready(item):\n            return", "        return"),
        source.replace("        if item is not None:\n            close(item)", "        close(item)"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("conditional_handler_finally_borrow_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn conditional_iterator_return_copies_preserve_locations_and_owners() {
    let source = r"async def sample(items, manager, state):
    try:
        for item in items:
            try:
                with manager:
                    if item:
                        return make_value()
                    return await state.read()
            except LookupError:
                if item:
                    raise
            finally:
                if state is not None:
                    await state.close()
                    state = None
    except ValueError:
        return failed()
    except Exception as error:
        return recover(error)
";
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def").replace("await ", ""),
        source.replace("with manager:", "with manager if item else fallback():"),
        source.replace("                    state = None\n", ""),
        source
            .replace(" as error", "")
            .replace("recover(error)", "recover()"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            &source,
            &format!("conditional_iterator_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py315],
        );
    }
}

#[test]
fn continue_copies_cleanup_after_completed_finalizer_prefixes() {
    let source = r"def sample(manager, resource, work, note, choice):
    with manager:
        while True:
            try:
                yield work()
            except ValueError as error:
                note(error)
                continue
            finally:
                PREFIX
                if resource is not None:
                    try:
                        resource.close()
                    except Exception:
                        pass
            return
";
    for (index, prefix) in [
        "note()",
        "if choice:\n                    note()\n                else:\n                    work()",
        "try:\n                    work()\n                except Exception:\n                    note()",
        "with manager:\n                    note()",
        "try:\n                    work()\n                except Exception:\n                    with manager:\n                        note()\n                if choice:\n                    note()",
        "yield note()",
    ]
    .into_iter()
    .enumerate()
    {
        let source = source.replace("PREFIX", prefix);
        assert_matches_cpython(
            &source,
            &format!("prefixed_finally_continue_{index}.py"),
            &LEVELS,
        );
    }
    assert_matches_cpython(
        &source
            .replace("PREFIX", "note()")
            .replace("yield work()", "work()"),
        "prefixed_finally_continue_sync.py",
        &LEVELS,
    );
}

#[test]
fn continue_copies_inner_cleanup_inside_an_enclosing_finalizer() {
    let source = r"async def sample(state, cleanup):
    try:
        while True:
            value = None
            try:
                if await state.ready():
                    continue
                value = await state.next()
            finally:
                if value is not None:
                    try:
                        await cleanup(value)
                    except Exception:
                        pass
    finally:
        await state.close()
";
    let mut nested_body = String::new();
    for line in source.lines().skip(1) {
        writeln!(nested_body, "    {line}").unwrap();
    }
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def").replace("await ", ""),
        source.replace(
            "                        await cleanup(value)",
            "                        cleanup(value)",
        ),
        source.replace("value is not None", "value is None"),
        format!("async def sample(state, cleanup):\n    try:\n{nested_body}    finally:\n        await state.finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("enclosing_finally_continue_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn negated_finally_guard_preserves_copied_continue_owner() {
    let source = r"async def sample(state, cleanup, recover):
    while True:
        released = False
        try:
            if await state.ready():
                continue
            await state.work()
        finally:
            if not released:
                try:
                    await cleanup()
                except Exception:
                    recover()
";
    let mut nested_body = String::new();
    for line in source.lines().skip(1) {
        writeln!(nested_body, "    {line}").unwrap();
    }
    for (index, source) in [
        source.to_owned(),
        source.replace("                    recover()", "                    pass"),
        source.replace("await cleanup()", "cleanup()"),
        source.replace("await state.ready()", "state.ready()"),
        format!("async def sample(state, cleanup, recover):\n    try:\n{nested_body}    finally:\n        await state.close()\n"),
        source.replace(
            "                    recover()",
            "                    recover()\n            else:\n                state.finish()",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("negated_finally_continue_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn conditional_finalizer_recovery_preserves_borrow_boundary() {
    let source = r"def sample(resource, work, flag):
    with resource() as state:
        try:
            result = work()
        finally:
            if flag == 1:
                try:
                    work()
                except Exception:
                    pass
            elif flag == 2:
                work()
            work()
        if state.failed:
            raise ValueError
        return result
";
    let unmanaged = r"def sample(state, work, flag):
    try:
        result = work()
    finally:
        if flag == 1:
            try:
                work()
            except Exception:
                pass
        elif flag == 2:
            work()
        work()
    if state.failed:
        raise ValueError
    return result
";
    for (name, source) in [
        ("base", source.to_owned()),
        ("no_manager", unmanaged.to_owned()),
        (
            "no_elif",
            source.replace("            elif flag == 2:\n                work()\n", ""),
        ),
        (
            "no_inner_handler",
            source.replace(
                "                try:\n                    work()\n                except Exception:\n                    pass\n",
                "                work()\n",
            ),
        ),
    ] {
        assert_matches_cpython(&source, &format!("conditional_finalizer_{name}.py"), &LEVELS);
    }
}

#[test]
fn named_finalizer_elif_recovery_keeps_owned_continuation() {
    for condition in ["flag", "flag and value"] {
        for cleanup in [
            "action(value)",
            "with guard(value):\n                    action(value)",
        ] {
            for outer_handler in [
                "",
                "    except Exception as outer_error:\n        state['error'] = str(outer_error)\n",
            ] {
                let source = format!(
                    "def sample(flag, value):
    state = {{}}
    try:
        action(value)
{outer_handler}    finally:
        if {condition}:
            try:
                {cleanup}
            except Exception as error:
                state['error'] = str(error)
        elif value:
            state['phase'] = 'missing'
        finish(value)
        restore(value)
    consume(state)
    return state
"
                );
                assert_matches_cpython(&source, "named_finalizer_elif.py", &LEVELS);
            }
        }
    }
}

#[test]
fn named_elif_cleanup_retains_recovery_and_conditional_suffix() {
    for managed in [false, true] {
        for condition in ["ready", "result is not None and not result.failed"] {
            for recovery_suffix in [
                "",
                "        try:\n            resource.close()\n        except Error as error:\n            report(error)\n",
            ] {
                for successor in ["", "    consume(resource, result)\n"] {
                    let work = if managed {
                        "        with resource.collect():\n            result = resource.run()\n"
                    } else {
                        "        result = resource.run()\n"
                    };
                    let source = format!(
                        "def sample(resource, context, result, toolbox, ready):\n    try:\n{work}    finally:\n        if context is not None:\n            try:\n                context.clean(result)\n            except Error as error:\n                report(error)\n        elif toolbox is not None:\n            try:\n                toolbox.close()\n            except Error as error:\n                report(error)\n{recovery_suffix}        record(resource, outcome='ok' if {condition} else 'error')\n{successor}    return resource.finish(result)\n"
                    );
                    assert_matches_cpython(&source, "named_elif_cleanup_suffix.py", &LEVELS);
                }
            }
        }
    }
}

#[test]
fn conditional_finalizer_preserves_tuple_return_copy_order() {
    let source = r"async def sample(work, flag, left, right):
    try:
        await work()
    except Exception:
        raise
    finally:
        try:
            await work()
        finally:
            if not flag:
                try:
                    await work()
                except Exception as error:
                    work(error)
    return left, right
";
    for (index, source) in [
        source.to_owned(),
        source.replace("    except Exception:\n        raise\n", ""),
        source
            .replace("except Exception as error:", "except Exception:")
            .replace("work(error)", "work()"),
        source.replace("return left, right", "return left"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("conditional_finalizer_tuple_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn guarded_finalizer_handlers_share_tuple_return_copies() {
    let source = r"def sample(first, second, condition, callback):
    try:
        work()
    finally:
        if condition:
            try:
                callback()
            except Exception:
                if condition:
                    recover()
                else:
                    raise
    return first, second
";
    for (index, source) in [
        source.to_owned(),
        source
            .replace("def sample", "async def sample")
            .replace("work()", "await work()")
            .replace("callback()", "await callback()"),
        source.replace("    finally:", "    except LookupError:\n        raise\n    finally:"),
        source.replace("                else:\n                    raise\n", ""),
        source.replace(
            "                if condition:\n                    recover()\n                else:\n                    raise",
            "                recover()",
        ),
        source.replacen("if condition:", "if condition or first and second:", 1),
        source
            .replace("def sample", "async def sample")
            .replace("callback()", "await callback()")
            .replacen("if condition:", "if condition or first and second:", 1),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("guarded_finalizer_tuple_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn managed_handler_finalizer_jump_keeps_enclosing_owner() {
    let source = r"async def sample(outer, inner, action, recover, flag):
    with outer:
        try:
            await action()
        except ValueError:
            raise
        except RuntimeError as error:
            recover(error)
        finally:
            if flag:
                try:
                    action()
                except Exception:
                    if not flag:
                        raise
                    with inner:
                        recover()
    action()
";
    for (name, source) in [
        ("coroutine", source.to_owned()),
        (
            "synchronous",
            source
                .replace("async def", "def")
                .replace("await action()", "action()"),
        ),
        (
            "async_inner",
            source.replace("with inner:", "async with inner:"),
        ),
        (
            "completed_assignment",
            source.replace(
                "\n                try:\n",
                "\n                value = action()\n                try:\n",
            ),
        ),
        (
            "awaited_recovery",
            source.replace(
                "\n                    action()\n",
                "\n                    await action()\n",
            ),
        ),
        (
            "assignment_and_await",
            source
                .replace(
                    "\n                try:\n",
                    "\n                value = action()\n                try:\n",
                )
                .replace(
                    "\n                    action()\n",
                    "\n                    await action()\n",
                ),
        ),
        (
            "awaited_assignment",
            source.replace(
                "\n                try:\n",
                "\n                value = await action()\n                try:\n",
            ),
        ),
        (
            "yield_prefix",
            source
                .replace("async def", "def")
                .replace("await action()", "action()")
                .replace(
                    "\n                try:\n",
                    "\n                yield flag\n                try:\n",
                ),
        ),
        (
            "yield_from_prefix",
            source
                .replace("async def", "def")
                .replace("await action()", "action()")
                .replace(
                    "\n                try:\n",
                    "\n                yield from recover()\n                try:\n",
                ),
        ),
        (
            "no_outer_manager",
            source
                .replace("    with outer:\n", "")
                .replace("\n        ", "\n    "),
        ),
        (
            "trailing_finalizer_call",
            source.replace(
                "\n    action()\n",
                "\n            recover()\n    action()\n",
            ),
        ),
        (
            "no_inner_guard",
            source.replace(
                "                    if not flag:\n                        raise\n",
                "",
            ),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("managed_handler_owner_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn conditional_manager_finally_copies_iterator_cleanup() {
    let source = r"async def sample(items, first, second, manager, work, cleanup, finish):
    for item in items:
        try:
            await work(item)
            break
        except LookupError:
            pass
        except ValueError:
            pass
        finally:
            if first:
                await cleanup()
            elif second:
                with manager:
                    await cleanup()
    finish(first)
    finish(second)
    return finish()
";
    for (name, source) in [
        ("full_shape", source.to_owned()),
        (
            "no_handlers",
            source.replace("        except LookupError:\n            pass\n        except ValueError:\n            pass\n", ""),
        ),
        (
            "direct_manager",
            source.replace("            if first:\n                await cleanup()\n            elif second:\n                with manager:\n                    await cleanup()", "            with manager:\n                await cleanup()"),
        ),
        ("synchronous", source.replace("async def", "def").replace("await ", "")),
        ("none_guards", source.replace("if first:", "if first is None:").replace("elif second:", "elif second is None:")),
    ] {
        assert_matches_cpython(&source, &format!("conditional_manager_cleanup_{name}.py"), &LEVELS);
    }
}

#[test]
fn conditional_manager_preserves_shared_subscript_return() {
    let source = r"def sample(key, root, cache):
    try:
        if key:
            with root:
                pass
        else:
            work()
    except Exception:
        pass
    return cache[key]
";
    for (name, source) in [
        ("subscript", source.to_owned()),
        ("tuple", source.replace("cache[key]", "(cache, key)")),
        ("name", source.replace("cache[key]", "cache")),
        ("constant", source.replace("cache[key]", "None")),
        ("call", source.replace("cache[key]", "cache(key)")),
        (
            "final_arm",
            source.replace(
                "            with root:\n                pass\n        else:\n            work()",
                "            work()\n        else:\n            with root:\n                pass",
            ),
        ),
        (
            "no_outer_try",
            "def sample(key, root, cache):\n    if key:\n        with root:\n            pass\n    else:\n        work()\n    return cache[key]\n".to_owned(),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("conditional_manager_return_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn retired_managed_handlers_keep_outer_try_ownership_for_finalizer_copies() {
    let source = r#"async def sample(items, flag, resource, logger, parser):
    try:
        if flag:
            try:
                for item in items:
                    try:
                        if flag:
                            return result("early")
                        async with manage(item):
                            async for entry in entries:
                                try:
                                    value = parser.read(entry)
                                except (ValueError, TypeError):
                                    reason = "left" if parser.check(parser.option("mode", default="")) else "right"
                                    logger.record("failure", reason)
                                    return result("failed")
                                logger.record(entry, value)
                                if flag:
                                    return result("done")
                        if flag:
                            return result("finished")
                        resource.count += 1
                    finally:
                        try:
                            await resource.finish()
                        except OSError:
                            await resource.recover()
                            raise
                return result("empty")
            except Exception:
                logger.record("outer", flag)
                return result("recovered")
        return result("skipped")
    except BaseException:
        logger.record("final", flag)
        return result("aborted")
    finally:
        logger.complete(resource)
"#;
    for asynchronous_manager in [false, true] {
        for asynchronous_iterator in [false, true] {
            for returned in ["value", "result(\"failed\")"] {
                let mut source =
                    source.replace("return result(\"failed\")", &format!("return {returned}"));
                if !asynchronous_manager {
                    source = source.replace("async with manage(item):", "with manage(item):");
                }
                if !asynchronous_iterator {
                    source = source.replace("async for entry in entries:", "for entry in entries:");
                }
                assert_matches_cpython(
                    &source,
                    &format!(
                        "retired_handler_{asynchronous_manager}_{asynchronous_iterator}_{returned}.py"
                    ),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn nested_tries_own_copied_finalizer_handler_cleanup() {
    let source = r"async def sample(items, flag):
    try:
        if flag:
            try:
                for item in items:
                    try:
                        if flag:
                            return result(False)
                        await work(item)
                        return result(True)
                    finally:
                        try:
                            await finish(item)
                        except ValueError:
                            await recover(item)
                            raise
                return result(None)
            except Exception:
                report()
                return result(False)
        return result(None)
    except BaseException:
        report()
        return result(False)
    finally:
        complete()
";
    for asynchronous in [false, true] {
        for raising in [false, true] {
            for (managed, iterated) in [(false, false), (true, false), (false, true), (true, true)]
            {
                let mut source = source.to_string();
                if iterated {
                    source = source.replace(
                        "                        return result(True)\n",
                        "                        async for value in values:\n                            return result(True)\n",
                    );
                }
                if managed {
                    let body = if iterated {
                        "                        async for value in values:\n                            return result(True)\n"
                    } else {
                        "                        return result(True)\n"
                    };
                    let mut replacement =
                        String::from("                        async with manager():\n");
                    for line in body.lines() {
                        writeln!(replacement, "    {line}").unwrap();
                    }
                    source = source.replace(body, &replacement);
                }
                if !asynchronous {
                    source = source
                        .replace("async def", "def")
                        .replace("async with", "with")
                        .replace("async for", "for")
                        .replace("await ", "");
                }
                if !raising {
                    source = source.replace("                            raise\n", "");
                }
                assert_matches_cpython(
                    &source,
                    &format!(
                        "nested_try_finalizer_cleanup_{asynchronous}_{raising}_{managed}_{iterated}.py"
                    ),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn try_else_manager_returns_preserve_conditional_finalizer_entry() {
    let source = r"async def sample(manager, work, pending):
    try:
        try:
            with manager() as resource:
                try:
                    value = await work()
                except Error:
                    raise
                else:
                    return value
        except Error:
            return None
    finally:
        if pending is not None:
            await pending
";
    for nested in [false, true] {
        for assigned in [false, true] {
            for coroutine in [false, true] {
                for present in [false, true] {
                    let mut source = source.to_owned();
                    if nested {
                        source = source.replace(
                            "                    value = await work()",
                            "                    async with manager():\n                        value = await work()",
                        );
                    }
                    if assigned {
                        source = source.replace(
                            "                    return value",
                            "                    resource.state = 1\n                    return value",
                        );
                    }
                    if !coroutine {
                        source = source.replace("async ", "").replace("await ", "");
                    }
                    if !present {
                        source = source.replace("pending is not None", "pending is None");
                    }
                    assert_matches_cpython(
                        &source,
                        &format!(
                            "try_else_manager_finalizer_{nested}_{assigned}_{coroutine}_{present}.py"
                        ),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn managed_handler_returns_preserve_conditional_finalizer_entry() {
    let source = r"async def sample(manager, work, cleanup, ready, value):
    try:
        try:
            with manager():
                await work()
        except Error:
            with manager():
                if await ready():
                    return value, {}, None
            raise
        return value
    except Other as error:
        raise Error() from error
    finally:
        if cleanup is not None:
            await cleanup.close()
";
    for asynchronous in [false, true] {
        for returned in ["value, {}, None", "value", "None", "False"] {
            for present in [false, true] {
                for kind in ["coroutine", "function", "generator"] {
                    if asynchronous && kind != "coroutine" {
                        continue;
                    }
                    let mut source =
                        source.replace("return value, {}, None", &format!("return {returned}"));
                    if asynchronous {
                        source = source.replace("with manager():", "async with manager():");
                    }
                    if !present {
                        source = source.replace("cleanup is not None", "cleanup is None");
                    }
                    if kind != "coroutine" {
                        source = source.replace("async ", "").replace("await ", "");
                    }
                    if kind == "generator" {
                        source = source.replacen(":\n", ":\n    yield value\n", 1);
                    }
                    assert_matches_cpython(
                        &source,
                        &format!(
                            "managed_conditional_finalizer_{asynchronous}_{returned}_{present}_{kind}.py"
                        ),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn managed_handler_returns_preserve_primitive_finalizer_owners() {
    let source = r"async def sample(manager, flag):
    with manager:
        try:
            while True:
                try:
                    await action()
                except Exception as error:
                    if flag:
                        record()
                        return
                    raise
        finally:
            if not flag:
                record()
                await finish()
";
    for (index, prefix) in [
        "record()",
        "record(); record()",
        "current = flag; record(current)",
        "pass",
    ]
    .into_iter()
    .enumerate()
    {
        for (return_index, returned) in ["", " None"].into_iter().enumerate() {
            let source = source.replace(
                "record()\n                        return",
                &format!("{prefix}\n                        return{returned}"),
            );
            assert_matches_cpython(
                &source,
                &format!("managed_handler_finalizer_{index}_{return_index}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn enclosing_tries_own_handler_returns_before_finalizers() {
    let body = r"try:
    try:
        await work()
    except Error as error:
        return value
except Other:
    recover()
finally:
    await finish()
";
    let indent = |source: &str| {
        let mut indented = String::new();
        for line in source.lines() {
            writeln!(indented, "    {line}").unwrap();
        }
        indented
    };
    let with_manager = format!("async with manager():\n{}", indent(body));
    let bodies = [
        body.to_owned(),
        with_manager.clone(),
        with_manager.replace("async with", "with"),
        format!("with manager():\n{}", indent(&with_manager)),
        body.replace(
            "        return value",
            "        async with manager():\n            return value",
        ),
        format!("try:\n{}finally:\n    finish()\n", indent(&with_manager)),
        format!("while value:\n{}", indent(body)),
        format!("while value:\n{}", indent(&with_manager)),
        body.replace(
            "        return value",
            "        try:\n            return value\n        except Third:\n            recover()",
        ),
    ];
    for (index, body) in bodies.iter().enumerate() {
        for named in [false, true] {
            for returned in ["value", "None"] {
                for kind in ["coroutine", "function", "generator"] {
                    let mut source = format!(
                        "async def sample(manager, work, finish, value):\n{}",
                        indent(body)
                    );
                    if !named {
                        source = source.replace("except Error as error:", "except Error:");
                    }
                    if kind != "coroutine" {
                        source = source.replace("async ", "").replace("await ", "");
                    }
                    if kind == "generator" {
                        source = source.replacen(":\n", ":\n    yield value\n", 1);
                    }
                    source = source.replace("return value", &format!("return {returned}"));
                    assert_matches_cpython(
                        &source,
                        &format!("handler_try_owner_{index}_{named}_{returned}_{kind}.py"),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn guarded_recovery_preserves_populated_normal_cleanup_entries() {
    fn indent(source: &str) -> String {
        source.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    }
    for (work_name, work) in [
        ("linear", "perform(value)\n"),
        (
            "guarded",
            "if value is not None:\n    try:\n        perform(value)\n    finally:\n        finish(value)\n",
        ),
    ] {
        for (entry_name, entry) in [("boolean", "not (flag and done)"), ("local", "flag")] {
            let work = indent(work);
            let cleanup = format!(
                "done = False\ntry:\n{work}    done = True\nexcept RuntimeError as error:\n    report(error)\n    if flag:\n        raise\nfinally:\n    quiet = {entry}\n    try:\n        with manager() if quiet else alternate():\n            close(value)\n    finally:\n        if flag:\n            with manager() if quiet else alternate():\n                finish(value)\n"
            );
            for (name, prefix) in [
                ("flat", None),
                ("normal", Some("perform(value)")),
                ("generator", Some("yield value")),
            ] {
                let body = if let Some(prefix) = prefix {
                    let nested = indent(&cleanup);
                    format!("try:\n    {prefix}\nfinally:\n{nested}")
                } else {
                    cleanup.clone()
                };
                let body = indent(&body);
                let source = format!("def sample(value, flag):\n{body}");
                assert_matches_cpython(
                    &source,
                    &format!("populated_recovery_cleanup_{work_name}_{entry_name}_{name}.py"),
                    &LEVELS,
                );
            }
            if entry_name == "boolean" {
                let controls = if work_name == "linear" {
                    vec![
                        (
                            "exceptional",
                            format!(
                                "try:\n    raise LookupError\nfinally:\n{}",
                                indent(&cleanup)
                            ),
                        ),
                        (
                            "handler",
                            format!(
                                "try:\n    perform(value)\nexcept LookupError:\n{}",
                                indent(&cleanup)
                            ),
                        ),
                    ]
                } else {
                    let tail = cleanup.replace("    done = True\n", "");
                    vec![(
                        "structured_tail",
                        format!("try:\n    yield value\nfinally:\n{}", indent(&tail)),
                    )]
                };
                for (name, body) in controls {
                    let source = format!("def sample(value, flag):\n{}", indent(&body));
                    assert_matches_cpython(
                        &source,
                        &format!("populated_recovery_boundary_{name}.py"),
                        &LEVELS,
                    );
                }
            }
        }
    }
}

#[test]
fn named_handler_returns_preserve_outer_finalizer_owners() {
    let source = r"def named_nested(choice, item, owner, operation, cleanup, recover):
    value = None
    try:
        try:
            try:
                value = operation(value)
            except ValueError as error:
                value = recover(value)
                if choice is None:
                    cleanup(value)
                    raise
                try:
                    value = operation(value)
                except TypeError:
                    cleanup(value)
                    return {'result': item}
        finally:
            for entry in (owner.left, owner.middle, owner.right):
                if entry is not None:
                    cleanup(entry)
    except ValueError:
        return None
    except TypeError:
        return item
    return value
";
    let without_outer = r"def no_outer_try(choice, item, owner, operation, cleanup, recover):
    value = None
    try:
        try:
            value = operation(value)
        except ValueError as error:
            value = recover(value)
            if choice is None:
                cleanup(value)
                raise
            try:
                value = operation(value)
            except TypeError:
                cleanup(value)
                return {'result': item}
    finally:
        for entry in (owner.left, owner.middle, owner.right):
            if entry is not None:
                cleanup(entry)
    return value
";
    for (index, source) in [
        source.to_owned(),
        source.replace("named_nested(", "unnamed_outer(")
            .replace("except ValueError as error:", "except ValueError:"),
        source.replace("named_nested(", "no_conditional_raise(")
            .replace("                if choice is None:\n                    cleanup(value)\n                    raise\n", ""),
        source.replace("named_nested(", "single_handler(")
            .replace("                try:\n                    value = operation(value)\n                except TypeError:\n                    cleanup(value)\n                    return {'result': item}",
                     "                cleanup(value)\n                return {'result': item}"),
        source.replace("named_nested(", "constant_return(")
            .replace("return {'result': item}", "return None"),
        without_outer.to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("named_nested_return_owner_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn following_name_return_with_copied_cleanup() {
    let source = r"def plain_call(value, fallback, flag, work, cleanup, owner):
    try:
        try:
            if flag:
                work()
        except Exception:
            return fallback
        return value
    finally:
        cleanup()
";
    let attribute = source
        .replace("plain_call", "attribute_store")
        .replace("cleanup()", "owner.attr = flag");
    for (index, source) in [
        source.to_owned(),
        source
            .replace("plain_call", "conditional_call")
            .replace("cleanup()", "cleanup(1 if flag else 2)"),
        source
            .replace("plain_call", "short_circuit_call")
            .replace("cleanup()", "cleanup(flag and value)"),
        attribute.clone(),
        attribute.replace("attribute_store", "linear_body").replace(
            "            if flag:\n                work()",
            "            work()",
        ),
        attribute
            .replace("attribute_store", "fallthrough_handler")
            .replace("return fallback", "work()"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("following_name_return_with_copied_cleanup_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_handler_iterator_returns_end_inner_try_ownership() {
    let source = r"def sample(items, first, second, choose):
    try:
        return first()
    except Error as error:
        try:
            return second()
        except Error:
            for item in items:
                try:
                    return choose(item)
                except Error:
                    continue
            return None
";
    for source in [
        source.to_owned(),
        source.replace("except Error as error:", "except Error:"),
        source.replacen("except Error:\n", "except Error as inner:\n", 1),
        source.replace("return choose(item)", "return item"),
        source.replace("return choose(item)", "return"),
        source.replace("def sample", "async def sample"),
        source.replacen("    try:\n", "    yield None\n    try:\n", 1),
    ] {
        assert_matches_cpython(&source, "nested_handler_iterator_return.py", &LEVELS);
    }
    for source in [
        r"def sample(items, choose):
    for item in items:
        try:
            return choose(item)
        except Error:
            continue
    return None
",
        r"def sample(items, first, choose):
    try:
        return first()
    except Error as error:
        for item in items:
            try:
                return choose(item)
            except Error:
                continue
        return None
",
    ] {
        assert_matches_cpython(source, "handler_iterator_return_control.py", &LEVELS);
    }
}

#[test]
fn nested_handler_returns_keep_enclosing_try_owner_inside_while() {
    let source = r"def sample(flag):
    try:
        while flag:
            try:
                work()
            except ValueError:
                try:
                    recover()
                except OSError:
                    return False
                continue
            finish()
        raise RuntimeError
    except Exception:
        cleanup()
        raise
";
    for source in [
        source.to_owned(),
        source
            .replace("def sample", "async def sample")
            .replace("work()", "await work()")
            .replace("recover()", "await recover()"),
    ] {
        for (index, source) in [
            source.clone(),
            source.replace("return False", "return flag"),
            source.replace("return False", "return"),
            source.replace("except OSError:", "except OSError as error:"),
        ]
        .into_iter()
        .enumerate()
        {
            assert_matches_cpython(
                &source,
                &format!("nested_handler_while_{index}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn enclosing_tries_own_copied_finalizers_after_handler_returns() {
    let source = r"def sample(first, second):
    try:
        prepare()
        try:
            check(first)
            try:
                check(second)
            except ValueError:
                return True
            else:
                finish(second)
                return False
            finally:
                finish(first)
        finally:
            release(second)
            release(first)
    except OSError:
        return False
";
    for asynchronous in [false, true] {
        for named in [false, true] {
            for returned in ["True", "first"] {
                let mut source = source.replace("return True", &format!("return {returned}"));
                if asynchronous {
                    source = source.replace("def sample", "async def sample");
                }
                if named {
                    source = source.replace("except ValueError:", "except ValueError as error:");
                }
                assert_matches_cpython(
                    &source,
                    &format!("enclosing_try_finalizers_{asynchronous}_{named}_{returned}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn completed_recovery_finalizers_retire_iterator_return_owners() {
    let source = r"async def sample(items, flag):
    for item in items:
        try:
            token = await acquire(item)
            try:
                try:
                    value = await work(token)
                except LookupError:
                    raise
                except ValueError as error:
                    value = error.value
                    if flag and retry(value):
                        raise RuntimeError() from error
                if flag and value:
                    raise RuntimeError()
                return result(value if flag else None, token=token)
            finally:
                try:
                    await finish(token)
                except Exception:
                    recover(token)
        except Exception as error:
            if flag or not retry(error):
                raise
            await delay()
    raise AssertionError()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def").replace("await ", ""),
        source.replace("recover(token)", "pass"),
        source.replace("await finish(token)", "finish(token)"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("completed_recovery_iterator_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_handler_returns_preserve_outer_finalizer_owners() {
    let source = r"def nested_handlers(choice, item, owner, operation, cleanup, recover):
    value = None
    try:
        try:
            try:
                value = operation(value)
            except TypeError:
                try:
                    value = recover(value)
                except ValueError:
                    return {'result': item}
        finally:
            for entry in (owner.left, owner.middle, owner.right):
                if entry is not None:
                    cleanup(entry)
    except ValueError:
        return None
    except TypeError:
        return item
    return value
";
    let without_outer = r"def nested_handlers(choice, item, owner, operation, cleanup, recover):
    value = None
    try:
        try:
            value = operation(value)
        except TypeError:
            try:
                value = recover(value)
            except ValueError:
                return {'result': item}
    finally:
        for entry in (owner.left, owner.middle, owner.right):
            if entry is not None:
                cleanup(entry)
    return value
";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "            try:\n                value = operation(value)\n            except TypeError:\n                try:\n                    value = recover(value)\n                except ValueError:\n                    return {'result': item}",
            "            try:\n                value = recover(value)\n            except ValueError:\n                return {'result': item}",
        ),
        without_outer.to_owned(),
        source.replace(
            "            for entry in (owner.left, owner.middle, owner.right):\n                if entry is not None:\n                    cleanup(entry)",
            "            cleanup(owner.right)",
        ),
        source.replace("return {'result': item}", "return None"),
        source.replace(
            "                except ValueError:\n                    return",
            "                except ValueError as error:\n                    return",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("nested_handler_return_owner_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_erased_assertion_tails_keep_original_loop_backedges() {
    for (index, source) in [
        r"from __future__ import annotations

async def nested_await(values, first, second, prepare, observe, ready):
    for item in values:
        pending = prepare(item)
        result = await pending
        assert result.value
        current = result.value
        observe(current)
        if first:
            observe(current)
            if second:
                selected = values[0]
                if selected.ready:
                    pending = prepare(selected)
                    await pending
                assert ready(current)
    return tuple(values)
",
        r"from __future__ import annotations

def sync_nested(values, first, second, prepare, observe, ready):
    for item in values:
        pending = prepare(item)
        result = pending
        assert result.value
        current = result.value
        observe(current)
        if first:
            observe(current)
            if second:
                selected = values[0]
                if selected.ready:
                    pending = prepare(selected)
                    pending
                assert ready(current)
    return tuple(values)
",
        r"from __future__ import annotations

async def no_suspension(values, first, second, prepare, observe, ready):
    for item in values:
        pending = prepare(item)
        result = pending
        assert result.value
        current = result.value
        observe(current)
        if first:
            observe(current)
            if second:
                selected = values[0]
                if selected.ready:
                    pending = prepare(selected)
                    pending
                assert ready(current)
    return tuple(values)
",
        r"from __future__ import annotations

async def no_trailing_assert(values, first, second, prepare, observe, ready):
    for item in values:
        pending = prepare(item)
        result = await pending
        assert result.value
        current = result.value
        observe(current)
        if first:
            observe(current)
            if second:
                selected = values[0]
                if selected.ready:
                    pending = prepare(selected)
                    await pending
    return tuple(values)
",
        r"from __future__ import annotations

async def unconditional(values, first, second, prepare, observe, ready):
    for item in values:
        pending = prepare(item)
        result = await pending
        assert result.value
        current = result.value
        observe(current)
        observe(current)
        selected = values[0]
        if selected.ready:
            pending = prepare(selected)
            await pending
        assert ready(current)
    return tuple(values)
",
        r"from __future__ import annotations

async def no_inner_suspension(values, first, second, prepare, observe, ready):
    for item in values:
        pending = prepare(item)
        result = await pending
        assert result.value
        current = result.value
        observe(current)
        if first:
            observe(current)
            if second:
                selected = values[0]
                if selected.ready:
                    pending = prepare(selected)
                    pending
                assert ready(current)
    return tuple(values)
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("nested_erased_loop_backedge_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn handler_break_releases_iterator_exception_owner() {
    let source = r"async def nested_managers(items):
    for item in items:
        try:
            if prepare_needed(item):
                try:
                    async with acquire(item) as token:
                        await prepare(token)
                except LookupError:
                    break
            async with outer(item):
                async with inner(item):
                    if accepted(item):
                        return item
                    if stopped(item):
                        break
        except Exception:
            continue
    return None
";
    for (index, source) in [
        source.to_owned(),
        source
            .replace("async def", "def")
            .replace("async with", "with")
            .replace("await ", ""),
        source.replace(
            "            async with outer(item):\n                async with inner(item):\n                    if accepted(item):\n                        return item\n                    if stopped(item):\n                        break\n",
            "            async with outer(item):\n                if accepted(item):\n                    return item\n                if stopped(item):\n                    break\n",
        ),
        source.replace("                    if accepted(item):\n                        return item\n", ""),
        source.replace(
            "                try:\n                    async with acquire(item) as token:\n                        await prepare(token)\n                except LookupError:\n                    break\n",
            "                async with acquire(item) as token:\n                    await prepare(token)\n",
        ),
        source.replace("                    if stopped(item):\n                        break\n", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &format!("from __future__ import annotations\n{source}"),
            &format!("handler_break_iterator_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn copied_finally_compound_break_threads_both_conditions() {
    let source = r#"async def copied_break(self, key):
    while not self.finished():
        try:
            with acquire(self.path / "lease") as resource:
                if self.finished():
                    break
        finally:
            if key not in self.active and pending is not None:
                pass
        await clock.pause(0.25)
    report("done", "loop")
    raise Halt("complete")
"#;
    let mut wrapped = String::from("class Runner:\n");
    for line in source.lines() {
        writeln!(wrapped, "    {line}").unwrap();
    }
    for source in [
        source.to_owned(),
        source.replace("async def", "def").replace("await clock", "clock"),
        wrapped,
        source.replace(
            "            with acquire(self.path / \"lease\") as resource:\n                if self.finished():\n                    break",
            "            if self.finished():\n                break",
        ),
        source.replace("key not in self.active and pending is not None", "key not in self.active"),
        source.replace("                    break", "                    pass"),
    ] {
        assert_matches_cpython(
            &format!("from __future__ import annotations\n{source}"),
            "copied_finally_break.py",
            &LEVELS,
        );
    }
}

#[test]
fn handler_raising_while_else_preserves_break_cleanup() {
    let source = r"async def guarded_finalizer(owner, enabled, marker):
    try:
        await perform(owner)
    except ValueError:
        raise
    except RuntimeError as failure:
        note(owner)
        while ready(owner):
            if selected(owner):
                marker = 41
                break
            advance(owner)
        else:
            raise
    finally:
        if enabled and marker != 17:
            try:
                await owner.finish(marker)
            except OSError:
                note(29)
";
    for source in [
        source.to_owned(),
        source.replace("        else:\n            raise\n", ""),
        source.replace("except RuntimeError as failure:", "except RuntimeError:"),
        source.replace("async def", "def").replace("await ", ""),
        source.replace("        while ready(owner):\n            if selected(owner):\n                marker = 41\n                break\n            advance(owner)\n        else:\n            raise\n",
            "        if selected(owner):\n            marker = 41\n        else:\n            raise\n"),
        source.replace("        if enabled and marker != 17:\n            try:\n                await owner.finish(marker)\n            except OSError:\n                note(29)\n",
            "        try:\n            await owner.finish(marker)\n        except OSError:\n            note(29)\n"),
    ] {
        assert_matches_cpython(&source, "handler_raising_while_else.py", &LEVELS);
    }
}

#[test]
fn nested_mapping_captures_preserve_multiline_cleanup_boundaries() {
    let body = r#"match value:
    case {
        "state": "ready",
        "box": {"kind": "single", "items": [{"value": item}]},
    }GUARD:
        return item
"#;
    let guard = " if guard.get(\"warnings\", []) == []";
    for coroutine in [false, true] {
        for (protected, guard) in [(false, ""), (false, guard), (true, guard)] {
            let prefix = if coroutine { "async " } else { "" };
            let mut source = format!("{prefix}def probe(value, guard):\n");
            let indent = if protected { "        " } else { "    " };
            if protected {
                source.push_str("    try:\n");
            }
            for line in body.replace("GUARD", guard).lines() {
                writeln!(source, "{indent}{line}").unwrap();
            }
            if protected {
                source.push_str("    except RuntimeError as error:\n        report(error)\n");
            }
            source.push_str("    return None\n");
            assert_matches_cpython(&source, "multiline_capture_boundary.py", &LEVELS);
        }
    }
}

#[test]
fn single_mapping_captures_preserve_other_local_guards() {
    let body = r#"match value:
    case {"box": {"items": [{"value": item}]}} if GUARD is not None:
        return item
"#;
    for coroutine in [false, true] {
        for protected in [false, true] {
            for guard in ["item", "guard"] {
                let prefix = if coroutine { "async " } else { "" };
                let mut source = format!("{prefix}def probe(value, guard):\n");
                let indent = if protected { "        " } else { "    " };
                if protected {
                    source.push_str("    try:\n");
                }
                for line in body.replace("GUARD", guard).lines() {
                    writeln!(source, "{indent}{line}").unwrap();
                }
                if protected {
                    source.push_str("    except RuntimeError as error:\n        report(error)\n");
                }
                source.push_str("    return None\n");
                assert_matches_cpython(
                    &source,
                    &format!("mapping_local_guard_{coroutine}_{protected}_{guard}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn starred_mapping_capture_cleanups_apply_static_swaps() {
    let source = r#"def probe(value):
    match value:
        case {"box": {"items": [*items]} as record}:
            return items is not None and record is not None
    return False
"#;
    for scalar in [false, true] {
        for guarded in [false, true] {
            let mut source = source.to_owned();
            if scalar {
                source = source.replace("\"items\":", "\"kind\": kind, \"items\":");
            }
            if guarded {
                source = source.replace(
                    " as record}:",
                    " as record} if (\n            len(items) > 0\n        ):",
                );
            }
            assert_matches_cpython(
                &source,
                &format!("starred_mapping_capture_{scalar}_{guarded}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn multiline_mapping_capture_cleanups_apply_static_swaps() {
    for guard in ["", " if record.get(\"extra\") is None"] {
        let source = format!(
            r#"def sample(data, mode):
    match data:
        case {{
            "kind": "item",
            "payload": {{"active": True, "items": [{{"kind": "text", "value": str(value)}}]}} as record,
        }}{guard}:
            return value == "absent" or (mode == "read" and value == "unknown")
    return False
"#
        );
        assert_matches_cpython(&source, "multiline_mapping_capture_cleanup.py", &LEVELS);
    }
}

#[test]
fn composed_mapping_capture_cleanups_apply_static_swaps() {
    for pattern in [
        "{\"number\": int(number), \"value\": str(value)} as record",
        "{\"payload\": {\"number\": int(number), \"value\": str(value)} as record}",
    ] {
        let source = format!(
            "def sample(data):\n    match data:\n        case {pattern}:\n            return accept(number, value, record)\n    return False\n"
        );
        assert_matches_cpython(&source, "mapping_capture_cleanup.py", &LEVELS);
    }
}

#[test]
fn unnamed_handler_match_cleanup_preserves_success_edges() {
    let source = r"async def handler_match(owner, kinds, operation, sink, choice):
    result = None
    try:
        result = operation()
        sink(result)
    except ArithmeticError:
        return result
    except LookupError:
        sink(choice)
        match choice:
            case kinds.first:
                sink(choice)
                return result
            case kinds.second:
                sink(choice)
                sink(result)
                result = operation()
            case _ as captured:
                sink(captured)
    except RuntimeError:
        result = operation()
    except Exception:
        raise
    if owner.value is None:
        return result
    return operation()
";
    let unprotected = r"async def unprotected_match(owner, kinds, operation, sink, choice):
    result = None
    result = operation()
    sink(result)
    sink(choice)
    match choice:
        case kinds.first:
            sink(choice)
            return result
        case kinds.second:
            sink(choice)
            sink(result)
            result = operation()
        case _ as captured:
            sink(captured)
    if owner.value is None:
        return result
    return operation()
";
    let conditional = source.replace("handler_match", "conditional_dispatch").replace(
        "        match choice:\n            case kinds.first:\n                sink(choice)\n                return result\n            case kinds.second:\n                sink(choice)\n                sink(result)\n                result = operation()\n            case _ as captured:\n                sink(captured)",
        "        if choice == kinds.first:\n            sink(choice)\n            return result\n        elif choice == kinds.second:\n            sink(choice)\n            sink(result)\n            result = operation()\n        else:\n            captured = choice\n            sink(captured)",
    );
    for source in [
        source.to_owned(),
        source.replace("async def handler_match", "def synchronous_match"),
        unprotected.to_owned(),
        source
            .replace("handler_match", "wildcard_match")
            .replace("case _ as captured:", "case _:")
            .replace("sink(captured)", "sink(choice)"),
        source
            .replace("handler_match", "returning_middle_match")
            .replace(
                "                result = operation()",
                "                return operation()",
            ),
        conditional,
    ] {
        assert_matches_cpython(&source, "unnamed_handler_match.py", &LEVELS);
    }
}

#[test]
fn protected_conditional_tuples_fuse_locals_with_scope_fallbacks() {
    for (prefix, suspend) in [("", ""), ("async ", ""), ("", "    yield value\n")] {
        for handler in ["RuntimeError", "RuntimeError as error"] {
            let source = format!(
                "{prefix}def sample(flag, value, path, state):
{suspend}    try:
        if flag:
            return value, path if value is not None else None, state
        return None
    except {handler}:
        return None
"
            );
            assert_matches_cpython(&source, "protected_conditional_tuple.py", &LEVELS);
        }
    }
}

#[test]
fn unpacked_or_captures_apply_static_cleanup_swaps() {
    let nested = r#"{"kind": "outer", "value": {"kind": "leaf", "text": str(content)}}"#;
    let flat = r#"{"kind": "leaf", "text": str(content)}"#;
    for (name, pattern) in [
        ("nested_first", format!("{nested} | {flat}")),
        ("flat_first", format!("{flat} | {nested}")),
        (
            "multiline",
            format!(
                "{nested} | {{\n            \"kind\": \"leaf\",\n            \"text\": str(content),\n        }}"
            ),
        ),
        ("single_alternative", nested.to_owned()),
    ] {
        let source = format!(
            "def sample(value):\n    match value:\n        case {pattern}:\n            return content\n        case _:\n            return None\n"
        );
        assert_matches_cpython(&source, &format!("unpacked_or_capture_{name}.py"), &LEVELS);
    }
}

#[test]
fn sequence_capture_wildcard_cleanup_matrix() {
    let source = r#"
def run(value):
    match value:
        case Packet(
            kind=Kind(tag="entry"),
            values=[first, _, _, _, last],
            options=[],
        ):
            return first, last
    return None
"#;
    let nested = format!(
        "def outer():\n{}\n    return run\n",
        source.lines().fold(String::new(), |mut output, line| {
            writeln!(output, "    {line}").unwrap();
            output
        })
    );
    for source in [
        source.to_owned(),
        source.replace("_, last]", "_, _]").replace("return first, last", "return first"),
        source.replace(
            "Packet(\n            kind=Kind(tag=\"entry\"),\n            values=[first, _, _, _, last],\n            options=[],\n        )",
            "Packet(kind=Kind(tag=\"entry\"), values=[first, _, _, _, last], options=[])",
        ),
        source.replace("            kind=Kind", "            options=[],\n            kind=Kind").replace("            options=[],\n        )", "        )"),
        source.replace("[first, _, _, _, last]", "[first, _, last]"),
        nested,
    ] {
        assert_matches_cpython(&source, "sequence_capture_cleanup.py", &LEVELS);
    }
}

#[test]
fn retired_normal_try_exits_preserve_borrow_boundaries() {
    let source = r"
def run(active, take, consume, entries):
    result = 0
    try:
        consume(active)
    finally:
        result = 1
        try:
            while True:
                if active:
                    result = 2
                    break
                result = take()
                if result:
                    break
        except LookupError as error:
            result = 3
        consume(result)
    if result:
        return 1
    return 0
";
    let unnamed = source.replace("except LookupError as error:", "except LookupError:");
    for source in [
        source.to_owned(),
        unnamed.clone(),
        unnamed.replace("        except LookupError:", "                selected = [entry for entry in entries]\n                consume(selected)\n        except LookupError:"),
        unnamed.replace("        except LookupError:", "                for left, right in entries:\n                    consume(left)\n        except LookupError:"),
        source.replace("        except LookupError as error:", "                selected = [entry for entry in entries]\n                for left, right in entries:\n                    consume(right)\n                selected = take() if active else result\n                if active and selected:\n                    consume(selected)\n        except LookupError as error:"),
        unnamed.replace("while True:", "while active:").replace("        except LookupError:", "                consume(result)\n        except LookupError:"),
    ] {
        assert_matches_cpython(&source, "retired_normal_try_exit.py", &LEVELS);
    }
    for source in [
        r"
def run(flag, take, note):
    result = 0
    try:
        note(flag)
    finally:
        result = 1
        while True:
            if flag:
                result = 2
                break
            result = take()
            if result:
                break
        note(result)
    if result:
        return 1
    return 0
",
        r"
def run(flag, take, note):
    result = 0
    try:
        note(flag)
    finally:
        result = 1
        turn = 0
        try:
            result = take()
        except LookupError:
            result = 3
        note(result)
        note(turn)
    if result:
        return 1
    return 0
",
    ] {
        assert_matches_cpython(source, "retired_normal_try_exit_control.py", &LEVELS);
    }
}

#[test]
fn copied_nested_recovery_preserves_borrow_boundary() {
    let source = r"
def run(flag, load, note):
    try:
        load()
    except LookupError:
        try:
            note()
        except TypeError:
            try:
                note()
            except ValueError:
                pass
        note()
    else:
        note()
    if not flag:
        return []
    note(flag)
    return flag
";
    for source in [
        source.to_owned(),
        source.replace("    else:\n        note()\n", ""),
        source.replace("        note()\n    else:", "        raise\n    else:"),
        source.replace("            try:\n                note()\n            except ValueError:\n                pass\n", "            pass\n"),
        source.replace("                pass\n        note()", "                pass\n            note()\n        note()"),
        source.replace("except TypeError:", "except TypeError as error:"),
        source.replace("def run", "async def run").replace("        load()", "        await load()"),
        source.replace("except LookupError:", "except LookupError as error:"),
    ] {
        assert_matches_cpython(&source, "nested_recovery_borrow.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_inner_arm_preserves_loop_backedges() {
    let source = r"
async def run(items, mode, note):
    await note(mode)
    for item in items:
        marker = note(item)
        if mode == 1:
            assert marker
        else:
            assert not marker
        if mode == 2:
            if marker:
                assert item
            assert marker
        else:
            assert not marker
    if mode:
        assert items
    value = note(mode)
    return value
";
    let (prefix, loop_and_tail) = source.split_once("    for item in items:\n").unwrap();
    let (loop_body, tail) = loop_and_tail.split_once("    if mode:\n").unwrap();
    let mut nested_body = String::new();
    for line in loop_body.lines() {
        writeln!(nested_body, "    {line}").unwrap();
    }
    let nested_loop = format!(
        "{prefix}    for batch in items:\n        for item in batch:\n{nested_body}    if mode:\n{tail}"
    );
    let protected_loop = format!(
        "{}    with manager:\n        for item in items:\n{nested_body}    if mode:\n{tail}",
        prefix.replace("items, mode, note", "items, mode, note, manager")
    );
    for (index, source) in [
        source.to_owned(),
        source.replace("    await note(mode)\n", ""),
        source.replace("        if mode == 1:\n            assert marker\n        else:\n            assert not marker\n", ""),
        source.replace("                assert item\n            assert marker\n", "                assert item\n"),
        source.replace("async def", "def").replace("    await note(mode)\n", ""),
        source.replace("if marker:", "if mode != 3:"),
        source.replace("    if mode:\n", "    else:\n        note(mode)\n    if mode:\n"),
        nested_loop,
        protected_loop,
    ].into_iter().enumerate() {
        assert_matches_cpython(&source, &format!("erased_coroutine_arm_{index}.py"), &LEVELS);
    }
}

#[test]
fn protected_erased_else_preserves_completed_iterator_backedges() {
    let simple = r"def collect(items, selected, sink, cleanup):
    try:
        for item in items:
            if item not in selected:
                sink(item)
            else:
                assert item
    finally:
        cleanup()
";
    let compound = simple.replace(
        "item not in selected:",
        "item not in selected and selected:",
    );
    let comprehension = simple.replace(
        "                sink(item)",
        "                snapshot = {value: value for value in items}\n                sink(snapshot)",
    );
    let compound_comprehension = comprehension.replace(
        "item not in selected:",
        "item not in selected and selected:",
    );
    let nested = r"def collect(batches, selected, sink, cleanup):
    try:
        for batch in batches:
            for key, value in batch:
                if key not in selected and value in selected:
                    snapshot = {slot: data for slot, data in batch}
                    assert snapshot
                    selected[key] = value
                    sink(snapshot)
                else:
                    assert key != value
    finally:
        cleanup()
";
    let unprotected = nested
        .replace(
            "batches, selected, sink, cleanup",
            "batches, selected, sink",
        )
        .replace("    try:\n", "")
        .replace("    finally:\n        cleanup()\n", "")
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for (index, source) in [
        simple.to_owned(),
        compound,
        comprehension.clone(),
        comprehension
            .replace(
                "items, selected, sink, cleanup",
                "items, pairs, selected, sink, cleanup",
            )
            .replace(
                "{value: value for value in items}",
                "{key: value for key, value in pairs}",
            ),
        compound_comprehension.clone(),
        compound_comprehension.replace("assert item", "sink(item)"),
        nested.to_owned(),
        nested.replace("assert key != value", "sink(key)"),
        unprotected,
        nested.replace(" and value in selected", ""),
        nested.replace("{slot: data for slot, data in batch}", "selected.copy()"),
        nested
            .replace("def collect(batches,", "def collect(batch,")
            .replace("        for batch in batches:\n", "")
            .lines()
            .map(|line| {
                line.strip_prefix("    ")
                    .filter(|_| line.starts_with("            "))
                    .unwrap_or(line)
            })
            .collect::<Vec<_>>()
            .join("\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_erased_iterator_backedge_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn managed_name_await_preserves_erased_else_backedge() {
    let source = r"
async def run(values, transform, manager, pending, cleanup):
    try:
        for value in values:
            item = transform(value)
            if item == 0:
                with manager(item, enabled=True):
                    await pending
            else:
                assert item
    finally:
        cleanup()
        cleanup()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("await pending", "await transform(item)"),
        source.replace("await pending", "transform(item)"),
        source.replace(
            "                with manager(item, enabled=True):\n                    await pending",
            "                await pending",
        ),
        source.replace("with manager(", "with manager.scope("),
        source.replace(
            "await pending",
            "await pending\n                assert item",
        ),
        source.replace(
            "                with manager(item, enabled=True):",
            "                assert item\n                with manager(item, enabled=True):",
        ),
        source.replace(
            "                assert item",
            "                transform(item)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("managed_name_await_{index}.py"), &LEVELS);
    }
}

#[test]
fn completed_manager_work_preserves_retained_loop_owners() {
    let source = r"async def sample(items, resource, work, cleanup, flag):
    with resource():
        try:
            for item in items:
                if flag:
                    with resource() as handle:
                        await work(item)
                    assert handle
                else:
                    assert item
        finally:
            cleanup()
";
    let nested = r"async def sample(items, resource, work, cleanup, flag):
    try:
        with resource():
            for batch in items:
                try:
                    for item in batch:
                        if flag:
                            with resource() as handle:
                                await work(item)
                            assert handle
                        else:
                            assert item
                finally:
                    cleanup()
    finally:
        cleanup()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("                    assert handle\n", ""),
        source.replace(
            "                else:\n                    assert item",
            "                else:\n                    work(item)",
        ),
        nested.to_owned(),
        nested.replace("                            assert handle\n", ""),
        nested.replace(
            "                            assert handle",
            "                            assert handle\n                            assert item",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("retained_manager_loop_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn protected_conditional_store_preserves_erased_else_backedge() {
    let protected = r"def collect(batches, left, right, cleanup, sink):
    try:
        for batch in batches:
            for key in batch:
                if key in (2, 5):
                    chosen = left if key == 2 else right
                    assert chosen
                    assert chosen != key
                    assert key
                else:
                    assert key
                    assert batch
                    assert batches
    finally:
        cleanup()
        sink(batches)
";
    let constants = protected.replace("left if key == 2 else right", "7 if key == 2 else 11");
    let unprotected = protected
        .replace("    try:\n", "")
        .replace(
            "    finally:\n        cleanup()\n        sink(batches)\n",
            "",
        )
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for (index, source) in [
        constants.clone(),
        constants.replace("if key in (2, 5):", "if key:"),
        protected.to_owned(),
        protected.replace("left if key == 2 else right", "key"),
        unprotected,
        protected.replace(
            "                else:\n                    assert key\n                    assert batch\n                    assert batches",
            "                else:\n                    sink(key)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_conditional_store_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn protected_completed_inner_for_preserves_erased_else_backedge() {
    let protected = r"def collect(groups, cleanup, sink, fallback):
    try:
        for group in groups:
            for record in group:
                if record.enabled:
                    assert record.entries
                    for flag, value in record.entries:
                        chosen = value if flag else fallback
                        assert chosen
                else:
                    assert record.enabled
            assert group
    finally:
        cleanup()
        sink(groups)
";
    let unprotected = protected
        .replace("    try:\n", "")
        .replace(
            "    finally:\n        cleanup()\n        sink(groups)\n",
            "",
        )
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for (index, source) in [
        protected.replace("value if flag else fallback", "value"),
        protected.to_owned(),
        unprotected.replace("value if flag else fallback", "value"),
        unprotected,
        protected.replace(
            "                else:\n                    assert record.enabled",
            "                else:\n                    sink(record)",
        ),
        protected.replace(
            "                if record.enabled:",
            "                for flag, value in record.entries:\n                    assert flag == value\n                if record.enabled:",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_completed_inner_for_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_nested_arms_keep_the_enclosing_iterator_exit() {
    let source = r"def process(items, flags, kinds, width):
    with manager() as owner, manager(), manager(), manager(), manager(), manager():
        for entry in items:
            if flags & kinds.first:
                assert entry.left is not None
                assert entry.width == width
                if len(entry.left.shape) > 3:
                    assert entry.left.shape[1] == width
                    assert entry.right.shape[1] == width
            else:
                assert entry.left is None
            assert entry.valid
            if flags & kinds.second:
                assert entry.left is not None
                assert entry.width == width
                if len(entry.right.shape) > 3:
                    assert entry.left.shape[1] == width
                    assert entry.right.shape[1] == width
            else:
                assert entry.right is None
        result = transform(items)
";
    for source in [
        source.to_owned(),
        source.replace(
            "with manager() as owner, manager(), manager(), manager(), manager(), manager():",
            "with manager():",
        ),
    ] {
        assert_matches_cpython(&source, "erased_nested_iterator_arm.py", &LEVELS);
    }
}

#[test]
fn nested_erased_joins_inherit_conditional_successors() {
    let source = r"def process(items, count, flags, kinds, enabled, limit):
    with manager() as owner, manager():
        for index, entry in enumerate(items):
            if count > 0 and flags & kinds.first and enabled:
                if index == limit:
                    assert entry.left is not None
                    assert entry.width == count
                else:
                    assert entry.left is None
            assert entry.valid
            if count > 0 and flags & kinds.second:
                assert entry.right is not None
            else:
                assert entry.right is None
";
    for source in [
        source.to_owned(),
        source.replace("with manager() as owner, manager():", "with manager():"),
        source.replace(
            "for index, entry in enumerate(items):",
            "while ready(items):\n            index, entry = next(items)",
        ),
    ] {
        assert_matches_cpython(&source, "nested_conditional_successor.py", &LEVELS);
    }
}

#[test]
fn nested_erased_loop_joins_keep_the_manager_owner() {
    let source = r"def process(items, count, flags, kinds, enabled):
    with manager() as first, manager() as second, manager():
        for entry in items:
            if count > 0 and flags & kinds.first:
                assert entry.left is not None
                if enabled:
                    assert entry.scale == 128 / count
                else:
                    assert entry.scale == 1
            else:
                assert entry.left is None
            assert entry.valid
            if count > 0 and flags & kinds.second:
                assert entry.right is not None
            else:
                assert entry.right is None
";
    for source in [
        source.to_owned(),
        source.replace(
            "with manager() as first, manager() as second, manager():",
            "with manager():",
        ),
        source.replace(
            "for entry in items:",
            "while ready(items):\n            entry = next(items)",
        ),
    ] {
        assert_matches_cpython(&source, "nested_erased_loop_join.py", &LEVELS);
    }
}

#[test]
fn async_manager_preserves_conditional_join_before_outer_finalizer() {
    let source = r"async def f(manager, stream, enabled, expected):
    with manager:
        try:
            async with stream:
                result = await receive()
                if enabled and transform(result).value == expected:
                    assert result.valid
                else:
                    assert not result.valid
                data = (await receive()).read()
                consume(data)
                for item in data:
                    consume(item)
        finally:
            try:
                if stream is not None:
                    await cleanup(stream)
            finally:
                await finish()
";
    for source in [
        source.to_owned(),
        source.replace("    with manager:", "    async with manager:"),
        source.replace("            async with stream:", "            async with stream, other:"),
        source.replace(
            "        finally:\n            try:\n                if stream is not None:\n                    await cleanup(stream)\n            finally:\n                await finish()",
            "        finally:\n            await finish()",
        ),
    ] {
        assert_matches_cpython(&source, "async_manager_finally_join.py", &LEVELS);
    }
}

#[test]
fn populated_elif_chains_preserve_conditional_loop_join() {
    let source = r#"def collect(groups, first, second):
    for group in groups:
        for batch in group:
            for item in batch:
                if item.kind == 1:
                    if first:
                        value = f"first_{item.part}"
                    else:
                        value = "first"
                elif item.kind == 2:
                    assert first
                    if second:
                        value = f"second_{item.part}"
                    elif first:
                        value = f"second_{item.other}"
                    else:
                        value = "second"
                else:
                    assert False
                if item.ready:
                    emit(value)
"#;
    for source in [
        source.to_owned(),
        source.replace("                    assert first\n", ""),
        source.replace(
            "                if item.ready:\n                    emit(value)",
            "                emit(value)",
        ),
        source.replace(
            "                    elif first:\n                        value = f\"second_{item.other}\"\n",
            "",
        ),
        source.replace(
            "                    else:\n                        value = \"second\"",
            "                    elif item.valid:\n                        value = \"third\"\n                    else:\n                        value = \"second\"",
        ),
        source.replace(
            "                        value = f\"second_{item.other}\"",
            "                        assert first",
        ),
        source.replace(
            "                        value = f\"second_{item.other}\"",
            "                        value = [part for part in item.parts]",
        ),
    ] {
        assert_matches_cpython(&source, "populated_chain_loop_join.py", &LEVELS);
    }
}

#[test]
fn protected_nested_joins_keep_their_finally_owner() {
    let source = r"async def collect(items, enabled):
    try:
        if enabled:
            if ready():
                values = {entry.key: entry for entry in items}
            else:
                report('missing')
        else:
            assert valid()
        with manager() as stream:
            consume(stream)
        yield values
    finally:
        await cleanup()
";
    for source in [
        source.to_owned(),
        source.replace("yield values", "await process(values)"),
        source
            .replace("async def", "def")
            .replace("await cleanup()", "cleanup()"),
        source
            .replace("async def", "def")
            .replace("yield values", "consume(values)")
            .replace("await cleanup()", "cleanup()"),
        source.replace(
            "        with manager() as stream:\n            consume(stream)",
            "        consume(values)",
        ),
    ] {
        assert_matches_cpython(&source, "protected_nested_join.py", &LEVELS);
    }
}

#[test]
fn erased_elif_chains_preserve_live_loop_join() {
    let source = r"def collect(items, kinds, enabled):
    for value in items:
        if enabled:
            if value.kind == kinds.first:
                assert value.valid
            elif value.kind == kinds.second:
                assert value.valid
            elif value.kind == kinds.third:
                assert value.valid
            else:
                assert False
        else:
            assert value.kind in kinds.allowed
        consume(value)
    finish()
";
    for source in [
        source.to_owned(),
        source.replace("for value in items:", "while ready(items):"),
        source.replace(
            "    finish()",
            "    else:\n        completed()\n    finish()",
        ),
        source.replace("        consume(value)", "        output = value"),
        source.replace("        consume(value)", "        assert value.valid"),
    ] {
        assert_matches_cpython(&source, "erased_chain_loop_join.py", &LEVELS);
    }
}

#[test]
fn enclosing_finalizer_preserves_live_loop_join() {
    let source = r"def process(items, owner):
    try:
        context = owner.context()
        with context:
            for item in items:
                prepare(item)
                if owner.enabled():
                    if ready():
                        assert context.valid
                    else:
                        assert context.other
                    context.reset()
        finish()
    finally:
        heartbeat()
        with timeout():
            owner.close()
    finish_all()
";
    for source in [
        source.to_owned(),
        source.replace(
            "        heartbeat()\n        with timeout():\n            owner.close()\n",
            "        cleanup()\n",
        ),
        source.replace("for item in items:", "while owner.more():"),
        source.replace("if ready():", "if ready() or other():"),
        source.replace(
            "                    context.reset()",
            "                    assert context.valid",
        ),
    ] {
        assert_matches_cpython(&source, "finally_loop_join.py", &LEVELS);
    }
}

#[test]
fn nested_erased_condition_keeps_live_loop_successor() {
    for successor in ["emit(item)", "result = transform(item)"] {
        let source = format!(
            "def f(items, flag):\n    with manager():\n        for item in items:\n            if flag:\n                if item.ready:\n                    assert item\n                else:\n                    assert flag\n            {successor}\n"
        );
        assert_matches_cpython(&source, "live_loop_successor.py", &LEVELS);
    }
}

#[test]
fn nested_erased_conditions_preserve_manager_join() {
    for managers in ["first(), second()", "first(), second(), third()"] {
        let source = format!(
            "async def f(flag, other):\n    with {managers}:\n        result = await work()\n        if flag:\n            assert result\n            if other:\n                assert result\n            else:\n                assert other\n        else:\n            assert flag\n"
        );
        assert_matches_cpython(&source, "manager_join.py", &LEVELS);
    }
}

#[test]
fn erased_iterator_else_preserves_optional_inner_branch() {
    for definition in ["def", "async def"] {
        for body in [
            "                if magnitude(item) / scale(item) > limit:\n                    output.emit(item)\n",
            "                if item:\n                    output.emit(item)\n",
            "                if item:\n                    output.emit(item)\n                assert item\n",
            "                if item:\n                    output.emit(item)\n                else:\n                    consume(item)\n",
        ] {
            let source = format!(
                "{definition} f(manager, items, output, limit):\n    with manager:\n        for item in items:\n            if first(item):\n                consume(item)\n            elif accepts(item):\n                assert item\n{body}            else:\n                assert False\n"
            );
            assert_matches_cpython(&source, "optional_inner_branch.py", &LEVELS);
        }
    }
}

#[test]
fn erased_iterator_else_preserves_conditional_call_argument_join() {
    for definition in ["def", "async def"] {
        for call in [
            "receiver.send(value=item, option=factory() if factory else None)",
            "consume(item, factory() if factory else None)",
            "receiver.send(value=item, option=factory())",
        ] {
            let source = format!(
                "{definition} f(manager, items, receiver, factory):\n    with manager:\n        for item in items:\n            if check(item):\n                consume(item)\n            elif receiver is not None:\n                assert item\n                {call}\n            else:\n                assert factory\n"
            );
            assert_matches_cpython(&source, "conditional_argument_iterator.py", &LEVELS);
        }
    }
}

#[test]
fn erased_inner_iterator_body_preserves_outer_condition_backedge() {
    for definition in ["def", "async def"] {
        for body in ["assert value", "pass", "emit(value)"] {
            let source = format!(
                "{definition} f(manager, groups):\n    with manager:\n        for group in groups:\n            prepare(group)\n            for item in group:\n                if valid(item):\n                    for value in values(item):\n                        {body}\n                else:\n                    assert item\n            finish(group)\n"
            );
            assert_matches_cpython(&source, "erased_inner_iterator.py", &LEVELS);
        }
    }
}

#[test]
fn erased_else_preserves_outer_iterator_after_inner_for() {
    for prefix in ["", "                assert item\n"] {
        let source = format!(
            "def f(items):\n    try:\n        for item in items:\n            if first(item):\n                emit(item)\n            elif second(item) and ready(item):\n{prefix}                for value in values(item):\n                    emit(value)\n            else:\n                assert False\n    except ValueError:\n        raise\n"
        );
        assert_matches_cpython(&source, "outer_iterator.py", &LEVELS);
    }
}

#[test]
fn erased_else_preserves_populated_iterator_elif() {
    for body in [
        "                emit(value)\n",
        "                if flag:\n                    result = left(value)\n                else:\n                    result = right(value)\n                emit(result)\n",
    ] {
        let source = format!(
            "def f(manager, values, flag):\n    with manager:\n        for value in values:\n            if first(value):\n                emit(value)\n                if flag:\n                    emit(value)\n            elif second(value):\n{body}            else:\n                assert False\n"
        );
        assert_matches_cpython(&source, "populated_iterator_elif.py", &LEVELS);
    }
}

#[test]
fn erased_else_preserves_try_while_work() {
    for prefix in ["", "                assert flag\n"] {
        let source = format!(
            "def f(flag):\n    try:\n        while True:\n            if first(flag):\n                emit(flag)\n            elif second(flag):\n{prefix}                result = transform(flag)\n                emit(result)\n            else:\n                assert False\n    except ValueError:\n        pass\n"
        );
        assert_matches_cpython(&source, "try_while_work.py", &LEVELS);
    }
}

#[test]
fn erased_else_preserves_protected_pass_backedge() {
    for definition in ["def", "async def"] {
        for loop_header in ["while True", "for value in flag"] {
            let source = format!(
                "{definition} f(flag):\n    try:\n        {loop_header}:\n            if flag == 1:\n                pass\n            else:\n                assert False\n    finally:\n        print(flag)\n"
            );
            assert_matches_cpython(&source, "protected_pass_backedge.py", &LEVELS);
        }
    }
}

#[test]
fn erased_else_preserves_protected_iterator_continue() {
    for body in [
        "            if value == 1:\n                continue\n            else:\n                assert False\n",
        "            if value == 0:\n                await sink(value)\n            elif value == 1:\n                continue\n            else:\n                assert False\n",
    ] {
        let source = format!(
            "async def f(values):\n    try:\n        async for value in values:\n{body}    finally:\n        print(values)\n",
        );
        assert_matches_cpython(&source, "protected_iterator_continue.py", &LEVELS);
    }
}

#[test]
fn copied_callable_finalizer_preserves_iterator_break_exits() {
    for (index, successor) in [
        "        cleanup()\n",
        "        if cleanup is not None:\n            cleanup()\n",
        "        if cleanup is not None:\n            cleanup()\n        marker = 1\n",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def sample(items, groups, choose, holder, read, cleanup):\n    for outer in items:\n        for inner in choose(groups, 0):\n            cache: dict = {{}}\n            if cleanup is not None:\n                cleanup()\n            try:\n                first = holder.open(inner)\n                second = read(first)\n                marker = 0\n                break\n            except LookupError as error:\n                seen = error\n                read(seen)\n            finally:\n                if cleanup is not None:\n                    cleanup()\n{successor}"
        );
        assert_matches_cpython(
            &source,
            &format!("copied_callable_break_{index}.py"),
            &LEVELS,
        );
    }

    for (index, source) in [
        "def sample(items, groups, read, cleanup):\n    for outer in items:\n        for inner in groups:\n            try:\n                value = read(inner)\n                break\n            except LookupError as error:\n                read(error)\n            finally:\n                if cleanup is not None:\n                    cleanup()\n        cleanup()\n",
        "def sample(items, groups, read, cleanup):\n    for outer in items:\n        for inner in groups:\n            try:\n                value = read(inner)\n                break\n            except LookupError as error:\n                read(error)\n            finally:\n                if cleanup is not None:\n                    cleanup.close()\n        read(outer)\n",
        "def sample(items, groups, read, cleanup):\n    for outer in items:\n        for inner in groups:\n            try:\n                value = read(inner)\n                break\n            except LookupError as error:\n                read(error)\n            finally:\n                if cleanup is not None:\n                    cleanup()\n",
        "def sample(items, groups, read, cleanup):\n    for outer in items:\n        for inner in groups:\n            try:\n                value = read(inner)\n                break\n            except LookupError:\n                read(outer)\n            finally:\n                if cleanup is not None:\n                    cleanup()\n        cleanup()\n",
        "def sample(items, groups, read, cleanup):\n    for outer in items:\n        for inner in groups:\n            try:\n                value = read(inner)\n                break\n            except LookupError as error:\n                seen = error\n                read(seen)\n            finally:\n                if cleanup is not None:\n                    cleanup(inner)\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("copied_callable_break_control_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn coroutine_mixed_erased_iterator_guard_preserves_shared_backedge() {
    let source = r"async def sample(items, check, first, second, flag, work, manager):
    for item in items:
        if check(item, first) or (not flag and check(item, second)):
            assert item
";
    for (name, source) in [
        ("mixed", source.to_owned()),
        (
            "three_false_edges",
            source.replace(
                "not flag and check(item, second)",
                "not flag and check(item, second) and first",
            ),
        ),
        (
            "disjunction",
            source.replace("(not flag and check(item, second))", "check(item, second)"),
        ),
        (
            "conjunction",
            source.replace(
                "check(item, first) or (not flag and check(item, second))",
                "first and (check(item, first) or check(item, second))",
            ),
        ),
        ("synchronous", source.replace("async def", "def")),
        ("live_body", source.replace("assert item", "work(item)")),
        (
            "live_suffix",
            source.replace(
                "            assert item\n",
                "            assert item\n        work(item)\n",
            ),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("mixed_erased_iterator_guard_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn async_iterator_preserves_shared_try_else_backedges() {
    for (definition, iterator) in [("async def", "async for"), ("def", "for")] {
        for body in [
            "                output[item] = value\n",
            "                value = read(item)\n                if value is None:\n                    continue\n                if not value:\n                    touch(item)\n                else:\n                    output[item] = read(value)\n",
        ] {
            let source = format!(
                "{definition} sample(items, output, read, touch):\n    {iterator} item in items:\n        try:\n            value = read(item)\n            if value is False:\n                raise LookupError\n        except LookupError:\n            touch(item)\n        else:\n            if item.primary is not None:\n                touch(item)\n            elif item.secondary is not None:\n{body}"
            );
            assert_matches_cpython(&source, "shared_try_else_backedge.py", &LEVELS);
        }
    }

    for source in [
        "async def sample(items, output, read, touch):\n    async for item in items:\n        try:\n            value = read(item)\n            if value is False:\n                raise LookupError\n        except LookupError:\n            touch(item)\n        else:\n            if item.secondary is not None:\n                output[item] = value\n",
        "async def sample(items, output):\n    async for item in items:\n        output[item] = item\n",
        "async def sample(items, output, read, touch):\n    async for item in items:\n        try:\n            value = read(item)\n            if value is False:\n                raise LookupError\n        except LookupError:\n            touch(item)\n        else:\n            if item.primary is not None:\n                touch(item)\n            elif item.secondary is not None:\n                output[item] = value\n        touch(item)\n",
        "async def sample(items, output):\n    async for item in items:\n        if item is not None:\n            output[item] = item\n        pass\n",
        "async def sample(items, output):\n    async for item in items:\n        output[item] = item\n        assert item\n",
        "async def sample(items, output):\n    async for item in items:\n        if item is not None:\n            output[item] = item\n            assert item\n",
    ] {
        assert_matches_cpython(source, "async_backedge_control.py", &LEVELS);
    }
}

#[test]
fn erased_branches_preserve_protected_while_join() {
    for loop_test in ["True", "flag", "flag != stop"] {
        let source = format!(
            "def f(manager, flag, stop):\n    with manager:\n        while {loop_test}:\n            if flag:\n                assert flag\n            else:\n                assert stop\n            if stop:\n                break\n            else:\n                print(flag)\n"
        );
        assert_matches_cpython(&source, "protected_while_join.py", &LEVELS);
    }
}

#[test]
fn annotated_comprehension_preserves_implicit_return_location() {
    let source = "async def collect(items, fields):\n    await prepare()\n    result: list[tuple] = [\n        tuple(item for field in fields)\n        for item in items\n    ]\n    assert result\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    assert result\n", ""),
        source
            .replace("async def", "def")
            .replace("    await prepare()\n", ""),
        source
            .replace("result: list[tuple] = [", "result: set[tuple] = {")
            .replace("    ]\n", "    }\n"),
        source
            .replace("result: list[tuple] = [", "result: dict[object, tuple] = {")
            .replace(
                "        tuple(item for field in fields)",
                "        item: tuple(item for field in fields)",
            )
            .replace("    ]\n", "    }\n"),
        source.replace("result: list[tuple]", "result"),
        source.replace("    assert result\n", "    consume(result)\n"),
        source.replace("tuple(item for field in fields)", "item"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("annotated_comprehension_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_match_default_keeps_manager_exit_outside_exception_regions() {
    let source = "def f(manager, other, value):\n    with manager:\n        match value:\n            case 1:\n                return first()\n            case 2:\n                return second()\n            case _:\n                assert False\n";
    for (case, source) in [
        source.to_string(),
        source.replace("with manager:", "with manager, other:"),
        source.replace("case 1:", "case 1 if value:"),
        source.replace("case _:", "case _ if value:"),
        format!("{source}        finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("match_default_manager_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_match_case_suffix_preserves_protected_conditional_join() {
    for managers in ["manager", "manager, other"] {
        for suffix in ["", "        finish()\n"] {
            for body in ["action()", "pass"] {
                let source = format!(
                    "def f(value, ready, manager, other):\n    with {managers}:\n        match value:\n            case 1:\n                if ready:\n                    {body}\n                assert ready\n{suffix}"
                );
                assert_matches_cpython(&source, "protected_match_conditional_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn compacted_branch_pass_preserves_return_copy_location() {
    for definition in ["def", "async def"] {
        let source = format!(
            "{definition} f(kind, left, right, flag):\n    if kind == left:\n        result = left\n        assert flag\n        pass\n    elif kind == right:\n        result = right\n        if flag:\n            assert kind\n        else:\n            assert kind\n        pass\n    else:\n        raise ValueError(\"kind\")\n    assert result\n"
        );
        assert_matches_cpython(&source, "compacted_branch_pass.py", &LEVELS);
    }
}

#[test]
fn protected_iterator_preserves_completed_conditional_backedges() {
    for definition in ["def", "async def"] {
        for managers in ["manager", "manager, other, third"] {
            let source = format!(
                "{definition} f(manager, other, third, values, active):\n    with {managers}:\n        for value in values:\n            if skip(value):\n                continue\n            item = value.load()\n            if active:\n                assert item\n                if item.kind == 1:\n                    expected = {{1: 2}}\n                else:\n                    expected = {{1: 3}}\n                assert expected\n            else:\n                assert item\n"
            );
            assert_matches_cpython(&source, "protected_nested_conditional.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_preserves_erased_branch_backedges() {
    for manager in ["with", "async with"] {
        for iterator in ["for", "async for"] {
            let source = format!(
                "async def collect(manager, items):\n    {manager} manager:\n        with subscribe():\n            previous = None\n            {iterator} item in items:\n                if valid(item) and item.kind == 'entry':\n                    if previous is None:\n                        assert item.value is not None\n                        previous = item.value\n                        assert previous\n                    else:\n                        assert item.value == previous\n            assert previous is not None\n"
            );
            assert_matches_cpython(&source, "protected_iterator_erased_branches.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_chained_condition_preserves_erased_else_backedge() {
    for iterator in ["for", "async for"] {
        for condition in [
            "previous == item.value == None",
            "previous is item.value is None",
            "0 < item.value < 3",
            "previous == item.value == None and ready(item)",
            "previous == item.value == None or ready(item)",
            "not (previous == item.value == None)",
            "previous == await read(item) == None",
            "previous == (item.value if ready(item) else fallback()) == None",
        ] {
            let source = format!(
                "async def collect(manager, items):\n    async with manager:\n        with subscribe():\n            previous = None\n            {iterator} item in items:\n                if valid(item) and item.kind == 'entry':\n                    if {condition}:\n                        assert item.value is not None\n                        previous = item.value\n                    else:\n                        assert item.value == previous\n            assert previous is not None\n"
            );
            assert_matches_cpython(&source, "iterator_chained_erased_else.py", &LEVELS);
        }
    }
}

#[test]
fn async_iterator_erased_suffix_preserves_inner_manager_exit() {
    for outer in ["async with manager:", "try:"] {
        let handler = if outer == "try:" {
            "    except Error:\n        recover()\n"
        } else {
            ""
        };
        let source = format!(
            "async def collect(manager, items):\n    {outer}\n        with subscribe():\n            async for item in items:\n                consume(item)\n            assert items\n{handler}"
        );
        assert_matches_cpython(&source, "async_iterator_inner_manager_exit.py", &LEVELS);
    }
}

#[test]
fn erased_nested_branches_preserve_async_iterator_join() {
    let source = "async def f(values, flag):\n    items = []\n    async for value in values:\n        if not items:\n            if flag:\n                assert value\n            else:\n                assert value\n        else:\n            assert value\n        items.append(value)\n";
    for suffix in ["", "    result = tuple(items)\n    assert result\n"] {
        assert_matches_cpython(
            &format!("{source}{suffix}"),
            "async_iterator_elided_join.py",
            &LEVELS,
        );
    }
}

#[test]
fn erased_coroutine_comparison_arms_preserve_shared_cleanup() {
    for prefix in ["", "    await worker()\n"] {
        let source = format!(
            "async def f(a, b, worker):\n{prefix}    if a == b == \"ready\":\n        assert a\n    else:\n        assert b\n"
        );
        assert_matches_cpython(&source, "coroutine_comparison_cleanup.py", &LEVELS);
    }
}

#[test]
fn coroutine_protected_iterator_preserves_erased_else_backedge() {
    for manager in ["with", "async with"] {
        for body in ["previous = value", "consume(value)", "await consume(value)"] {
            let source = format!(
                "async def collect(manager, items):\n    {manager} manager:\n        previous = None\n        for item in items:\n            value = item.value\n            assert value\n            if previous is None:\n                {body}\n            else:\n                assert previous is value\n"
            );
            assert_matches_cpython(&source, "coroutine_protected_iterator_else.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "async def collect(manager, items):\n    with manager:\n        previous = None\n        for item in items:\n            async def prepare():\n                async with open_stream(item) as stream:\n                    async for packet in stream:\n                        pass\n            with capture():\n                await prepare()\n            value = item.value\n            if previous is None:\n                previous = value\n            else:\n                assert previous is value\n",
        "coroutine_nested_definition_iterator_else.py",
        &LEVELS,
    );
}

#[test]
fn unpacked_store_preserves_loop_backedge_location() {
    let source = "def reset_groups(groups, enabled, limit):\n    first, second, total = 0, 0, 0\n    for group in groups:\n        for item in group:\n            consume(item)\n            if enabled and (item > limit or total > limit):\n                if item:\n                    report(item)\n                first, second, total = 0, 0, 0\n            assert first >= 0\n    finish(total)\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "                if item:\n                    report(item)\n",
            "                report(item)\n",
        ),
        source.replace("first, second, total = 0, 0, 0", "first, second = 0, 0"),
        source.replace(
            "first, second, total = 0, 0, 0",
            "first, second, first = 0, 0, 0",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("unpacked_store_backedge_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_finally_branch_preserves_conditional_continuation() {
    let source = "def configure(kind, ready):\n    try:\n        if kind == 'first':\n            assert ready\n        elif kind == 'second':\n            assert kind\n        else:\n            assert False\n        if ready and available():\n            consume(kind)\n    finally:\n        finish()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "        elif kind == 'second':\n            assert kind\n",
            "",
        ),
        source.replace(
            "        if ready and available():\n            consume(kind)",
            "        consume(kind)",
        ),
        source.replace(
            "        if ready and available():",
            "        assert ready\n        if ready and available():",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_finally_continuation_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn completed_await_arms_preserve_erased_outer_join() {
    let source = r"async def sample(enabled, condition, receiver, wait, touch, check):
    outer = enabled
    inner = condition
    item = receiver
    result = None
    if not outer:
        assert inner or item
        assert item is not None
        if inner:
            await wait(item)
            touch(item)
        else:
            await wait(item)
    else:
        assert check(item)
        assert item is receiver
    receiver.finish(result)
";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "    else:\n        assert check(item)\n        assert item is receiver\n",
            "    else:\n        touch(item)\n        touch(result)\n",
        ),
        source.replace(
            "        assert inner or item\n        assert item is not None\n",
            "        touch(item)\n        touch(result)\n",
        ),
        source.replace(
            "        else:\n            await wait(item)\n",
            "        else:\n            touch(item)\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("completed_await_arm_{index}.py"), &LEVELS);
    }
}

#[test]
fn coroutine_populated_nested_branch_preserves_erased_join() {
    for condition in ["isinstance(item, dict)", "accept(item)"] {
        let source = format!(
            "async def normalize(record):\n    await prepare()\n    if record.values is not None:\n        assert record.values\n        item = record.values[0]\n        if {condition}:\n            label = item.get(\"label\")\n        else:\n            label = getattr(item, \"label\", None)\n        assert label\n    else:\n        assert record.values is None\n    await finish()\n"
        );
        assert_matches_cpython(&source, "coroutine_populated_nested_join.py", &LEVELS);
    }
}

#[test]
fn erased_assertion_preserves_legacy_loop_return_copies() {
    let source = "def f(worker):\n    if not worker.ready:\n        value = worker.read()\n        while value not in [\"a\", \"b\"]:\n            value = worker.read()\n        assert value == \"a\"\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "    if not worker.ready:\n",
            "    if worker.invalid:\n        raise ValueError(\"invalid\")\n    if not worker.ready:\n",
        ),
        source.replace("        assert value == \"a\"\n", "        pass\n"),
        format!("{source}        worker.finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("legacy_loop_return_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_loop_branch_before_condition_keeps_protected_join() {
    for condition in ["first", "first and third", "first or third", "not first"] {
        for gap in ["", "                assert third\n"] {
            let source = format!(
                "def f(manager, items, first, second, third):\n    try:\n        with manager:\n            for item in items:\n                if {condition}:\n                    assert first\n                else:\n                    assert second\n{gap}                if second:\n                    assert second\n                else:\n                    assert first\n    except Exception:\n        recover()\n"
            );
            assert_matches_cpython(&source, "erased_loop_condition_join.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "def f(manager, items, first, second):\n    with manager:\n        try:\n            for item in items:\n                if first:\n                    assert first\n                else:\n                    assert second\n                if second:\n                    assert second\n                else:\n                    assert first\n        except Exception:\n            recover()\n",
        "erased_loop_condition_inner_try.py",
        &LEVELS,
    );
}

#[test]
fn erased_try_else_preserves_manager_borrow_boundary() {
    let source = "async def run():\n    try:\n        async with manager() as resource:\n            await resource.first()\n    except RuntimeError as error:\n        assert error\n    else:\n        assert False\n    async with manager() as resource:\n        await resource.second()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    else:\n        assert False\n", ""),
        source.replace("        assert False\n", "        pass\n"),
        source.replace("        assert False\n", "        finish()\n"),
        source.replace("        async with manager()", "        with manager()"),
        source.replace(
            "        assert False\n",
            "        assert False\n    finally:\n        finish()\n",
        ),
        "async def run():\n    async with manager() as resource:\n        pass\n    try:\n        await resource.first()\n    except ValueError:\n        pass\n    else:\n        assert False\n    configure()\n    async with manager() as resource:\n        await resource.second()\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("erased_manager_try_else_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_suffix_preserves_with_pop_block() {
    let source = "def check(manager):\n    with manager:\n        try:\n            action()\n        except OSError as error:\n            assert error\n        else:\n            raise AssertionError()\n        assert manager\n        assert manager\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("        assert manager\n", ""),
        source.replace("        assert manager\n        assert manager\n", "        finish()\n"),
        source.replace("            assert error\n", "            pass\n"),
        source.replace("except OSError as error:", "except OSError:").replace("            assert error\n", "            assert manager\n"),
        "def check(manager, items):\n    with manager:\n        for item in items:\n            if item:\n                break\n        else:\n            raise ValueError()\n        assert items\n".to_owned(),
        "def check(manager, ready):\n    with manager:\n        while ready():\n            if stop():\n                break\n        else:\n            raise ValueError()\n        assert ready\n".to_owned(),
        "def check(first, second):\n    with first:\n        with second:\n            raise ValueError()\n        assert first\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("erased_with_tail_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_coroutine_handler_assertions_preserve_manager_pop_block() {
    for work in ["worker()", "await worker()"] {
        for exit in ["raise Failure('expected')", "return 42"] {
            let source = format!(
                "async def verify(manager, worker):\n    with manager:\n        try:\n            {work}\n            {exit}\n        except Problem as error:\n            assert error.value\n    await worker()\n"
            );
            assert_matches_cpython(&source, "erased_coroutine_handler_exit.py", &LEVELS);
        }
    }
}

#[test]
fn erased_handler_assertions_preserve_manager_pop_block() {
    const SOURCE: &str = "def f(manager, factory):\n    with manager():\n        obj = factory()\n        try:\n            obj.run()\n            raise ValueError(\"failed\")\n        except Error as exc:\n            text = str(exc)\n            assert \"bad\" in text\n        assert obj.ready\n    finish()\n";
    for (index, assignment) in [
        "text = str(exc)",
        "obj.text = str(exc)",
        "text, other = unpack(exc)",
        "text, *other = unpack(exc)",
        "text = obj.text = str(exc)",
        "(obj\n             .text) = str(exc)",
    ]
    .into_iter()
    .enumerate()
    {
        let source = SOURCE.replace("text = str(exc)", assignment);
        assert_matches_cpython(
            &source,
            &format!("erased_handler_store_{index}.py"),
            &LEVELS,
        );
        let source = source
            .replace("            assert \"bad\" in text\n", "")
            .replace("        assert obj.ready\n", "");
        assert_matches_cpython(&source, &format!("live_handler_store_{index}.py"), &LEVELS);
    }
    for (index, source) in [
        SOURCE.replace("        assert obj.ready\n", ""),
        SOURCE.replace("            assert \"bad\" in text\n", ""),
        SOURCE
            .replace("except Error as exc:", "except Error:")
            .replace("str(exc)", "str(obj)"),
        SOURCE.replace("text = str(exc)", "consume(exc)"),
        SOURCE.replace(
            "        assert obj.ready\n",
            "        assert obj.ready\n        obj.finish()\n",
        ),
        SOURCE.replace("            raise ValueError(\"failed\")\n", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("handler_manager_boundary_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertions_preserve_manager_while_exit() {
    let source = "def f(manager, worker):\n    with manager:\n        while len(worker.items) != 1:\n            worker.advance()\n        assert len(worker.items) == 1\n";
    let repeated = source.replace(
        "        assert len(worker.items) == 1\n",
        "        assert len(worker.items) == 1\n        worker.restart()\n        while len(worker.items) != 2:\n            worker.advance()\n        assert len(worker.items) == 2\n",
    );
    for (index, source) in [
        source.to_owned(),
        repeated,
        source.replace("        assert len(worker.items) == 1\n", "        pass\n"),
        format!("{source}        worker.finish()\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("manager_while_exit_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertion_preserves_conditional_manager_store_location() {
    for assignment in ["value: object = await work()", "value = await work()"] {
        for suffix in ["assert valid(value)", "pass", "check(value)"] {
            let source = format!(
                "async def f(manager, flag):\n    with manager:\n        if flag:\n            {assignment}\n            return\n        else:\n            {assignment}\n            {suffix}\n    after()\n"
            );
            assert_matches_cpython(&source, "conditional_manager_store.py", &LEVELS);
        }
    }
}

#[test]
fn comprehension_assignment_prefix_preserves_finalizer_manager_exit() {
    for suffix in ["assert valid(values)", "pass", "check(values)"] {
        let source = format!(
            "def f(manager, items):\n    with manager:\n        values = [work(item) for item in items]\n        try:\n            assert ready()\n            wait()\n        finally:\n            finish()\n        {suffix}\n"
        );
        assert_matches_cpython(&source, "comprehension_prefix_finalizer.py", &LEVELS);
    }
}

#[test]
fn completed_finalizer_prefix_preserves_manager_exit() {
    for suffix in ["assert done(second)", "pass", "check(second)"] {
        let source = format!(
            "def f(first, second):\n    with first, second:\n        value = acquire(first)\n        try:\n            other = acquire(second)\n        finally:\n            release(first)\n        assert value\n        assert other\n        value = acquire(second)\n        try:\n            assert value\n        finally:\n            release(second)\n        {suffix}\n"
        );
        assert_matches_cpython(&source, "completed_finalizer_prefix.py", &LEVELS);
    }
}

#[test]
fn completed_iterator_prefix_preserves_finalizer_manager_exit() {
    for prefix in [
        "        for item in items:\n            begin(item)\n        value = []\n",
        "        items = [make(item) for item in items]\n        for item in items:\n            begin(item)\n        value = []\n",
    ] {
        for suffix in ["assert valid(items)", "pass", "check(items)"] {
            let source = format!(
                "def f(manager, items):\n    with manager:\n{prefix}        try:\n            for item in items:\n                work(item)\n        except Error as error:\n            raise Failure() from error\n        finally:\n            for item in items:\n                finish(item)\n            for item in items:\n                if active(item):\n                    stop(item)\n                    finish(item)\n        {suffix}\n    after()\n"
            );
            assert_matches_cpython(&source, "iterator_prefix_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_finalizer() {
    for body in ["assert value.ready", "pass", "record(value)"] {
        for successor in [
            "",
            "        record(value)\n",
            "        result = build(value)\n",
        ] {
            let source = format!(
                "def f(flag):\n    try:\n        value = work()\n        assert value\n        if flag:\n            {body}\n        else:\n            assert value.done\n{successor}    finally:\n        cleanup()\n        finish()\n"
            );
            assert_matches_cpython(&source, "conditional_before_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_coroutine_finalizer() {
    for cleanup in ["cleanup()", "await cleanup()"] {
        for body in ["assert value.ready", "pass", "record(value)"] {
            let source = format!(
                "async def f(flag):\n    try:\n        value = await work()\n        if flag:\n            {body}\n        else:\n            assert value.done\n    finally:\n        {cleanup}\n"
            );
            assert_matches_cpython(&source, "conditional_coroutine_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_erased_try_tail() {
    for prefix in [
        "        value = work()\n",
        "        with manager:\n            value = work()\n",
    ] {
        for suffix in ["assert complete()", "record()"] {
            let source = format!(
                "async def f(manager, flag):\n    try:\n{prefix}        if flag == expected:\n            assert valid(value)\n        else:\n            assert ready(value)\n        assert done()\n        {suffix}\n    finally:\n        cleanup()\n        finish()\n"
            );
            assert_matches_cpython(&source, "conditional_erased_try_tail.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_managed_finalizer() {
    for managers in ["manager", "manager, other"] {
        for body in ["assert value.ready", "pass", "record(value)"] {
            let source = format!(
                "def f(manager, other, flag):\n    with {managers}:\n        value: object = setup()\n        try:\n            work(value)\n            if flag is None:\n                {body}\n            else:\n                assert value.done\n        finally:\n            cleanup(value)\n"
            );
            assert_matches_cpython(&source, "conditional_managed_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_guarded_finalizer() {
    for asynchronous in [false, true] {
        for body in ["assert valid()", "pass", "record()"] {
            let source = format!(
                "async def f(flag, resource):\n    try:\n        await work()\n        if flag:\n            {body}\n        else:\n            assert ready()\n    finally:\n        if resource is not None:\n            resource.close()\n            await finish(resource)\n"
            );
            let source = if asynchronous {
                source
            } else {
                source.replace("async def", "def").replace("await ", "")
            };
            assert_matches_cpython(&source, "conditional_guarded_finalizer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_conditional_preserves_jump_before_nested_finalizers() {
    for managers in ["manager", "manager, other"] {
        for body in ["assert valid()", "pass", "record()"] {
            let source = format!(
                "def f(manager, other, flag, resource):\n    try:\n        with {managers}:\n            begin()\n            try:\n                work()\n                if flag:\n                    {body}\n                else:\n                    assert ready()\n            finally:\n                if resource is not None:\n                    resource.close()\n                finish()\n    finally:\n        cleanup()\n"
            );
            assert_matches_cpython(&source, "conditional_nested_finalizers.py", &LEVELS);
        }
    }
}

#[test]
fn elided_handler_assertion_preserves_shared_false_cleanup() {
    for (condition, suffix) in [
        ("config.recover and not stopped", "assert False"),
        ("config.recover and not stopped", "pass"),
        ("config.recover and not stopped", "repair(error)"),
        ("config.recover or not stopped", "assert False"),
    ] {
        let source = format!(
            "def collect(items, config, fallback, stopped):\n    for item in items:\n        try:\n            value = parse(item)\n        except ValueError as error:\n            if {condition}:\n                report(error)\n                return fallback\n            {suffix}\n        consume(value)\n    return items\n"
        );
        assert_matches_cpython(&source, "shared_handler_false_cleanup.py", &LEVELS);
    }
}

#[test]
fn elided_handler_assertion_preserves_conditional_cleanup() {
    for suffix in ["assert value in str(error)", "pass", "after()"] {
        let source = format!(
            "async def f(manager, other, value):\n    async with manager:\n        try:\n            async with await other():\n                work()\n        except Error as error:\n            if value in str(error):\n                recover()\n            {suffix}\n"
        );
        assert_matches_cpython(&source, "handler_conditional_cleanup.py", &LEVELS);
    }
}

#[test]
fn elided_handler_assertion_preserves_while_break_cleanup() {
    for suffix in ["assert found", "pass", "after()"] {
        let source = format!(
            "async def f(worker):\n    try:\n        await worker.run()\n    except Error as error:\n        found = False\n        while error.cause:\n            if \"marker\" in str(error):\n                found = True\n                break\n            error = error.cause\n        {suffix}\n"
        );
        assert_matches_cpython(&source, "handler_while_break_cleanup.py", &LEVELS);
    }
    for definition in ["def", "async def"] {
        let source = format!(
            "{definition} f(worker):\n    try:\n        worker.run()\n    except Error as error:\n        cursor = error\n        found = False\n        while cursor.cause:\n            if \"marker\" in cursor.payload.data.decode():\n                found = True\n                break\n            cursor = cursor.cause\n        assert found\n"
        );
        assert_matches_cpython(&source, "handler_cursor_break_cleanup.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_preserve_nested_recovery_borrow_boundary() {
    for definition in ["def", "async def"] {
        for body in [
            "                assert obj.state is None\n                obj.state = {}\n",
            "                obj.state = {}\n                assert obj.state is None\n",
            "                obj.state = {}\n",
        ] {
            let source = format!(
                "{definition} f(obj, flag):\n    try:\n        if flag and ready(obj):\n            assert obj.ready\n            try:\n{body}            except Exception:\n                pass\n    except Exception:\n        return None\n    use(obj)\n    use(obj)\n"
            );
            assert_matches_cpython(&source, "nested_recovery_borrow.py", &LEVELS);
        }
    }
}

#[test]
fn erased_nested_else_preserves_try_exit_borrow_boundary() {
    let source = "def validate(values, enabled):\n    item = values[0]\n    try:\n        if enabled:\n            if valid(item):\n                consume(item)\n            else:\n                assert item\n    except Exception:\n        return False\n    if 'payload' in item:\n        assert item.payload\n    return True\n";
    let preceding_branch = source.replace(
        "    try:\n",
        "    try:\n        prepare(item)\n        if valid(item):\n            consume(item)\n        else:\n            assert item\n",
    );
    for (index, source) in [
        source.to_owned(),
        preceding_branch.replace(
            "    except Exception:\n",
            "    except Exception as error:\n        report(error)\n",
        ),
        source.replace("                assert item\n", "                consume(item)\n"),
        source.replace("                assert item\n", "                pass\n"),
        source.replace(
            "            else:\n",
            "            elif alternate(item):\n                consume(item)\n            else:\n",
        ),
        source.replace(
            "        if enabled:\n            if valid(item):\n                consume(item)\n            else:\n                assert item\n",
            "        if valid(item):\n            consume(item)\n        else:\n            assert item\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("nested_erased_else_try_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditional_preserves_join_before_async_manager_exit() {
    for body in ["assert flag", "pass"] {
        let source = format!(
            "async def f(manager, outer, inner, stop, flag, work):\n    with manager:\n        while not stop:\n            async with outer:\n                async with inner:\n                    if not flag:\n                        {body}\n                    else:\n                        {body}\n            await work()\n"
        );
        assert_matches_cpython(&source, "async_manager_conditional_join.py", &LEVELS);
    }
    for source in [
        "async def f(inner, stop, flag, work):\n    while not stop:\n        async with inner:\n            if not flag:\n                assert flag\n            else:\n                assert flag\n        await work()\n",
        "async def f(outer, inner, flag, work):\n    async with outer:\n        async with inner:\n            if not flag:\n                assert flag\n            else:\n                assert flag\n    await work()\n",
    ] {
        assert_matches_cpython(source, "async_manager_loop_boundary.py", &LEVELS);
    }
}

#[test]
fn conditional_attribute_copies_fuse_aliased_local_entries() {
    for operator in ["and", "or"] {
        for condition in ["flag", "isinstance(value, str)"] {
            let source = format!(
                "def sample(receiver, explicit, value, flag):\n    receiver.value = explicit {operator} (value if {condition} else None)\n"
            );
            assert_matches_cpython(&source, "conditional_attribute_entry.py", &LEVELS);
        }
    }
}

#[test]
fn conditional_call_preserves_erased_else_try_exit() {
    for (index, (call, otherwise)) in [
        ("consume(item if enabled else values)", "assert item"),
        (
            "consume(\n                    value=item if enabled else values,\n                )",
            "assert item",
        ),
        ("(consume if enabled else report)(item)", "assert item"),
        (
            "if enabled:\n                    prepare(item)\n                else:\n                    prepare(values)\n                consume(item if enabled else values)",
            "assert item",
        ),
        ("consume(item) if enabled else consume(values)", "assert item"),
        ("consume(item if enabled else values)", "consume(item)"),
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def validate(values, enabled):\n    item = values[0]\n    try:\n        if enabled:\n            if valid(item):\n                {call}\n            else:\n                {otherwise}\n    except Exception:\n        return False\n    if 'payload' in item:\n        assert item.payload\n    return True\n"
        );
        assert_matches_cpython(
            &source,
            &format!("conditional_call_try_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertion_preserves_try_exit_borrow_boundary() {
    for definition in ["def", "async def"] {
        for suffix in ["assert value.kind == 'item'", "pass", "check(value)"] {
            let source = format!(
                "{definition} collect(items):\n    result = []\n    for entry in items:\n        try:\n            value = entry.read().value\n            if value.kind == 'group':\n                assert len(value.parts) == 1\n                value = value.parts[0]\n            {suffix}\n        except Error as error:\n            report(error)\n            value = fallback()\n        result.append(value)\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_try_exit_assertion.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "def collect(items):\n    result = []\n    for entry in items:\n        try:\n            value = entry.read()\n            while value.ready and value.active:\n                value = step(value)\n            assert value\n        except Error:\n            value = fallback()\n        result.append(value)\n    return result\n",
        "erased_while_try_exit_assertion.py",
        &LEVELS,
    );
}

#[test]
fn elided_assertion_after_nested_iterator_preserves_condition_backedges() {
    for suffix in ["assert item", "pass", "after()"] {
        let source = format!(
            "def f(items, first, second, left, right, target):\n    for item in items:\n        if first:\n            if left and right:\n                for value in item:\n                    del target[value]\n            {suffix}\n        elif second:\n            values = [value for value in item]\n            assert values\n        else:\n            assert item\n"
        );
        assert_matches_cpython(&source, "nested_iterator_condition_backedges.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(items, first, left, right, target):\n    for item in items:\n        if first:\n            if left and right:\n                for value in item:\n                    del target[value]\n            assert item\n",
        "nested_iterator_terminal_condition.py",
        &LEVELS,
    );
}

#[test]
fn coroutine_erased_while_else_normalizes_protected_backedges() {
    let source = "async def f(receive, process):\n    try:\n        while True:\n            value = receive()\n            if isinstance(value, (list, tuple)):\n                first, second, third = value\n                assert third is not None\n                await process(first=first, second=second, third=third)\n            else:\n                assert False, value\n    except ValueError:\n        report()\n";
    for source in [
        source.to_owned(),
        source.replace("while True:", "while receive.enabled:"),
        source.replace("first, second, third = value", "third = value"),
        source.replace("await process(", "result = await process("),
        source.replace("await ", ""),
        source.replace("assert False, value", "pass"),
    ] {
        assert_matches_cpython(&source, "coroutine_protected_while_else.py", &LEVELS);
    }
}

#[test]
fn erased_while_else_normalizes_protected_backedges() {
    let source = "def process(state):\n    with guard():\n        while True:\n            value = receive()\n            if value == \"start\":\n                work()\n            elif value == \"stop\":\n                other()\n            else:\n                assert False, value\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("while True:", "while state.ready:"),
        source.replace("                assert False, value", "                pass"),
        source.replace("                assert False, value", "                recover(value)"),
        source.replace("            else:\n                assert False, value\n", ""),
        source.replace(
            "                other()",
            "                with state.lock:\n                    state.ready = True\n                    state.lock.notify()\n                    while state.ready:\n                        state.lock.wait()\n                publish(None)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("protected_while_backedge_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_protected_loop_arms_preserve_condition_polarity() {
    for condition in ["check(item)", "not check(item)"] {
        for body in [
            "assert item",
            "assert item\n                assert item.flag",
            "consume(item)",
        ] {
            let source = format!(
                "def f(items):\n    try:\n        for item in items:\n            if {condition}:\n                {body}\n            else:\n                assert item\n        return True\n    except Exception:\n        return False\n"
            );
            assert_matches_cpython(&source, "erased_protected_loop_arms.py", &LEVELS);
        }
    }
}

#[test]
fn erased_match_capture_body_drops_redundant_success_nop() {
    let source = "def f(items):\n    result = {}\n    for key, value in items:\n        match value:\n            case First(value=a):\n                output = transform(a)\n            case Second(value=b):\n                output = transform(b)\n            case other:\n                assert False, other\n        result[key] = consume(output)\n    return result\n";
    for source in [
        source.to_string(),
        source.replace("assert False, other", "pass"),
        source.replace("assert False, other", "output = transform(other)"),
        source.replace(
            "assert False, other",
            "assert other\n                assert valid(other)",
        ),
        source.replace("case other:", "case other if check(other):"),
        source.replace("    result = {}", "    global other\n    result = {}"),
    ] {
        assert_matches_cpython(&source, "erased_match_capture.py", &LEVELS);
    }
}

#[test]
fn erased_match_cases_keep_unlocated_loop_exits() {
    let source = r"def f(items):
    for item in items:
        match item:
            case First() | Second():
                item.value = 1
            case Third():
                assert item.value is not None
            case Fourth():
                pass
            case _:
                raise ValueError(item)
";
    for source in [
        source.to_owned(),
        source.replace("Third()", "Third() | Fifth()"),
        source.replace("Third()", "Third(value)"),
        source.replace("Third()", "3"),
        source.replace("Third():", "Third() if valid(item):"),
        source.replace("assert item.value is not None", "pass"),
        source.replace("assert item.value is not None", "work(item)"),
        format!("{source}        after(item)\n"),
    ] {
        assert_matches_cpython(&source, "erased_match_loop_exit.py", &LEVELS);
    }
}

#[test]
fn erased_match_cases_preserve_loop_backedges() {
    for body in [
        "assert item",
        "pass",
        "consume(item)",
        "assert item\n                consume(item)",
    ] {
        for suffix in ["", "        after()\n"] {
            let source = format!(
                "def f(items):\n    for item in items:\n        match item:\n            case 1:\n                {body}\n            case 2:\n                {body}\n            case _:\n                assert False\n{suffix}"
            );
            assert_matches_cpython(&source, "erased_match_loop_backedges.py", &LEVELS);
        }
    }
}

#[test]
fn elided_async_loop_branches_keep_augmented_assignment_join() {
    for (index, (function, loop_kind, successor)) in [
        ("async def", "async for", "count += 1"),
        ("async def", "async for", "count = count + 1"),
        ("def", "for", "count += 1"),
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "{function} count_events(items):\n    count = 0\n    {loop_kind} item in items:\n        if count == 0:\n            assert item\n        elif count == 1:\n            assert item\n        else:\n            assert item\n        {successor}\n    assert count == 3\n"
        );
        assert_matches_cpython(
            &source,
            &format!("elided_async_loop_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn coroutine_raise_preserves_repeated_scope_footer() {
    for prefix in [
        "    if expected is not None:\n        assert value == expected\n",
        "    if expected is not None:\n        pass\n",
        "",
    ] {
        let source = format!(
            "async def f(expected, error, calls, value):\n{prefix}    calls.append(value)\n    if error is not None:\n        raise error\n"
        );
        assert_matches_cpython(&source, "coroutine_raise_footer.py", &LEVELS);
    }
    assert_matches_cpython(
        "def outer(expected, error, calls):\n    async def f(value):\n        if expected is not None:\n            assert value == expected\n        calls.append(value)\n        if error is not None:\n            raise error\n    return f\n",
        "coroutine_closure_raise_footer.py",
        &LEVELS,
    );
}

#[test]
fn elided_finally_assertion_preserves_exceptional_exit_locations() {
    for tail in ["assert flag", "pass", "after()"] {
        let source = format!(
            "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        finally:\n            if flag:\n                cleanup()\n            {tail}\n"
        );
        assert_matches_cpython(&source, "finally_assertion_tail.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(manager, flag):\n    with manager:\n        try:\n            work()\n        finally:\n            while flag:\n                cleanup()\n            assert flag\n",
        "finally_loop_assertion_tail.py",
        &LEVELS,
    );
}

#[test]
fn elided_finally_assertion_preserves_assignment_exit_locations() {
    let source = "def f(items):\n    try:\n        yield\n    finally:\n        values = {\n            key: value\n            for key, value in items\n        }\n        assert values\n";
    for source in [
        source.to_owned(),
        source.replace("values =", "values: dict ="),
        source.replace("yield", "work()"),
        source
            .replace("def f(", "async def f(")
            .replace("yield", "await work()"),
        source.replace("        assert values\n", ""),
        format!("{source}        pass\n"),
        format!("{source}        work()\n"),
        format!("{source}        global flag\n"),
    ] {
        assert_matches_cpython(&source, "finally_assignment_assertion_tail.py", &LEVELS);
    }
}

#[test]
fn folded_comprehension_filter_chains_preserve_borrow_boundaries() {
    for (index, filter) in [
        "not __debug__ and mapping.get(item) and item",
        "mapping.get(item) and not __debug__ and item",
        "mapping.get(item) and item and not __debug__",
        "mapping.get(item) if not __debug__ if item",
        "mapping.get(item) or not __debug__",
        "not (mapping.get(item) or __debug__)",
        "not (mapping.get(item) or __debug__ or not item)",
        "(not __debug__ if mapping.get(item) else item)",
        "(mapping.get(item)\n        and not __debug__\n        and item)",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def collect(mapping, items):\n    result = [item for item in items if {filter}]\n    return result\n"
        );
        assert_matches_cpython(
            &source,
            &format!("folded_comprehension_filter_chain_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn folded_negated_comprehension_filter_preserves_operand_location() {
    for condition in ["not __debug__", "not True", "not False", "False"] {
        let source = format!(
            "def collect(mapping, items):\n    result = [\n        item for item in items\n        if mapping.get(item)\n        and {condition}\n    ]\n    return result\n"
        );
        assert_matches_cpython(&source, "folded_negated_comprehension_filter.py", &LEVELS);
    }
}

#[test]
fn folded_comprehension_and_tail_preserves_borrow_boundary() {
    for (index, source) in [
        "def collect(mapping, items):\n    result = [item for item in items if mapping.get(item) and not __debug__]\n    return result\n",
        "def collect(mapping, items):\n    result = [item for item in items if mapping.get(item) and True]\n    return result\n",
        "def collect(mapping, items):\n    result = [item for item in items if mapping.get(item)]\n    return result\n",
        "def collect(mapping, items):\n    result = [\n        item for item in items\n        if mapping.get(item)\n        and True\n    ]\n    return result\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("folded_comprehension_and_tail_{index}.py"), &LEVELS);
    }
}

#[test]
fn linear_prefix_and_elided_suffix_preserve_nested_if_join() {
    let source = "def update(state):\n    if state.ready():\n        item = state.next()\n        if available(item):\n            apply(item)\n        else:\n            discard(item)\n        assert state.valid()\n    else:\n        assert state.valid()\n    result = state.finish()\n    assert result\n";
    for source in [
        source.to_owned(),
        source.replace(
            "available(item)",
            "all(value == 1 for value in item.values())",
        ),
        source.replace("available(item)", "any(value for value in item.values())"),
        source.replace("available(item)", "tuple(value for value in item.values())"),
        source.replace("        item = state.next()", "        state.prepare()"),
        source.replace(
            "        item = state.next()",
            "        state.prepare()\n        item = state.next()\n        other = item.value",
        ),
        source.replace("        item = state.next()\n", ""),
        source.replacen("        assert state.valid()", "        consume(item)", 1),
    ] {
        assert_matches_cpython(&source, "prefixed_nested_if_suffix.py", &LEVELS);
    }
}

#[test]
fn erased_prefix_preserves_nested_branch_join() {
    for prefix in [
        "        assert first\n",
        "        assert first\n        assert second\n",
    ] {
        for (body, alternate) in [
            ("assert first", "assert second"),
            ("prepare(first)", "prepare(second)"),
        ] {
            let source = format!(
                "def sample(outer, inner, first, second, prepare, record):\n    if outer:\n{prefix}        if inner:\n            {body}\n        else:\n            {alternate}\n        assert first\n        assert second\n    else:\n        assert second\n        assert first\n    assert first\n    assert second\n    result = record.value\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_prefix_nested_join.py", &LEVELS);
        }
    }
}

#[test]
fn erased_suffix_preserves_assertion_only_nested_join() {
    for prefix in ["", "        work(left)\n", "        saved = left\n"] {
        for assertion in [
            "left == right",
            "(\n                left == right\n            )",
        ] {
            for value in ["result", "work(left, right, outer, inner, result)"] {
                let source = format!(
                    "def sample(outer, inner, left, right, work):\n    if outer:\n{prefix}        if inner:\n            assert {assertion}\n        else:\n            assert left != right\n        assert left\n        assert right\n    else:\n        assert right\n        assert left\n    result = left\n    return {value}\n"
                );
                assert_matches_cpython(&source, "assertion_only_nested_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_else_after_breaking_for_preserves_branch_join() {
    for tail in ["finish()", "value = None", "return None"] {
        let source = format!(
            "def f(flag, items):\n    if flag:\n        for item in items:\n            if item:\n                break\n    else:\n        assert flag\n    {tail}\n"
        );
        assert_matches_cpython(&source, "breaking_for_erased_else.py", &LEVELS);
    }
}

#[test]
fn elided_for_after_completed_prefix_preserves_legacy_footer_location() {
    for source in [
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        value = prepare()\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        value = prepare()\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if active():\n        value = prepare()\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
        "def f(items, others):\n    assert items\n    assert others\n    if ready():\n        assert items\n    if active():\n        value = items\n        for item in items:\n            assert item\n            assert value\n        for other in others:\n            assert other\n",
    ] {
        assert_matches_cpython(source, "completed_for_prefix.py", &LEVELS);
    }
    for (target, first, last) in [
        ("item", "items", "range(count)"),
        ("key, value", "items.items()", "others"),
        ("key, value", "items.items()", "range(count)"),
    ] {
        let source = format!(
            "def f(items, others, count):\n    if ready():\n        assert items\n    if active():\n        value = prepare()\n        for {target} in {first}:\n            assert items\n        for other in {last}:\n            assert other\n"
        );
        assert_matches_cpython(&source, "completed_iterator_call_prefix.py", &LEVELS);
    }
}

#[test]
fn elif_footer_keeps_each_predecessors_location_after_removed_assertion() {
    for otherwise in ["assert kind", "value = build()"] {
        let source = format!(
            "def f(flag, kind, build):\n    if flag:\n        if kind == 1:\n            value = build()\n        elif kind == 2:\n            value = build()\n        else:\n            {otherwise}\n        assert flag\n"
        );
        assert_matches_cpython(&source, "elif_footer.py", &LEVELS);
    }
}

#[test]
fn returning_while_uses_condition_location_after_removed_assertion() {
    for condition in ["value >= 0", "count", "value >= 0 or count"] {
        for prefix in [
            "",
            "            if value > 0 and count != 0:\n                count += 1\n                continue\n",
        ] {
            let source = format!(
                "def f(obj, value, count):\n    if value < 0:\n        return obj.first()\n    else:\n        while {condition}:\n{prefix}            assert count\n            return obj.second()\n        assert value\n"
            );
            assert_matches_cpython(&source, "returning_while_footer.py", &LEVELS);
        }
    }
}

#[test]
fn erased_false_branch_preserves_borrowed_local_loads() {
    for condition in ["False", "__debug__", "0", "0x0"] {
        for assertion in ["assert x", "assert x\n            assert y"] {
            let source = format!(
                "def f(a, b, x, y):\n    if a:\n        if {condition}:\n            {assertion}\n        result = consume(x, y)\n    elif b:\n        result = x\n    else:\n        result = None\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_false_borrow.py", &LEVELS);
        }
    }
}

#[test]
fn genuine_pass_preserves_conditional_expression_join() {
    for (index, expression) in [
        "x if b else y",
        "left(x) if b else y",
        "x if b else right(y)",
        "left(x) if b else right(y)",
        "obj.left if b else obj.right",
        "obj.left(x) if b else obj.right(y)",
        "x[0] if b else y[0]",
        "x if b else 1",
        "1 if b else x",
        "1 if b else 2",
        "x if b else 1 + 2",
        "'value' if b else x",
        "x if b else None",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def f(a, b, x, y, left, right, obj):\n    if a:\n        {expression}\n        pass\n"
        );
        assert_matches_cpython(
            &source,
            &format!("conditional_expression_pass_{index}.py"),
            &LEVELS,
        );
    }
    for (expression, suffix) in [
        ("left() if b else right()", ""),
        ("left() if b else right()", "        assert a\n"),
        ("left() if b else right()", "        pass\n    after()\n"),
        ("x if b else 1", "        pass\n    after()\n"),
    ] {
        let source =
            format!("def f(a, b, x, left, right):\n    if a:\n        {expression}\n{suffix}");
        assert_matches_cpython(&source, "conditional_expression_pass_boundary.py", &LEVELS);
    }
}

#[test]
fn erased_suffix_preserves_conditional_expression_join() {
    for condition in ["b", "b is not None"] {
        for (left, right) in [("x", "y"), ("left(x)", "right(y)")] {
            for suffix in ["        assert a\n", "        assert a\n        assert b\n"] {
                let source = format!(
                    "def f(a, b, x, y):\n    if a:\n        {left} if {condition} else {right}\n{suffix}"
                );
                assert_matches_cpython(&source, "conditional_expression_erased_suffix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_nested_binary_if_preserves_legacy_scope_exit() {
    for (prefix, outer, otherwise) in [
        ("    if a:\n        assert a\n", true, true),
        ("", true, true),
        ("    d = a\n", true, true),
        ("    if a:\n        pass\n", true, true),
        (
            "    if a:\n        assert b\n    else:\n        assert c\n",
            true,
            true,
        ),
        ("    if a:\n        assert a\n", false, true),
        ("    if a:\n        assert a\n", true, false),
        ("    assert a\n", true, true),
    ] {
        let (outer_if, indent) = if outer {
            ("    if b:\n", "        ")
        } else {
            ("", "    ")
        };
        let mut source =
            format!("def f(a, b, c):\n{prefix}{outer_if}{indent}if c:\n{indent}    assert b\n");
        if otherwise {
            writeln!(source, "{indent}else:\n{indent}    assert c").unwrap();
        }
        assert_matches_cpython(&source, "erased_nested_binary_if.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(a, b):\n    if a.ready:\n        assert b\n    if a is None and b is not None:\n        flag = not a.ready\n        if b.ready:\n            assert a\n        else:\n            assert b\n",
        "erased_nested_binary_if_assignment.py",
        &LEVELS,
    );
}

#[test]
fn attribute_identity_tests_preserve_legacy_scope_exit() {
    let erased_prefix =
        "    if obj.ready:\n        assert left\n        assert right\n        assert obj.valid\n";
    for (condition, prefix) in [
        ("left is None and right is not None", erased_prefix),
        ("obj.left is None and obj.right is not None", erased_prefix),
        ("left is None and obj.right is not None", erased_prefix),
        ("obj.left is None and obj.right is not None", ""),
    ] {
        let source = format!(
            "def f(obj, left, right):\n    if obj.stop:\n        return\n    assert obj.valid\n{prefix}    if {condition}:\n        value = not obj.flag\n        if obj.check:\n            assert value\n        else:\n            assert obj.valid\n"
        );
        assert_matches_cpython(&source, "attribute_identity_scope_exit.py", &LEVELS);
    }
    for assignment in [
        "obj.value = not obj.flag",
        "value = not obj.part.flag",
        "obj.value = not obj.part.flag",
    ] {
        let source = format!(
            "def f(obj, left, right):\n    if obj.stop:\n        return\n{erased_prefix}    if obj.left is None and obj.right is not None:\n        {assignment}\n        if obj.check:\n            assert obj.valid\n        else:\n            assert obj.check\n"
        );
        assert_matches_cpython(&source, "attribute_assignment_scope_exit.py", &LEVELS);
    }
}

#[test]
fn erased_linear_prefix_preserves_populated_nested_join() {
    for prefix in [
        "        assert obj.ready\n        value = prepare(obj)\n",
        "        assert obj.ready\n        pass\n        assert obj.valid\n        value = obj\n",
        "        value = prepare(obj)\n",
    ] {
        let source = format!(
            "def f(obj):\n    if obj is not None:\n{prefix}        if obj.left or obj.right:\n            result = value.left\n        else:\n            result = choose(value)\n            witness = value.right\n    else:\n        assert obj is None\n    witness = obj.after\n"
        );
        assert_matches_cpython(&source, "erased_linear_prefix_join.py", &LEVELS);
    }
}

#[test]
fn raising_arm_preserves_nested_erased_else_join() {
    let source = r#"def configure(enabled, path, options, config):
    if enabled:
        assert path is None
    else:
        if path is None:
            if options.family is None:
                raise ValueError("missing")
            else:
                assert options.name is not None
                assert config is None
                item = lookup(options.family)
                path = item.path
                config = item.config
                if item.layout is not None:
                    options.first = item.layout.first
                    options.second = item.layout.second
                else:
                    assert options.valid
        else:
            assert options.family is None
    result = work(config)
    try:
        finish(result)
    except Error:
        raise
    return result
"#;
    for source in [
        source.to_owned(),
        source.replace("raise ValueError(\"missing\")", "raise error"),
        source.replace("raise ValueError(\"missing\")", "raise error from cause"),
        source.replace("if item.layout is not None:", "if item.ready():"),
    ] {
        assert_matches_cpython(&source, "raising_arm_nested_join.py", &LEVELS);
    }
}

#[test]
fn assigned_call_conditional_prefix_preserves_populated_join() {
    let source = r"def choose(owner, incoming, first):
    if incoming is not None:
        active = owner.enabled()
        assert incoming.valid
        if first:
            value = incoming.prepare(first)
        else:
            value = incoming.prepare(0)
        if value.kind != owner.kind:
            value = value.convert(owner.kind)
        options = configure(callback=lambda: None)
        args = pack(value, options) if owner.recompute or active else None
        if active:
            result = None
        else:
            result = emit(value, args)
    else:
        assert first
    consume(result)
";
    for source in [
        source.to_owned(),
        source.replace("owner.recompute or active", "owner.recompute and active"),
        source.replace("owner.recompute or active", "not owner.ready()"),
        source.replace(
            "pack(value, options) if owner.recompute or active else None",
            "value.left if owner.recompute else value.right",
        ),
    ] {
        assert_matches_cpython(&source, "assigned_call_conditional_prefix.py", &LEVELS);
    }
}

#[test]
fn assigned_conditional_prefix_preserves_populated_nested_join() {
    for successor in ["    after = obj\n", "    after = obj.after\n"] {
        let source = format!(
            "def f(obj, a, b, c, d, work):\n    if obj is not None:\n        if a:\n            first = work()\n        else:\n            first = work()\n        if b:\n            second = work()\n        chosen = c if d else a\n        if c:\n            result = None\n        else:\n            result = work()\n    else:\n        assert obj\n{successor}    return after\n"
        );
        assert_matches_cpython(&source, "assigned_conditional_prefix_join.py", &LEVELS);
    }
}

#[test]
fn conditional_prefix_preserves_erased_nested_join() {
    for prefix in [
        "        if b:\n            value = obj.first\n",
        "        if b:\n            value = obj.first\n        else:\n            value = obj.second\n",
        "        if b:\n            value = obj.first\n            assert b\n        else:\n            value = obj.second\n            assert c\n",
        "        if b:\n            return obj.first\n",
        "        if b:\n            value = obj.first\n        else:\n            return obj.second\n",
        "        if b:\n            value = obj.first\n            return value\n",
        "        if b:\n            return obj.first\n        else:\n            value = obj.second\n",
    ] {
        let source = format!(
            "def f(a, b, c, obj):\n    if a:\n{prefix}        if c:\n            assert c\n        else:\n            assert b\n    else:\n        assert a\n    action()\n"
        );
        assert_matches_cpython(&source, "conditional_prefix_join.py", &LEVELS);
    }
}

#[test]
fn comprehension_and_erased_prefixes_preserve_nested_join() {
    let source = "def collect(kind, items, score, count):\n    if kind == 'selected':\n        assert items\n        values = [item.strip().lower() for item in items]\n        assert values\n        if score < 0:\n            assert values\n        else:\n            assert not values\n        if count > 0:\n            assert values\n        else:\n            assert not values\n        if 'key' in values:\n            assert score\n        else:\n            assert count\n    else:\n        assert not items\n    return {'kind': kind, 'items': items, 'score': score, 'count': count}\n";
    for source in [
        source.to_owned(),
        source.replace(
            "[item.strip().lower() for item in items]",
            "{item.strip().lower() for item in items}",
        ),
        source.replace(
            "[item.strip().lower() for item in items]",
            "{item: item.strip().lower() for item in items}",
        ),
        source.replace(
            "[item.strip().lower() for item in items]",
            "[tuple(item for other in items) for item in items]",
        ),
        source.replace("[item.strip().lower() for item in items]", "items"),
        source.replace(
            "{'kind': kind, 'items': items, 'score': score, 'count': count}",
            "{'items': items}",
        ),
        source.replace(
            "    else:\n        assert not items",
            "    else:\n        consume(items)",
        ),
    ] {
        assert_matches_cpython(&source, "comprehension_erased_prefixes.py", &LEVELS);
    }
}

#[test]
fn erased_prefix_preserves_constructed_raise_return_copies() {
    const SOURCE: &str = "def f(obj):\n    if obj.count is not None:\n        assert obj.ready\n    if check(obj.child):\n        if obj.count is not None:\n            raise ValueError(\"count\")\n        values = obj.child.values\n        if any(value % obj.scale for value in values):\n            raise ValueError(\"scale\")\n        if any(value < obj.lower for value in values):\n            raise ValueError(\"lower\")\n        if obj.upper is not None and any(value > obj.upper for value in values):\n            raise ValueError(\"upper\")\n";
    let prefix = "    if obj.count is not None:\n        assert obj.ready\n";
    let mut method = "class C:\n".to_owned();
    for line in SOURCE.lines() {
        writeln!(method, "    {line}").unwrap();
    }
    for (index, source) in [
        SOURCE.to_owned(),
        SOURCE
            .replace("any(value % obj.scale for value in values)", "check(values)")
            .replace("any(value < obj.lower for value in values)", "check(values)")
            .replace("any(value > obj.upper for value in values)", "check(values)"),
        SOURCE.replace("check(obj.child)", "check(obj.child, obj.count, ready=True)"),
        SOURCE.replace("value > obj.upper for value in values", "(value if value else obj.upper) > obj.upper for value in values if value and obj.ready"),
        SOURCE.replace(prefix, ""),
        SOURCE.replace("        assert obj.ready\n", "        pass\n"),
        SOURCE.replace("        assert obj.ready\n", "        obj.prepare()\n"),
        format!("{SOURCE}    finish()\n"),
        "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            raise ValueError(\"bad\")\n".to_owned(),
        "def f(obj):\n    if obj.ready:\n        assert obj.ready\n    if obj.open:\n        if obj.bad:\n            raise obj.error\n".to_owned(),
        method,
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("constructed_raise_scope_exit_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_condition_before_nested_raise_preserves_scope_exit() {
    for prefix in [
        "    if a:\n        assert a\n",
        "",
        "    if a:\n        pass\n",
        "    assert a\n",
        "    if a:\n        assert b\n    else:\n        assert c\n",
    ] {
        let source = format!(
            "def f(a, b, c):\n{prefix}    if b:\n        if c:\n            raise ValueError\n"
        );
        assert_matches_cpython(&source, "nested_raise.py", &LEVELS);
    }
    for tail in ["c", "b and c"] {
        let source = format!(
            "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if {tail}:\n            raise ValueError\n"
        );
        assert_matches_cpython(&source, "nested_raise_condition.py", &LEVELS);
    }
    for guards in 0..=3 {
        for assignment in [false, true] {
            let mut source = "def f(a, obj, enter, check, finish):\n    if a is not None:\n        assert a\n    if enter(obj):\n".to_owned();
            for (index, condition) in ["a", "check(obj)", "a is not None"]
                .into_iter()
                .take(guards)
                .enumerate()
            {
                writeln!(
                    source,
                    "        if {condition}:\n            raise ValueError"
                )
                .unwrap();
                if assignment && index == 0 {
                    source.push_str("        value = obj.value\n");
                }
            }
            if assignment && guards == 0 {
                source.push_str("        value = obj.value\n");
            }
            source.push_str(
                "        if a is not None and finish(obj):\n            raise ValueError\n",
            );
            assert_matches_cpython(&source, "nested_raise_guards.py", &LEVELS);
        }
    }
    for source in [
        "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            pass\n",
        "def f(a, b, c):\n    if a:\n        assert a\n    if b:\n        if c:\n            raise ValueError\n    return None\n",
        "def f(a, obj, finish):\n    if a is not None:\n        assert a\n    if a is not None and finish(obj):\n        raise ValueError\n",
    ] {
        assert_matches_cpython(source, "nested_raise_control.py", &LEVELS);
    }
}

#[test]
fn populated_nested_elif_preserves_outer_join() {
    let source = r#"def select(outer, first, second, flag, value, action):
    if outer:
        count = action()
        if first is not None:
            assert value is None
            values = [count]
            value = action(values)
        else:
            if second is not None:
                assert value.ready
                assert value.valid
                if value.stride(-1) != 1:
                    raise ValueError("invalid stride")
                value = value.T
            elif flag:
                value = action(count)
            else:
                value = action(count)
                value = value.T
    else:
        assert first is None
    action(value)
"#;
    for source in [
        source.to_owned(),
        source.replace(
            "                if value.stride(-1) != 1:\n                    raise ValueError(\"invalid stride\")\n",
            "",
        ),
        source.replace("        assert first is None", "        action(first)"),
    ] {
        assert_matches_cpython(&source, "populated_nested_elif_join.py", &LEVELS);
    }
}

#[test]
fn generator_call_stores_preserve_populated_nested_join() {
    let source = r"def choose(result, items):
    if result is None:
        flag = any(check(item) for item in items)
        if flag:
            assert all(check(item) for item in items)
            selected = next(item for item in items if check(item))
            result = build(selected)
        else:
            result = fallback(items)
    else:
        assert valid(result)
    consume(result)
    return result
";
    let ordinary_prefix = source.replace("any(check(item) for item in items)", "check(items)");
    let without_assertion = |source: &str| {
        source.replace(
            "            assert all(check(item) for item in items)\n",
            "",
        )
    };
    let linear_selection = |source: &str| {
        source.replace(
            "next(item for item in items if check(item))",
            "select(items)",
        )
    };
    for source in [
        source.to_owned(),
        linear_selection(&without_assertion(source)),
        without_assertion(&ordinary_prefix),
        linear_selection(&ordinary_prefix),
        linear_selection(&without_assertion(&ordinary_prefix)),
        source.replace("        assert valid(result)", "        consume(result)"),
    ] {
        assert_matches_cpython(&source, "generator_call_store_join.py", &LEVELS);
    }
}

#[test]
fn completed_comprehension_preserves_nested_branch_join() {
    let source = "def collect(options, values):\n    result = options.values\n    if result is None:\n        if values is not None:\n            result = [item.value for item in values]\n        else:\n            result = make_values()\n    else:\n        assert result\n    output = []\n    for item in result:\n        output.append(item)\n    return output\n";
    for source in [
        source.to_owned(),
        source.replace(
            "[item.value for item in values]",
            "[key for key, item in values.items() if item.enabled]",
        ),
        source.replace(
            "[item.value for item in values]",
            "{item.value for item in values}",
        ),
        source.replace(
            "[item.value for item in values]",
            "{item.key: item.value for item in values}",
        ),
        source.replace(
            "[item.value for item in values]",
            "[(item, other) for item in values if item for other in values if other]",
        ),
        source.replace(
            "[item.value for item in values]",
            "[tuple(item for other in values) for item in values]",
        ),
        source.replace(
            "            result = [item.value for item in values]",
            "            prepared = [item.value for item in values]\n            result = finish(prepared)",
        ),
        source.replace("        assert result", "        consume(result)"),
        source[..source.find("    output = []").unwrap()].to_owned(),
    ] {
        assert_matches_cpython(&source, "completed_comprehension_join.py", &LEVELS);
    }
}

#[test]
fn completed_loop_preserves_nested_branch_join() {
    let source = r#"def render(outputs, allowed):
    if outputs is None:
        prefix = "empty"
    elif isinstance(outputs, tuple):
        prefix = f"{outputs!r}"
    elif isinstance(outputs, dict):
        values = [value for value in outputs.values() if ready(value)]
        if values and all(value in allowed for value in values):
            prefix = f"{outputs!r}"
        else:
            token = make()
            prefix = f"{show(token)} = call"
            for name, value in outputs.items():
                if not ready(value):
                    continue
                register(value, f"{show(token)}[{show(name)}]")
    else:
        assert False
    consume(prefix, outputs, allowed)
    return prefix
"#;
    for source in [
        source.to_owned(),
        source.replace("values and all", "values or all"),
        source.replace(
            "if values and all(value in allowed for value in values):",
            "if not (values and all(value in allowed for value in values)):",
        ),
        source.replace(
            "                if not ready(value):\n                    continue\n",
            "",
        ),
    ] {
        assert_matches_cpython(&source, "completed_loop_join.py", &LEVELS);
    }
}

#[test]
fn conditional_prefixes_with_erased_else_preserve_nested_join() {
    for prefix in [
        "        if b:\n            value = obj.first\n        if c:\n            value = obj.second\n",
        "        if b:\n            value = obj.first\n        if c:\n            value = obj.second\n        else:\n            assert c\n",
        "        if b:\n            value = obj.first\n        else:\n            assert b\n        if c:\n            value = obj.second\n",
        "        if b:\n            value = obj.first\n        else:\n            assert b\n",
        "        if b:\n            value = obj.first\n        if c:\n            value = obj.second\n        else:\n            value = obj.third\n",
        "        if b:\n            value = obj.first\n        else:\n            assert b\n        if c:\n            value = obj.second\n        else:\n            assert c\n",
    ] {
        let source = format!(
            "def f(a, b, c, obj):\n    if a:\n{prefix}        if c:\n            assert c\n        else:\n            assert b\n    else:\n        assert a\n    action()\n"
        );
        assert_matches_cpython(&source, "conditional_prefix_elses.py", &LEVELS);
    }
}

#[test]
fn conditional_infinite_loop_preserves_shared_scope_return() {
    let source = "def f(state, work):\n    if state.check:\n        assert state.valid\n    work()\n    if not state.check:\n        while True:\n            try:\n                value = work()\n            except ValueError as error:\n                report(error)\n            if not state.repeat:\n                return value.result\n";
    for source in [
        source.to_owned(),
        source.replace("    if state.check:\n        assert state.valid\n", ""),
        source.replace("while True:", "while state.running:"),
        source
            .replace("except ValueError as error:", "except ValueError:")
            .replace("report(error)", "report()"),
        format!("{source}    work()\n"),
        source.replace(
            "        while True:\n",
            "        while True:\n            for item in work():\n                break\n",
        ),
    ] {
        assert_matches_cpython(&source, "conditional_infinite_loop_footer.py", &LEVELS);
    }
}

#[test]
fn module_terminal_calls_preserve_shared_return_locations() {
    let source = "if __name__ == \"__main__\":\n    if not wants_help():\n        assert ready()\n    launch(\n        value=config,\n    )\n";
    for source in [
        source.to_owned(),
        source.replace("launch(", "worker.launch("),
        source.replace("assert ready()", "pass"),
        format!("{source}consume(config)\n"),
        format!(
            "\"description\"\nfrom __future__ import annotations\nimport library\ndef prepare(value: int) -> None:\n    consume(value)\n\n{source}"
        ),
    ] {
        assert_matches_cpython(&source, "module_call_footer.py", &LEVELS);
    }
}

#[test]
fn coroutine_call_footer_preserves_original_location() {
    for prefix in [
        "",
        "    await prepare()\n",
        "    items = [item.value for item in obj]\n",
    ] {
        for prior in ["", "    if value in {1, 2}:\n        assert obj\n"] {
            let source = format!(
                "async def f(value, obj):\n{prefix}{prior}    if value == 3:\n        assert obj\n        obj.work()\n"
            );
            assert_matches_cpython(&source, "coroutine_call_footer.py", &LEVELS);
        }
    }
}

#[test]
fn async_manager_setups_preserve_repeated_loop_backedges() {
    let prefix = "    async with manager:\n        value = await worker()\n";
    let source = format!(
        "async def f(manager, worker, items):\n{prefix}    for item in items:\n        assert item\n        if item.kind == 1:\n            assert value\n        if item.kind == 0:\n            assert value\n"
    );
    for source in [
        source.clone(),
        source.replace(prefix, "    async with manager:\n        async with manager:\n            value = await worker()\n"),
        source.replace("    for item", "        for item")
            .replace("        assert item", "            assert item")
            .replace("        if item", "            if item")
            .replace("            assert value", "                assert value"),
        format!("{source}        worker()\n"),
        format!("{source}    worker()\n"),
        format!("async def f(manager, worker, first, second):\n{prefix}    if first:\n        assert second\n    if second:\n        worker()\n"),
    ] {
        assert_matches_cpython(&source, "async_manager_loop_backedge.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_branches_preserve_shared_return_locations() {
    let definition = "    def choose(value):\n        if value:\n            return first\n        return False\n";
    let source = format!(
        "async def f(work, first, second):\n{definition}    await work(choose)\n    value = await work()\n    assert value\n    if first is not None:\n        assert value\n    if second:\n        assert value\n    else:\n        assert value\n"
    );
    for source in [
        source.clone(),
        source
            .replace(definition, "")
            .replace("work(choose)", "work(first)"),
        source.replace("await ", ""),
        source.replace("    if first is not None:\n        assert value\n", ""),
        source.replace(
            "    else:\n        assert value\n",
            "    else:\n        pass\n",
        ),
        source.replace("    if second:", "    if not second:"),
    ] {
        assert_matches_cpython(&source, "coroutine_erased_branches_footer.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_else_preserves_shared_return_locations() {
    let source = "async def f(manager, worker, first, second, items):\n    with manager:\n        await worker()\n    values = {item: item for item in items}\n    item = values[0]\n    if first:\n        assert item\n    if second:\n        assert item\n        value = values[1]\n        assert value\n    else:\n        assert item\n";
    for source in [
        source.to_owned(),
        source.replace(
            "    with manager:\n        await worker()\n    values = {item: item for item in items}\n",
            "    values = items\n",
        ),
        source.replace("value = values[1]", "value = worker()"),
        source.replace("    else:\n        assert item", "    else:\n        pass"),
        source.replace("    else:\n        assert item", "    else:\n        worker()"),
        format!("{source}    worker()\n"),
    ] {
        assert_matches_cpython(&source, "coroutine_erased_else_footer.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_inside_terminal_call_suite_preserve_footer() {
    for prefix in [
        "    if a:\n        assert a\n",
        "",
        "    if a:\n        pass\n",
        "    assert a\n",
        "    if a:\n        assert a\n    else:\n        assert b\n",
    ] {
        for body in [
            "        assert b\n        finish(obj)\n",
            "        value = obj.left\n        assert value\n        value = obj.right\n        finish(value)\n",
            "        pass\n        assert b\n        finish(obj)\n",
            "        17\n        assert b\n        finish(obj)\n",
            "        assert a\n        value = obj.left\n        assert value\n        obj.finish(value)\n        assert b\n",
        ] {
            let source = format!("def f(a, b, obj, finish):\n{prefix}    if b:\n{body}");
            assert_matches_cpython(&source, "internal_erased_assert_call.py", &LEVELS);
        }
    }
}

#[test]
fn elided_condition_before_terminal_for_preserves_legacy_footer_location() {
    for first in ["a", "check(a)"] {
        for second in ["b", "check(b)"] {
            for loop_prefix in ["", "        assert a\n"] {
                let source = format!(
                    "def f(a, b, items):\n    assert a\n    if {first}:\n        assert a\n    if {second}:\n{loop_prefix}        for item in items:\n            assert item\n"
                );
                assert_matches_cpython(&source, "elided_condition_loop_footer.py", &LEVELS);
            }
        }
    }
}

#[test]
fn attribute_iterator_preserves_original_footer_location() {
    for prefix in ["    if obj.ready:\n        assert obj.ready\n", ""] {
        let source = format!(
            "def f(obj):\n{prefix}    if obj.active and obj.target is not None:\n        for item in obj.items:\n            item.action()\n"
        );
        assert_matches_cpython(&source, "attribute_iterator_footer.py", &LEVELS);
    }
}

#[test]
fn method_iterator_preserves_original_footer_location() {
    for (target, iterator, following) in [
        ("item", "obj.items()", ""),
        ("key, item", "obj.values.items()", ""),
        ("key, item", "obj.values.items(argument)", ""),
        ("key, item", "items(argument)", ""),
        ("key, item", "values", ""),
        ("key, item", "obj.values", ""),
        ("key, item", "obj.values.items()", "    finish()\n"),
    ] {
        let source = format!(
            "def f(obj, items, values, argument):\n    if obj.optional is not None:\n        assert obj\n    if obj.values:\n        for {target} in {iterator}:\n            assert item\n{following}"
        );
        assert_matches_cpython(&source, "method_iterator_footer.py", &LEVELS);
    }
}

#[test]
fn linear_for_method_arguments_preserve_original_footer_location() {
    for arguments in [
        "argument",
        "argument, other",
        "argument.value",
        "produce(argument)",
        "argument=argument",
        "*argument",
        "__debug__",
    ] {
        let source = format!(
            "def f(obj, argument, other):\n    if obj.ready:\n        assert obj.ready\n    if obj.active and obj.target is not None:\n        for item in obj.items:\n            item.action({arguments})\n"
        );
        assert_matches_cpython(&source, "linear_for_method_arguments.py", &LEVELS);
    }
}

#[test]
fn linear_for_unpacking_preserves_original_footer_location() {
    for (target, iterator, body) in [
        ("item", "make_items(items, argument)", "assert argument"),
        ("item, other", "items", "assert argument"),
        (
            "item, other",
            "make_items(items, argument=argument)",
            "assert argument",
        ),
        ("()", "make_items(items, argument)", "assert argument"),
        (
            "item, other",
            "make_items(items, argument.extra)",
            "item.action(argument)",
        ),
    ] {
        let source = format!(
            "def f(obj, items, argument):\n    if obj.ready:\n        assert obj.ready\n    if obj.active and obj.target is not None:\n        for {target} in {iterator}:\n            {body}\n"
        );
        assert_matches_cpython(&source, "linear_for_unpacking.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_footer_inherits_live_branch_location() {
    for exit in ["raise ValueError", "return", "return b"] {
        for suffix in [
            "",
            "        assert a\n",
            "        assert a\n        assert b\n",
        ] {
            let source = format!(
                "def f(a, b):\n    if a:\n        if b:\n            value = action()\n        else:\n            {exit}\n{suffix}    else:\n        raise RuntimeError\n"
            );
            assert_matches_cpython(&source, "elided_assertion_footer.py", &LEVELS);
        }
    }
}

#[test]
fn elided_call_suffix_preserves_legacy_footer_location() {
    for (first, second) in [("a", "b"), ("a is not None", "b is not None")] {
        for call in ["action(obj)", "obj.update(b)"] {
            for suffix in [
                "        assert obj\n",
                "        assert obj\n        assert b\n",
            ] {
                let source = format!(
                    "def f(a, b, obj):\n    if {first}:\n        assert a\n    if {second}:\n        value = factory(b)\n        {call}\n{suffix}"
                );
                assert_matches_cpython(&source, "elided_call_suffix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_async_for_assertions_preserve_continue_tail() {
    for prefix in ["", "        total += 1\n"] {
        for work in ["", "            skipped += 1\n"] {
            for suffix in ["", "        assert item\n        assert total\n"] {
                let source = format!(
                    "async def f(items):\n    total = 0\n    skipped = 0\n    async for item in items:\n{prefix}        if item is None:\n{work}            continue\n{suffix}"
                );
                assert_matches_cpython(&source, "async_for_continue_tail.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_assertions_preserve_terminal_call_join_boundaries() {
    for call in [
        "sink(x if flag else y)",
        "sink.call(x, y, x if flag else y)",
    ] {
        for suffix in ["        assert x\n", "        assert x\n        assert y\n"] {
            for tail in [
                "",
                "    else:\n        sink(x)\n",
                "    elif flag:\n        sink(x if tag else y)\n        assert y\n    else:\n        sink(y)\n",
            ] {
                let source = format!(
                    "def f(tag, sink, x, y, flag):\n    if tag:\n        {call}\n{suffix}{tail}"
                );
                assert_matches_cpython(&source, "elided_assertion_call_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn final_elif_preserves_nested_assignment_join() {
    for prefix in [
        "    if a == 1:\n        value = obj.first\n",
        "    if a == 0:\n        return obj.first\n    elif a == 1:\n        value = obj.first\n",
    ] {
        for condition in ["b", "b and c", "b or c"] {
            let source = format!(
                "def f(a, b, c, obj):\n{prefix}    elif a == 2:\n        if {condition}:\n            value = obj.left\n        else:\n            value = obj.right\n    else:\n        assert a\n    action()\n"
            );
            assert_matches_cpython(&source, "final_elif_nested_join.py", &LEVELS);
        }
    }
}

#[test]
fn ordinary_await_preserves_repeated_condition_backedge_locations() {
    for (before, inside, after) in [
        ("    await a\n", "", ""),
        ("", "        await a\n", ""),
        ("", "", "    await a\n"),
        ("    await a\n    await b\n", "", ""),
    ] {
        let source = format!(
            "async def f(items, a, b):\n{before}    for item in items:\n{inside}        if a:\n            assert item\n        if b:\n            assert item\n{after}"
        );
        assert_matches_cpython(&source, "ordinary_await_backedge.py", &LEVELS);
    }
}

#[test]
fn sync_manager_preserves_elided_branch_pop_block() {
    for (manager, indent) in [
        ("    with cm:\n", "        "),
        ("    with cm, other:\n", "        "),
        ("    with cm:\n        with other:\n", "            "),
    ] {
        for prefix in [
            "",
            "try:\n    work()\nexcept ValueError:\n    recover()\nwork()\n",
        ] {
            for (true_arm, false_arm) in [
                ("assert flag", "raise ValueError"),
                ("raise ValueError", "assert flag"),
            ] {
                let body = format!("{prefix}if flag:\n    {true_arm}\nelse:\n    {false_arm}\n");
                let mut source = format!("def f(cm, other, flag):\n{manager}");
                for line in body.lines() {
                    source.push_str(indent);
                    source.push_str(line);
                    source.push('\n');
                }
                assert_matches_cpython(&source, "manager_elided_receiver.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_match_case_suffix_preserves_join_before_successor() {
    for scope in ["def", "async def"] {
        for assertion in ["            assert c\n", "\n"] {
            for successor in [
                "    action()\n",
                "    with cm:\n        action()\n",
                "    try:\n        action()\n    except ValueError:\n        recover()\n",
                "    if b:\n        action()\n",
            ] {
                let source = format!(
                    "{scope} f(a, b, c, cm):\n    match a:\n        case 1:\n            if c:\n                action()\n{assertion}{successor}"
                );
                assert_matches_cpython(&source, "match_case_erased_suffix.py", &LEVELS);
            }
        }
    }
}

#[test]
fn coroutine_sequential_conditions_preserve_erased_joins() {
    for next_test in ["second", "second and third", "second or third"] {
        let source = format!(
            "async def f(items, first, second, third):\n    await prepare()\n    for item in items:\n        if first:\n            assert item\n        else:\n            assert items\n        if {next_test}:\n            assert item\n        else:\n            assert items\n"
        );
        assert_matches_cpython(&source, "coroutine_sequential_joins.py", &LEVELS);
    }
}

#[test]
fn coroutine_loop_store_preserves_elided_join_nop() {
    for target in ["item.state", "item.parent.state", "item[0]"] {
        let source = format!(
            "async def f(items):\n    await prepare()\n    for item in items:\n        if item.kind == 1:\n            assert item\n        else:\n            assert items\n        {target} = None\n        assert item\n"
        );
        assert_matches_cpython(&source, "coroutine_loop_store_join.py", &LEVELS);
    }
}

#[test]
fn coroutine_linear_prefix_preserves_nested_elided_join_nop() {
    for manager in ["with", "async with"] {
        for prefix in [
            "value = current.state.value",
            "current.value = result",
            "value = await read(current)",
        ] {
            let source = format!(
                "async def inspect(manager, enabled, kind, flag):\n    {manager} manager as current:\n        result = await current.read()\n        if enabled and kind == 'ready':\n            assert result\n            {prefix}\n            if flag:\n                assert result\n            else:\n                assert not result\n            assert result\n        else:\n            assert not result\n"
            );
            assert_matches_cpython(&source, "coroutine_prefix_nested_join.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_manager_preserves_nested_elided_join_nop() {
    for manager in ["with", "async with"] {
        for prefix in ["", "        work()\n", "        await work()\n"] {
            for continuation in ["", "        work()\n"] {
                let source = format!(
                    "async def f(cm, a, b):\n    {manager} cm:\n{prefix}        if a:\n            if b:\n                assert b\n            else:\n                assert a\n        else:\n            assert a\n{continuation}"
                );
                assert_matches_cpython(&source, "coroutine_manager_nested_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_non_final_disjunction_keeps_backedge_copy_order() {
    for (case, source) in [
        "def f(items, flag, a, b):\n    for item in items:\n        if flag:\n            if a or b:\n                assert item\n        else:\n            work(item)\n",
        "def f(items, flag, a, b, c):\n    for item in items:\n        if flag:\n            if a or b or c:\n                assert item\n        else:\n            pass\n",
        "def f(items, flag, a, b):\n    for outer in items:\n        for item in items:\n            if flag:\n                if a == 1 or check(b):\n                    assert item\n            else:\n                work(item)\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("elided_non_final_disjunction_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_return_guard_preserves_authored_else_jump() {
    let header = "def sample(outer, check, left, right, work, manager, items, limit):\n";
    let source = |guard: &str, body: &str, suffix: &str, alternate: Option<&str>| {
        let mut source = format!("{header}    if outer(left, right):\n        if {guard}:\n");
        for line in body.lines() {
            writeln!(source, "            {line}").unwrap();
        }
        for line in suffix.lines() {
            writeln!(source, "        {line}").unwrap();
        }
        if let Some(alternate) = alternate {
            writeln!(source, "    else:\n        {alternate}").unwrap();
        }
        source.push_str("    return 0\n");
        source
    };
    let guard = "check(left, right)";
    let mut cases = Vec::new();
    for exit in ["return 1", "raise ValueError"] {
        for alternate in ["assert right", "pass", "work(right)"] {
            cases.push(source(guard, exit, "assert left", Some(alternate)));
        }
        cases.push(source(guard, exit, "assert left", None));
    }
    cases.push(source(guard, "return 1", "", Some("assert right")));
    for guard in ["not check(left, right)", "left or right", "left and right"] {
        cases.push(source(
            guard,
            "return 1",
            "assert left",
            Some("assert right"),
        ));
    }
    for body in ["work(left)\nreturn 1", "if left:\n    work(left)\nreturn 1"] {
        cases.push(source(guard, body, "assert left", Some("assert right")));
    }
    let ordinary = source(guard, "return 1", "assert left", Some("assert right"));
    let body = ordinary.strip_prefix(header).unwrap();
    for (owner, cleanup) in [
        ("with manager:", ""),
        ("try:", "    finally:\n        work(right)\n"),
    ] {
        let mut wrapped = format!("{header}    {owner}\n");
        for line in body.lines() {
            writeln!(wrapped, "    {line}").unwrap();
        }
        wrapped.push_str(cleanup);
        cases.push(wrapped);
    }
    cases.push(ordinary.replace(
        "    return 0\n",
        "    return work(left, right, limit, items, manager)\n",
    ));
    for source in cases {
        assert_matches_cpython(&source, "erased_return_guard_else.py", &LEVELS);
    }
}

#[test]
fn erased_assertions_preserve_raising_try_guard_exit_locations() {
    for scope in ["def", "async def"] {
        for condition in ["flag", "not flag", "flag or other"] {
            for suffix in ["", "        assert flag\n        assert other, explain()\n"] {
                let source = format!(
                    "{scope} f(flag, other):\n    try:\n        if {condition}:\n            raise ValueError(flag) from other\n{suffix}    except Exception:\n        recover()\n    finish()\n"
                );
                assert_matches_cpython(&source, "erased_raising_guard_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn erased_assertions_preserve_raising_try_else_guard_exit_locations() {
    for scope in ["def", "async def"] {
        for condition in ["flag is not None", "not flag", "flag or other"] {
            for suffix in ["", "        assert flag\n        assert other, explain()\n"] {
                let source = format!(
                    "{scope} f(flag, other):\n    try:\n        work()\n    except Exception:\n        recover()\n    else:\n        if {condition}:\n            raise ValueError(flag) from other\n{suffix}    finish()\n"
                );
                assert_matches_cpython(&source, "erased_raising_else_guard_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn nested_boolean_assignment_joins_preserve_nops_before_elided_else() {
    for condition in ["b or c", "b and c", "(b or c) and d", "not (b and c)"] {
        for continuation in ["work(value)", "return value"] {
            let source = format!(
                "def f(a, b, c, d):\n    if a:\n        if {condition}:\n            value = 1\n        else:\n            value = 2\n    else:\n        assert a\n    {continuation}\n"
            );
            assert_matches_cpython(&source, "nested_boolean_assignment_join.py", &LEVELS);
        }
    }
}

#[test]
fn nested_assignment_joins_preserve_nops_before_elided_else() {
    for prefix in ["", "        value = 0\n", "        work()\n"] {
        for true_arm in ["assert b", "value = 1"] {
            for continuation in ["work()", "return value"] {
                let source = format!(
                    "def f(a, b):\n    if a:\n{prefix}        if b:\n            {true_arm}\n        else:\n            value = 2\n    else:\n        assert a\n    {continuation}\n"
                );
                assert_matches_cpython(&source, "nested_assignment_join.py", &LEVELS);
            }
        }
    }
}

#[test]
fn elided_else_preserves_the_original_legacy_scope_footer() {
    for condition in ["b", "b and c", "not (b and c)"] {
        for body in ["assert b", "obj.action()"] {
            let source = format!(
                "def f(a, b, c, obj):\n    if a:\n        assert a\n    if {condition}:\n        {body}\n    else:\n        assert c\n"
            );
            assert_matches_cpython(&source, "elided_else_scope_footer.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_finally_and_loop_preserves_returns_before_removed_assertions() {
    for (index, source) in [
        "def f(a, b):\n    try:\n        action()\n    finally:\n        while a and b:\n            action()\n    assert a\n",
        "def f(a, b, c):\n    try:\n        value = action()\n    finally:\n        while a and b and c:\n            b = step()\n    assert a\n    assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("finally_and_assert_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminal_finally_loop_after_linear_cleanup_preserves_returns() {
    let source = "def f(state):\n    try:\n        value = work()\n        assert value\n    finally:\n        state.close()\n        deadline = clock() + 2\n        while state.active and clock() < deadline:\n            pause()\n    assert not state.active\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("        assert value\n", ""),
        source.replace(
            "        state.close()\n        deadline = clock() + 2\n",
            "",
        ),
        source.replace("    assert not state.active\n", "    consume(state)\n"),
        source.replace(
            "            pause()\n",
            "            pause()\n        finish()\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("finally_loop_cleanup_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn terminating_if_before_erased_assertions_preserves_the_legacy_footer() {
    for body in [
        "return None",
        "return obj",
        "raise ValueError",
        "value = obj.attr\n        return value",
    ] {
        let source = format!(
            "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        {body}\n    assert b\n"
        );
        assert_matches_cpython(&source, "terminating_if_erased_footer.py", &LEVELS);
    }
}

#[test]
fn erased_assertion_gaps_preserve_conditional_join_nops() {
    for gap in [
        "",
        "        assert flag\n",
        "        assert flag, describe()\n        assert (hidden := flag)\n",
    ] {
        for successor in ["work()", "value = flag", "del flag"] {
            let source = format!(
                "def f(flag):\n    try:\n        if flag:\n            assert flag\n        else:\n            assert not flag\n{gap}        {successor}\n    except ValueError:\n        recover()\n"
            );
            assert_matches_cpython(&source, "erased_assertion_join_gap.py", &LEVELS);
        }
    }
}

#[test]
fn comprehension_setups_preserve_repeated_legacy_jump_locations() {
    for (case, source) in [
        "def f(items, a, b):\n    for item in items:\n        [x for x in item]\n        if a:\n            assert a\n        if b:\n            assert b\n",
        "def f(items, a, b):\n    value = {x for x in items}\n    for item in items:\n        if a:\n            assert a\n        if b:\n            assert b\n",
        "def f(items, a, b):\n    if a:\n        assert a\n    if b:\n        assert b\n    else:\n        [x for x in items]\n    a+a\n",
        "def f(items, a, b):\n    for item in items:\n        [[y for y in x] for x in items]\n        if a:\n            assert a\n        if b:\n            assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("comprehension_repeated_jump_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn manager_setups_preserve_repeated_legacy_empty_branch_locations() {
    for (case, source) in [
        "def f(a, b, c, obj):\n    with obj:\n        value = obj.action()\n    if a:\n        assert a\n    if b:\n        assert b\n    else:\n        assert c\n    obj.action()\n",
        "def f(a, b, obj):\n    with obj:\n        if a:\n            assert a\n        if b:\n            assert b\n        else:\n            assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("manager_repeated_empty_branch_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn direct_finally_async_for_keeps_exit_before_erased_assertions() {
    for (index, source) in [
        "async def f(items, obj):\n    try:\n        async for item in items:\n            pass\n        assert items\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in items:\n            obj.accept(item)\n        assert items\n    finally:\n        await obj.finish()\n",
        "async def f(items, obj):\n    try:\n        token = prepare()\n        obj.begin()\n        items = adapt(items)\n        async for item in items:\n            pass\n        assert items\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in items:\n            pass\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in adapt(\n            items\n        ):\n            pass\n        assert items\n    finally:\n        obj.finish()\n",
        "async def f(items, obj):\n    try:\n        async for item in items:\n            pass\n        assert items\n        async for again in items:\n            obj.accept(again)\n        assert items\n    finally:\n        obj.finish()\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("direct_finally_async_for_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_else_preserves_final_elif_try_exit_locations() {
    for condition in [
        "second(value)",
        "not second(value)",
        "all(valid(item) for item in items)",
    ] {
        for value in ["transform(items)", "[transform(item) for item in items]"] {
            let source = format!(
                "def f(items, value):\n    try:\n        if first(value):\n            result = transform(items)\n        elif {condition}:\n            result = {value}\n        else:\n            assert valid(value)\n    except ValueError as error:\n        raise TypeError(value) from error\n    consume(result)\n    return result\n"
            );
            assert_matches_cpython(&source, "erased_elif_try_exit.py", &LEVELS);
        }
    }
}

#[test]
fn erased_try_else_keeps_condition_location_on_retained_exit() {
    for (case, source) in [
        "def f(obj, flag):\n    try:\n        if flag:\n            obj.action()\n        else:\n            assert flag\n    except ValueError:\n        return False\n    if flag:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if not a:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a or b:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a == b:\n            value = obj.value\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        action()\n        if a:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a:\n            obj.action()\n        else:\n            assert a\n    except ValueError as exc:\n        recover(exc)\n    if a:\n        assert obj\n    return True\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a:\n            obj.action()\n        else:\n            assert a\n    except ValueError:\n        return False\n    action()\n    return obj\n",
        "def f(a,b,obj,cm,items):\n    try:\n        if a:\n            obj.action()\n        else:\n            assert a, describe()\n    except ValueError:\n        return False\n    if a:\n        assert obj\n    return True\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("erased_try_else_exit_{case}.py"), &LEVELS);
    }
}

#[test]
fn legacy_coroutine_conditional_work_keeps_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b, items):\n    if a:\n        if b:\n            value = [x for x in items]\n        assert a\n",
        "async def f(a, b, items):\n    if a:\n        return\n    else:\n        if b:\n            value = {x for x in items}\n        assert a\n",
        "async def f(a, b, items):\n    await action()\n    if a:\n        if b:\n            value = {x: a for x in items}\n        assert a\n",
        "async def f(a, b, obj):\n    if a:\n        if b:\n            value = [x for x in (a, b)]\n        assert a\n",
        "async def f(a, b, obj):\n    if a:\n        if b:\n            if obj:\n                action()\n        assert a\n",
        "async def f(a, b, obj, items):\n    if a:\n        if b:\n            value = [x for x in items]\n            if obj:\n                action(value)\n            value = [x for x in items]\n        assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("legacy_coroutine_conditional_work_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}
#[test]
fn terminating_legacy_branches_preserve_the_original_scope_footer() {
    for (index, tail) in [
        "raise ValueError",
        "return obj",
        "return 1",
        "return",
        "value = obj.attr",
    ]
    .into_iter()
    .enumerate()
    {
        let source =
            format!("def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        {tail}\n");
        assert_matches_cpython(
            &source,
            &format!("original_legacy_scope_footer_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn elided_assertions_preserve_legacy_nested_call_footers() {
    for condition in ["c", "c and a"] {
        for call in ["obj.action()", "obj.action(*items)"] {
            let source = format!(
                "def f(a, b, c, obj, items):\n    if a:\n        assert a\n    if b:\n        if {condition}:\n            {call}\n        assert b\n"
            );
            assert_matches_cpython(&source, "legacy_nested_call_footer.py", &LEVELS);
        }
    }
    assert_matches_cpython(
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        value = obj.attr\n        obj.before()\n        obj.action()\n",
        "legacy_linear_call_footer.py",
        &LEVELS,
    );
}

#[test]
fn empty_conditionals_preserve_legacy_terminal_return_locations() {
    for (case, source) in [
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        global marker\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        assert a\n    obj.action()\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        assert b\n    else:\n        value = obj.action()\n    return obj if b else a\n",
        "def f(a, b, obj):\n    if a:\n        pass\n    if b:\n        obj.action()\n",
        "def f(a, b, obj):\n    if a:\n        assert a\n    if b:\n        value = obj.action()\n",
        "def f(a, b, c, obj):\n    if a:\n        assert a\n    if b:\n        if c:\n            value = obj.attr\n",
        "def f(a, b, obj):\n    if b:\n        obj.action()\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("empty_condition_terminal_return_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn consecutive_elided_conditions_preserve_empty_branch_locations() {
    for (prefix, suffix) in [
        (
            "    if a:\n        assert a\n",
            "    if c:\n        assert c\n",
        ),
        ("", "    if c:\n        assert c\n"),
        ("    if a:\n        assert a\n", "    action()\n"),
    ] {
        let source = format!(
            "def f(a, b, c):\n{prefix}    if b:\n        assert b\n    else:\n        assert c\n{suffix}"
        );
        assert_matches_cpython(&source, "consecutive_elided_conditions.py", &LEVELS);
    }
}

#[test]
fn protected_coroutine_branches_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b):\n    try:\n        if a:\n            assert a\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a, b):\n    try:\n        if a:\n            pass\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a, b):\n    try:\n        if a:\n            action()\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a, b):\n    try:\n        if a:\n            value = b\n        assert b\n    except ValueError:\n        pass\n",
        "async def f(a):\n    try:\n        value = await action()\n        if a is not None:\n            assert value\n        assert a\n    except ValueError:\n        if a is not None:\n            assert a\n        else:\n            raise\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("protected_coroutine_elided_branch_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn legacy_protected_and_coroutine_branches_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "def f(a):\n    try:\n        if a:\n            pass\n        assert a\n    except:\n        raise\n",
        "def f(a):\n    try:\n        if a:\n            action()\n        assert a\n    except ValueError:\n        recover()\n        raise\n",
        "async def f(a, b):\n    if a:\n        if b:\n            pass\n        assert a\n",
        "async def f(a, b):\n    if a:\n        if b:\n            action()\n        assert a\n",
        "async def f(a, b):\n    await action()\n    if a:\n        if b:\n            value = a\n        assert a\n",
        "async def f(a):\n    try:\n        await action()\n        if a:\n            pass\n        assert a\n    except ValueError:\n        raise\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("legacy_elided_branch_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn coroutine_nested_branches_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b):\n    if a:\n        if b:\n            pass\n        assert a\n",
        "async def f(a, b):\n    if a:\n        if b:\n            value = action()\n        assert a\n",
        "async def f(a, b):\n    if a:\n        assert a\n    else:\n        if b:\n            action()\n        assert b\n",
        "async def f(a, b):\n    await action()\n    if a:\n        if b:\n            value = a\n        assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("coroutine_nested_return_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn coroutine_linear_tails_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "async def f(a, b, c):\n    if a:\n        return\n    else:\n        if b:\n            if c:\n                action()\n            action()\n        assert b\n",
        "async def f(a, b, items):\n    if a:\n        if b:\n            value = [item for item in items]\n        assert a\n",
        "async def f(a, b, c, items):\n    if a:\n        return\n    else:\n        if b:\n            value = [item for item in items]\n            if c:\n                action()\n            value = a + b\n        assert b\n",
        "async def f(a, b, c, manager):\n    async with manager:\n        await action()\n    if a:\n        return\n    else:\n        if b:\n            if c:\n                action()\n            action()\n        assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("coroutine_linear_tail_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn removed_assertions_preserve_constant_loop_break_borrow_boundary() {
    for (index, body) in [
        "value = obj.next()\nif value:\n    break\nassert value\n",
        "value = obj.next()\naction()\nif value in (1, 2):\n    break\nassert value\n",
        "value = obj.next()\nif value:\n    break\nassert value\nassert obj\n",
    ]
    .into_iter()
    .enumerate()
    {
        let mut source = String::from("def f(obj):\n    while True:\n");
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("    obj.close()\n");
        assert_matches_cpython(&source, &format!("constant_loop_break_{index}.py"), &LEVELS);
    }
}

#[test]
fn empty_try_before_removed_assertions_preserves_implicit_return() {
    for (case, source) in [
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        pass\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n        assert flag, describe()\n    except AssertionError as error:\n        pass\n    assert flag\n    assert flag\n",
        "def f(flag):\n    \"description\"\n    action()\n    try: assert flag\n    except: pass\n    assert flag; assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        recover()\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        return 1\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        pass\n    assert flag\n    action()\n",
        "def f(flag):\n    try:\n        assert flag\n        pass\n    except AssertionError:\n        pass\n    assert flag\n",
        "def f(flag):\n    try:\n        assert flag\n    except AssertionError:\n        pass\n    assert flag\n    pass\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("empty_try_assert_tail_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_terminal_boolean_preserves_shared_return_order() {
    for test in [
        "a or b",
        "a == 1 or b == 2",
        "a or b or c",
        "a and b",
        "a and (b or c)",
        "not a and (b or c)",
        "(a or b) and (c or d)",
        "not a and (not b or c)",
        "not a.ready and (b() or c.ready)",
    ] {
        let source = format!("def f(a, b, c, d):\n    if {test}:\n        assert a\n");
        assert_matches_cpython(&source, "elided_terminal_boolean.py", &LEVELS);
    }
    for (body, suffix) in [
        ("pass", ""),
        ("consume(a)", ""),
        ("assert a\n        consume(a)", ""),
        ("assert a", "    consume(a)\n"),
    ] {
        let source =
            format!("def f(a, b, c):\n    if not a and (b or c):\n        {body}\n{suffix}");
        assert_matches_cpython(&source, "terminal_boolean_live_body.py", &LEVELS);
    }
}

#[test]
fn terminal_conditional_name_assignments_preserve_join() {
    for scope in ["nonlocal", "local", "global", "attribute"] {
        for guard in ["compound", "simple", "none"] {
            for following in [false, true] {
                let mut source =
                    String::from("def sample(holder):\n    state = None\n    def assign(value):\n");
                match scope {
                    "local" => source.push_str("        state = None\n"),
                    "nonlocal" | "global" => {
                        writeln!(source, "        {scope} state").unwrap();
                    }
                    _ => {}
                }
                let target = if scope == "attribute" {
                    "holder.state"
                } else {
                    "state"
                };
                let indent = match guard {
                    "compound" => {
                        source.push_str("        if value is not None and value.ready:\n");
                        "            "
                    }
                    "simple" => {
                        source.push_str("        if value:\n");
                        "            "
                    }
                    _ => "        ",
                };
                writeln!(
                    source,
                    "{indent}{target} = value if {target} is None else {target}.combine(value)"
                )
                .unwrap();
                if following {
                    writeln!(source, "        consume({target})").unwrap();
                }
                source.push_str("    return state\n");
                assert_matches_cpython(
                    &source,
                    &format!("terminal_store_{scope}_{guard}_{following}.py"),
                    &LEVELS,
                );
            }
        }
    }
    for (index, source) in [
        "def f(self, x, flag, other, left, right):\n    if flag:\n        value = 1 if x else 2\n    else:\n        use()\n",
        "def f(self, x, flag, other, left, right):\n    if flag:\n        use()\n    elif other:\n        value = left if x else right\n    else:\n        done()\n",
        "def target(enabled, choose, left, right):\n    if enabled:\n        result = left if choose else right\n    else:\n        fallback()\n",
        "def target(outer, inner, choose, left, right):\n    if outer:\n        fallback()\n    else:\n        if inner:\n            result = left if choose else right\n        else:\n            finish()\n",
    ]
    .iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("terminal_store_sibling_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_try_arms_preserve_large_tuple_return_join() {
    for returned in [
        "value,",
        "value, owner",
        "1, 2",
        "1, 2, 3",
        "value, owner, first",
        "value, owner, first, {\"first\": first, \"second\": second}",
    ] {
        let source = format!(
            "def f(flag, first, second):\n    owner = acquire()\n    try:\n        value = owner.value\n        if flag:\n            assert ready(value)\n        else:\n            assert valid(value)\n        assert complete(value)\n        return {returned}\n    except Exception:\n        owner.close()\n        raise\n"
        );
        assert_matches_cpython(&source, "erased_try_tuple_return.py", &LEVELS);
    }
    assert_matches_cpython(
        "def f(flag, first, second):\n    if flag:\n        assert first\n    else:\n        assert second\n    assert flag\n    return first, second\n",
        "erased_unprotected_tuple_return.py",
        &LEVELS,
    );
}

#[test]
fn erased_assertions_preserve_try_body_joins_and_false_edge_locations() {
    for function in ["def", "async def"] {
        for alternate in [
            "chosen = converted",
            "if value:\n                return converted",
            "if value:\n                raise Problem(value)",
            "if value:\n                chosen = converted\n            else:\n                chosen = value",
        ] {
            let source = format!(
                "{function} f(value):\n    try:\n        converted = convert(value)\n        if predicate(converted):\n            chosen = converted[0]\n        else:\n            {alternate}\n        assert chosen\n    except Exception as error:\n        raise Problem(value) from error\n    consume(chosen)\n"
            );
            assert_matches_cpython(&source, "erased_try_body_join.py", &LEVELS);
        }
    }
}

#[test]
fn elided_try_guards_keep_python313_false_edge_return_locations() {
    for (index, source) in [
        "def f(a):\n    try:\n        if a:\n            return\n        assert False\n    except:\n        pass\n",
        "def f(a):\n    try:\n        if a:\n            action()\n            raise ValueError\n        assert False\n    except ValueError:\n        raise\n",
        "async def f(a):\n    try:\n        if a:\n            return await action()\n        assert False\n    except ValueError:\n        raise\n",
        "async def f(a, b):\n    try:\n        if a:\n            return await first()\n        elif b:\n            value = await second()\n            consume(value)\n            return value\n        assert False\n    except ValueError:\n        raise\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("python313_elided_try_guard_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn elided_coroutine_try_guards_preserve_false_edge_return_locations() {
    const SOURCE: &str = "async def f(a, b, obj):\n    try:\n        if a is not None:\n            return await obj.first(a)\n        elif b is not None:\n            value = await obj.second(b)\n            obj.check(value)\n            return obj.wrap(value)\n        assert a or b\n    except Error as exc:\n        raise ValueError from exc\n    except Exception as exc:\n        obj.log(exc)\n        raise ValueError from exc\n";
    for (index, source) in [
        SOURCE.to_owned(),
        SOURCE.replace("await ", ""),
        SOURCE.replace("return obj.wrap(value)", "raise ValueError(value)"),
        SOURCE
            .replace("return await obj.first(a)", "raise ValueError(a)")
            .replace("return obj.wrap(value)", "raise TypeError(value)"),
        SOURCE.replace("    try:\n", "    try:\n        if obj.ready:\n            obj.prepare()\n"),
        SOURCE.replace("        assert a or b\n", "        assert a or b\n        assert obj\n"),
        "async def f(a, obj):\n    try:\n        if a is not None:\n            return await obj.first(a)\n        assert a\n    except Error:\n        raise\n".to_owned(),
        SOURCE.replace("        assert a or b\n", ""),
        SOURCE.replace("        assert a or b\n", "        assert a or b\n        obj.finish()\n"),
        SOURCE.replace("        assert a or b\n", "        else:\n            return None\n        assert a or b\n"),
        SOURCE.replace("async def", "def").replace("await ", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("elided_coroutine_try_guard_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_protected_elif_preserves_loop_condition_owner() {
    for (function, manager) in [
        ("def", "with"),
        ("async def", "with"),
        ("async def", "async with"),
    ] {
        let source = format!(
            "{function} f(cm, items, a, b):
    {manager} cm:
        for item in items:
            if a:
                assert item
            elif b:
                assert items
            else:
                assert a
"
        );
        assert_matches_cpython(&source, "elided_protected_elif.py", &LEVELS);
    }
}

#[test]
fn with_conditional_exit_before_removed_assertion() {
    for (index, body) in [
        "if flag:\n    return 1\nassert flag\n",
        "if flag:\n    raise ValueError\nassert flag\n",
        "if flag is None:\n    return 1\nassert flag\nassert other\n",
        "if flag:\n    return 1\nelif other:\n    return 2\nassert flag\n",
        "if flag:\n    return 1\nelse:\n    return 2\nassert flag\n",
        "work()\nif flag:\n    return 1\nassert flag\n",
        "if flag:\n    work()\nassert flag\n",
    ]
    .into_iter()
    .enumerate()
    {
        let mut source = String::from("def f(cm, flag, other, work):\n    with cm:\n");
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        assert_matches_cpython(
            &source,
            &format!("with_conditional_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_assignment_exits_survive_removed_assertions() {
    for (index, source) in [
        "def f(a,b,c):\n    if a:\n        value=1\n    else:\n        if b:\n            if c:\n                value=2\n            else:\n                value=3\n        else:\n            value=4\n        assert value\n",
        "def f(a,b,c):\n    if a:\n        value = choose(1)\n        assert value\n    else:\n        value = choose(2)\n        if b:\n            value = choose(3)\n            if a == 1:\n                value = choose(4)\n            elif a == 2:\n                value = choose(5)\n            else:\n                value = choose(6)\n        else:\n            value = choose(7)\n        assert value\n",
        "def f(a,b,c):\n    value = choose(0)\n    assert value\n    if a:\n        value = 1\n        assert value\n    else:\n        value = 2\n        if b and c:\n            value = 3\n            if a == 1:\n                value = 4\n            elif a == 2:\n                value = 5\n            else:\n                value = 6\n        else:\n            value = 7\n        assert value\n",
        "def f(a,b,c,obj,items):\n    if a:\n        value=1\n    else:\n        if b:\n            if c: value=2\n            else: value=3\n        else:\n            value=4\n        assert value\n",
        "def f(a,b,c):\n    if a:\n        value=1\n    else:\n        if b:\n            if c:\n                value=2\n            else:\n                value=3\n        else:\n            value=4\n",
        "def f(a,b):\n    if a:\n        if b:\n            value=1\n        else:\n            value=2\n    else:\n        value=3\n    assert value\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("nested_assignment_exit_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_linear_branch_preserves_returns_before_removed_assertions() {
    for (index, leaf) in [
        "value = action()",
        "obj.value = action()",
        "value, other = action()",
        "action()",
        "b += 1",
        "assert b",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def f(a, b, obj):\n    if a:\n        if b:\n            {leaf}\n        assert a\n"
        );
        assert_matches_cpython(
            &source,
            &format!("nested_linear_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_branch_returns_survive_removed_inner_assertions() {
    for (index, source) in [
        "def f(a, b):\n    if a:\n        if b:\n            pass\n            assert b\n        assert a\n",
        "def f(a, b):\n    if a:\n        if b:\n            value = action()\n            assert b\n        assert a\n",
        "def f(a, b, c):\n    if a:\n        if b:\n            if c:\n                action()\n            action()\n        assert a\n",
        "def f(a, b, c):\n    if a:\n        action()\n    else:\n        if b:\n            if c:\n                value = b\n            value = action()\n        assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("nested_branch_assert_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn nested_authored_pass_preserves_returns_before_removed_assertions() {
    for (index, source) in [
        "def f(a, b, c):\n    if a:\n        if b:\n            if c:\n                pass\n        assert a\n",
        "def f(a, b, c):\n    if a:\n        pass\n    else:\n        if b:\n            if c:\n                pass\n        assert a\n        assert b\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("nested_authored_pass_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn copied_exits_fold_sequence_arithmetic() {
    for (left, right) in [
        ("'first'", "'second'"),
        ("b'first'", "b'second'"),
        ("(1, 2)", "(3, 4)"),
        ("'\\ud800'", "'text'"),
    ] {
        for expression in [
            format!("({left} if flag else {right}) + {left}"),
            format!("({left} if flag else {right}) * 4"),
            format!("4 * ({left} if flag else {right})"),
            format!("({left} if flag else {right}) + {left} + {right}"),
            format!("({left} if flag else {right}) * 4097"),
            format!("({left} if flag else {right}) * False"),
            format!("({left} if flag else {right}) * -1"),
        ] {
            let source = format!("def sample(flag):\n    return {expression}\n");
            assert_matches_cpython(&source, "copied_sequence_arithmetic.py", &LEVELS);
        }
    }
}

#[test]
fn copied_sequence_folds_preserve_locations_and_constant_order() {
    for (left, right) in [
        ("'first'", "'second'"),
        ("b'first'", "b'second'"),
        ("(1, 2)", "(3, 4)"),
    ] {
        for expression in [
            format!("(\n        ({left} if flag else {right})\n        + {left}\n    )"),
            format!("(\n        ({left} if flag else {right}) *\n        4\n    )"),
            format!("(({left} if flag else {right}) + {left},)"),
        ] {
            let source = format!(
                "def sample(flag, early):\n    if early:\n        return {expression}\n    return {left}, {right}\n"
            );
            assert_matches_cpython(&source, "copied_sequence_locations.py", &LEVELS);
        }
    }
}

#[test]
fn copied_empty_bytes_folds_preserve_cache_admission() {
    for prefix in ["", "empty = b''\n", "def earlier():\n    return None\n"] {
        for expression in [
            "(b'first' if flag else b'second') * False",
            "(b'' if flag else b'first') * -1",
            "(b'first' if flag else value) * 0",
        ] {
            let source = format!(
                "{prefix}def sample(flag, value):\n    return {expression}\n\ndef later():\n    return b''\n"
            );
            assert_matches_cpython(&source, "copied_empty_bytes.py", &LEVELS);
        }
    }
}

#[test]
fn protected_conditional_tuple_returns_revisit_completed_tail() {
    let source = r"
async def run(record, select, ready):
    try:
        item = record
        flag = ready
        if select:
            flag = not flag
        choice = flag
        return (item.field, 17 if choice else 23) if select else (29, 31)
    except Exception:
        return (37, 41)
";
    let unprotected = r"
async def run(record, select, ready):
    item = record
    flag = ready
    if select:
        flag = not flag
    choice = flag
    return (item.field, 17 if choice else 23) if select else (29, 31)
";
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def"),
        unprotected.to_owned(),
        source.replace("17 if choice else 23", "17"),
        source.replace(" if select else (29, 31)", ""),
        source.replace(
            "(item.field, 17 if choice else 23) if select else (29, 31)",
            "(17 if choice else 23) if select else 29",
        ),
        source.replace(
            "(item.field, 17 if choice else 23) if select else (29, 31)",
            "(17 if choice else 23,) if select else (29,)",
        ),
        source.replace(
            "(item.field, 17 if choice else 23) if select else (29, 31)",
            "(item.left, item.right, 17 if choice else 23) if select else (29, 31, 43)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("conditional_tuple_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_loop_branches_preserve_terminal_returns() {
    let source = r"
def run(data, items, consume, flag):
    if 101 in data:
        for key in (103, 107, 109, 113, 127):
            assert key not in data
        assert data[101] == 0
    assert data is not None
    assert 131 in data.fields
";
    let nested = r"
def run(data, items, consume, flag):
    if flag:
        consume(data)
    else:
        if 101 in data:
            for key in (103, 107, 109, 113, 127):
                assert key not in data
            assert data[101] == 0
        assert data is not None
        assert 131 in data.fields
";
    let suffix = "        assert data is not None\n        assert 131 in data.fields\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    assert 131 in data.fields\n", ""),
        source.replace(
            "    assert data is not None\n    assert 131 in data.fields\n",
            "",
        ),
        source.replace(
            "    assert data is not None\n    assert 131 in data.fields\n",
            "    consume(data)\n",
        ),
        nested.to_owned(),
        nested.replace(suffix, ""),
        nested.replace(suffix, "        pass\n"),
        nested.replace("(103, 107, 109, 113, 127)", "items"),
        nested.replace("(103, 107, 109, 113, 127)", "items.values()"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_loop_terminal_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn terminal_else_preserves_nested_conditional_exits_before_removed_assertions() {
    for (index, leaf) in ["assert marker", "pass", "marker = first"]
        .into_iter()
        .enumerate()
    {
        let source = format!(
            "def f(first, second):
    marker = setup()
    if first is None:
        assert marker
    else:
        if second is None:
            {leaf}
        assert marker
"
        );
        assert_matches_cpython(&source, &format!("terminal_else_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_assertion_preserves_conditional_break_backedge_donor() {
    let source = r#"def select(groups, expected):
    for group in groups:
        valid = True
        for field in group:
            if field.kind == "main":
                value = decode(field)
                if value != expected:
                    report(value)
                    valid = False
                    break
                assert value
            elif field.kind == "extra":
                if ready(field) and valid and expected:
                    details = decode(field)
                    if details not in known:
                        known[details] = {}
                    value = known[details]
        if valid:
            yield group
"#;
    for source in [
        source.to_owned(),
        source.replace("yield group", "consume(group)"),
        source.replace("                assert value\n", ""),
        source.replace(
            "if value != expected:",
            "if value != expected or invalid(field):",
        ),
        source.replace(
            "if value != expected:",
            "if (\n                    value\n                    != expected\n                ):",
        ),
        source.replace(
            "                    report(value)\n                    valid = False\n",
            "",
        ),
        source.replace("assert value", "consume(value)"),
    ] {
        assert_matches_cpython(&source, "conditional_break_backedge_donor.py", &LEVELS);
    }
}

#[test]
fn erased_assertion_preserves_try_break_exit_donor() {
    let source = "async def f(manager):\n    async with manager:\n        while True:\n            try:\n                await work()\n                value = await read()\n                state = value[\"state\"]\n                if state in (\"ready\", \"failed\"):\n                    break\n                assert state in (\"waiting\", \"busy\")\n            except Error:\n                recover()\n        await finish()\n    return True\n";
    for source in [
        source.to_owned(),
        source.replace(
            "                assert state in (\"waiting\", \"busy\")\n",
            "",
        ),
        source.replace("                recover()", "                pass"),
    ] {
        assert_matches_cpython(&source, "try_break_exit_donor.py", &LEVELS);
    }
}

#[test]
fn nested_try_else_break_preserves_borrowed_tail() {
    for (index, (nested, conditional, else_assignment, work_before_break)) in [
        (true, true, true, false),
        (true, true, true, true),
        (true, true, false, false),
        (true, false, false, false),
        (false, true, true, false),
    ]
    .into_iter()
    .enumerate()
    {
        let success_prefix = if work_before_break {
            "    work(result)\n    work(value)\n"
        } else {
            ""
        };
        let branch = if nested {
            format!(
                "try:\n    result = work(result)\nexcept LookupError:\n    result = 2\nelse:\n{success_prefix}    break"
            )
        } else {
            "break".to_owned()
        };
        let normal = if conditional {
            let fallback = if else_assignment {
                "\nelse:\n    result = 3"
            } else {
                ""
            };
            format!(
                "if result:\n    {}{fallback}",
                branch.replace('\n', "\n    ")
            )
        } else {
            branch
        };
        let source = format!(
            "def sample(values, emit, work):\n    for value in values:\n        try:\n            result = work(value)\n        except ValueError:\n            result = 0\n        else:\n            {}\n        emit(value, result)\n",
            normal.replace('\n', "\n            ")
        );
        assert_matches_cpython(
            &source,
            &format!("nested_try_else_break_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_managed_while_tail_preserves_borrow_boundary() {
    let source = r"def collect(manager, receiver, work, ready):
    with manager() as left, manager() as right:
        item = work()
        assert ready()
        other = work()
        while True:
            values = right.poll()
            if values and values[0]:
                break
            assert item.ready()
            assert ready()
        with manager():
            item.finish()
        other.finish()
        assert right.ready()
        assert right.empty()
    receiver.finish()
";
    for source in [
        source.to_owned(),
        source.replace("if values and values[0]:", "if values:"),
        source.replace(
            "        while True:\n            values = right.poll()\n            if values and values[0]:\n                break\n            assert item.ready()\n            assert ready()\n",
            "",
        ),
        source.replace(
            "with manager() as left, manager() as right:",
            "with manager() as right:",
        ),
        source.replace(
            "        assert right.empty()\n",
            "        assert right.empty()\n        right.finish()\n",
        ),
        source.replace(
            "        with manager():\n            item.finish()\n",
            "        item.finish()\n",
        ),
        source.replace("            assert ready()", "            ready()"),
        source.replace("while True:", "while ready():"),
    ] {
        assert_matches_cpython(&source, "erased_managed_while_tail.py", &LEVELS);
    }
}

#[test]
fn erased_inner_while_tail_preserves_borrow_boundary() {
    for (index, (outer, assertion_prefix, tail, guard)) in [
        (true, true, "assert value", "True"),
        (false, true, "assert value", "True"),
        (true, false, "assert value", "True"),
        (false, false, "assert value", "True"),
        (true, true, "work(value)", "True"),
        (true, false, "assert value", "receiver.ready(value)"),
    ]
    .into_iter()
    .enumerate()
    {
        let prefix = if assertion_prefix {
            "    assert attempt\n"
        } else {
            ""
        };
        let body = format!(
            "while {guard}:\n    value = work(value)\n    attempt += 1\n{prefix}    ready = receiver.ready(value)\n    if ready:\n        break\n    {tail}\nif receiver.ready(value):\n    assert value\n"
        );
        let body = if outer {
            format!("for item in items:\n    {}", body.replace('\n', "\n    "))
        } else {
            body
        };
        let source = format!(
            "def sample(value, items, work, receiver):\n    attempt = 0\n    {}",
            body.replace('\n', "\n    ")
        );
        assert_matches_cpython(&source, &format!("erased_inner_while_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_protected_while_tail_preserves_borrow_boundary() {
    let source = r"def sample(items, state, limit):
    try:
        while True:
            value = update(items)
            for key, item in pairs(items):
                consume(item)
            values = [transform(item) for item in items]
            publish({'values': [transform(item) for item in items]}, ready=True)
            state['value'] = value
            if value > limit and ready(items):
                break
            assert value < limit
        for item in items:
            finish(item)
            assert item
    finally:
        release(items)
    observe(items)
    return value
";
    for (index, source) in [
        source.to_owned(),
        source.replace("if value > limit and ready(items):", "if value > limit:"),
        source.replace("            for key, item in pairs(items):\n                consume(item)\n", ""),
        source.replace("            values = [transform(item) for item in items]\n", ""),
        source.replace("            publish({'values': [transform(item) for item in items]}, ready=True)\n", ""),
        source.replace("            assert value < limit", "            check(value)"),
        source.replace("while True:", "while ready(items):"),
        source.replace("        release(items)", "        for item in items:\n            if active(item):\n                release(item)"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("erased_protected_while_tail_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_terminal_loop_break_locations() {
    for (case, source) in [
        "def f(flag, items):\n    if flag:\n        for item in items:\n            if item:\n                break\n        assert flag\n",
        "def f(flag, items):\n    if flag:\n        action()\n    else:\n        for item in items:\n            if item:\n                break\n        assert flag\n        assert items\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("terminal_loop_break_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_linear_assignment_return_boundary() {
    let source = "def check(first, second):\n    if first:\n        if second:\n            left()\n        else:\n            right()\n        value = \"text\"\n        assert value\n    else:\n        other()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("value = \"text\"", "value = second"),
        source.replace("value = \"text\"", "value = choose()"),
        source.replace("value = \"text\"", "value = \"left\" if second else \"right\""),
        source.replace("value = \"text\"", "value = first or second"),
        source.replace("        assert value\n", ""),
        source.replace("        assert value\n", "        return\n"),
        "def check(first, second):\n    if first:\n        other()\n    else:\n        if second:\n            left()\n        else:\n            right()\n        value = \"text\"\n        assert value\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(&source, &format!("linear_assignment_assert_{index}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_conditional_assignment_return_boundary() {
    for (case, source) in [
        "def f(flag, value):\n    if value is not None:\n        result = 1 if flag else 2\n        assert result\n",
        "def f(flag, value):\n    if value is None:\n        consume()\n    else:\n        result = 1 if flag else 2\n        assert result\n        assert flag\n",
        "def f(obj, flag, value):\n    if value is not None:\n        obj.result = 1 if flag else 2\n        assert obj.result\n    else:\n        obj.result = None\n",
        "def f(obj, flag, value):\n    if value is not None:\n        obj[0] = 1 if flag else 2\n        assert flag\n",
        "def f(flag, value):\n    if value is None:\n        consume()\n    elif flag:\n        result = 1 if value else 2\n        assert result\n    else:\n        done()\n",
        "def f(flag):\n    result = 1 if flag else 2\n    assert result\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("conditional_assignment_assert_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertions_preserve_unpacking_return_locations() {
    for locals in [0, 16] {
        for targets in [
            "first, second",
            "first, second, third",
            "first, second, third, fourth",
        ] {
            for brackets in [false, true] {
                let mut source = String::from("def f(obj):\n");
                for index in 0..locals {
                    writeln!(source, "    temporary_{index} = {index}").unwrap();
                }
                let values = targets
                    .split(", ")
                    .map(|name| format!("obj.{name}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let target = if brackets {
                    format!("[{targets}]")
                } else {
                    targets.to_owned()
                };
                writeln!(source, "    {target} = {values}\n    assert first").unwrap();
                assert_matches_cpython(&source, "unpacking_before_assert.py", &LEVELS);
            }
        }
    }
}

#[test]
fn finally_guarded_loop_recovery_preserves_borrow_boundary() {
    const GUARDED: &str = "while True:\n    if flag:\n        result = 2\n        break\n    result = take()\n    if result:\n        break";
    const RUNTIME: &str = "while flag:\n    if flag:\n        result = 2\n        break\n    result = take()\n    if result:\n        break";
    const SINGLE: &str = "while True:\n    result = take()\n    if flag:\n        break";
    const IMMEDIATE: &str = "while True:\n    result = take()\n    break";
    const UNBROKEN: &str = "while True:\n    result = take()\n    if items:\n        marker = 4\n        note(marker)\n    for item in items:\n        note(item)\n    note(result)";
    let conditional = format!(
        "{GUARDED}\n    if items:\n        marker = 4\n        note(marker)\n    note(result)"
    );
    let iterator =
        format!("{GUARDED}\n    for item in items:\n        note(item)\n    note(result)");
    let combined = format!(
        "{GUARDED}\n    if items:\n        marker = 4\n        note(marker)\n    for item in items:\n        note(item)\n    note(result)"
    );
    let runtime = combined.replacen("while True:", "while flag:", 1);
    let runtime_live_tail = format!("{RUNTIME}\n    note(result)");
    let single = format!(
        "{SINGLE}\n    if items:\n        marker = 4\n        note(marker)\n    for item in items:\n        note(item)\n    note(result)"
    );
    for (index, (body, initial, handler, suffix)) in [
        (GUARDED, "result = 1", Some("result = 3"), "note(result)"),
        (GUARDED, "result = 1", Some("pass"), "note(result)"),
        (GUARDED, "", Some("result = 3"), "note(result)"),
        (RUNTIME, "result = 1", Some("result = 3"), "note(result)"),
        (SINGLE, "result = 1", Some("result = 3"), "note(result)"),
        (IMMEDIATE, "result = 1", Some("result = 3"), "note(result)"),
        (GUARDED, "result = 1", None, "note(result)"),
        (
            "result = take()",
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
        (GUARDED, "result = 1", Some("result = 3"), ""),
        (
            GUARDED,
            "result = 1",
            Some("result = 3"),
            "note(result)\nnote(flag)",
        ),
        (
            conditional.as_str(),
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
        (
            iterator.as_str(),
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
        (
            combined.as_str(),
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
        (
            runtime.as_str(),
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
        (
            single.as_str(),
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
        (combined.as_str(), "result = 1", None, "note(result)"),
        (UNBROKEN, "result = 1", Some("result = 3"), "note(result)"),
        (
            runtime_live_tail.as_str(),
            "result = 1",
            Some("result = 3"),
            "note(result)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut source = String::from(
            "def run(flag, take, note, items):\n    result = 0\n    try:\n        note(flag)\n    finally:\n",
        );
        for line in initial.lines() {
            writeln!(source, "        {line}").unwrap();
        }
        if let Some(handler) = handler {
            source.push_str("        try:\n");
            for line in body.lines() {
                writeln!(source, "            {line}").unwrap();
            }
            writeln!(source, "        except LookupError:\n            {handler}").unwrap();
        } else {
            for line in body.lines() {
                writeln!(source, "        {line}").unwrap();
            }
        }
        for line in suffix.lines() {
            writeln!(source, "        {line}").unwrap();
        }
        source.push_str("    if result:\n        return 1\n    return 0\n");
        assert_matches_cpython(
            &source,
            &format!("finally_guarded_loop_recovery_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn while_else_keeps_finally_borrowing_after_elided_assertions() {
    for condition in ["condition()", "condition() or count < 3"] {
        for following in ["", "    return obj\n"] {
            let source = format!(
                "def f(obj, count):
    try:
        while {condition}:
            action()
        else:
            recover()
        assert obj
    except ValueError:
        recover()
    finally:
        obj.close()
{following}"
            );
            assert_matches_cpython_versions(
                &source,
                "while_else_finally_borrow.py",
                &LEVELS,
                &[
                    PythonVersion::Py313,
                    PythonVersion::Py314,
                    PythonVersion::Py315,
                ],
            );
        }
    }
}

#[test]
fn terminal_chained_comparisons_preserve_empty_branch_exits() {
    for (index, body) in [
        "assert value",
        "global declared",
        "global declared\n        assert value",
    ]
    .into_iter()
    .enumerate()
    {
        for (name, prefix, keyword, suffix) in [
            ("plain", "", "if", ""),
            ("else", "", "if", "    else:\n        raise ValueError\n"),
            (
                "elif",
                "    if value is None:\n        raise ValueError\n",
                "elif",
                "    else:\n        raise ValueError\n",
            ),
        ] {
            let source = format!(
                "def f(value):\n{prefix}    {keyword} 0 < value < 3:\n        {body}\n{suffix}"
            );
            assert_matches_cpython(
                &source,
                &format!("empty_comparison_{name}_{index}.py"),
                &LEVELS,
            );
        }
    }
    assert_matches_cpython(
        "def f(low, value, high, other):
    if low < value < high:
        global state
    else:
        if low < other < high:
            global seen
",
        "nested_empty_comparison.py",
        &LEVELS,
    );
}

#[test]
fn elided_assertions_preserve_protected_while_exit_location() {
    for (case, condition) in ["condition()", "condition() or count < 3"]
        .into_iter()
        .enumerate()
    {
        let source = format!(
            "def f(obj, count):\n    try:\n        while {condition}:\n            action()\n        assert obj\n    except ValueError:\n        recover()\n    finally:\n        obj.close()\n"
        );
        assert_matches_cpython(&source, &format!("protected_while_exit_{case}.py"), &LEVELS);
    }
}

#[test]
fn elided_assertion_after_simple_if_preserves_finally_borrow_boundary() {
    for (case, branches) in [
        "",
        "        else:\n            pass\n",
        "        elif other():\n            recover()\n",
        "        elif other():\n            recover()\n        else:\n            pass\n",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "def f(obj, count):
    try:
        if condition():
            action()
{branches}        assert obj
    except ValueError:
        recover()
    finally:
        obj.close()
"
        );
        assert_matches_cpython(
            &source,
            &format!("simple_if_finally_borrow_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn discarded_imports_preserve_none_admission_order() {
    for (index, import) in [
        "import package",
        "import package.module as alias",
        "import package, other",
        "from package import item",
    ]
    .into_iter()
    .enumerate()
    {
        for (scope, prefix, indent) in [
            ("module", "", ""),
            ("function", "def f():\n", "    "),
            ("class", "class Outer:\n", "    "),
        ] {
            let source = format!(
                "{prefix}{indent}if False:\n{indent}    {import}\n\
                 {indent}class Later:\n{indent}    pass\n"
            );
            assert_matches_cpython(
                &source,
                &format!("discarded_import_{index}_{scope}.py"),
                &LEVELS,
            );
        }
    }
}

#[test]
fn dotted_import_parser_interning_matches_cpython() {
    for (case, import) in [
        "if False:\n    from SercOuter.SercInner import item\n",
        "if False:\n    from SercOuter.SercInner.Leaf import item\n",
        "seed = None\nif False:\n    from SercOuter.SercInner.Leaf import *\n",
        "seed = None\nif False:\n    import SercOuter.SercInner.Leaf\n",
        "from SercOuter.SercInner import item\n",
        "if condition:\n    from SercOuter.SercInner import item\n",
        "if False:\n    from SercOuter import item\n",
        "if False:\n    from SercOuter.SercInnerExtra.Leaf import item\n",
        "from SercOuter.SercInner.Leaf import item\nsaved = {'SercOuter.SercInner', 'other'}\n",
        "if False:\n    from SercOuter.SercInner.Leaf import item\nfirst = ('SercOuter.SercInner', ('SercOuter.SercInner',))\nsecond = ('SercOuter.' + 'SercInner', ('SercOuter.SercInner',))\n",
        "def select(flag):\n    try:\n        if True:\n            if flag:\n                return 1\n        else:\n            from SercOuter.SercInner.Leaf import item\n    except Exception:\n        return None\n",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!("{import}class SercOuter:\n    class SercInner:\n        pass\n");
        assert_matches_cpython(&source, &format!("dotted_import_{case}.py"), &LEVELS);
    }
    for (case, import) in [
        "from A.B.C.Leaf import item",
        "from A.B.C.Leaf import *",
        "import A.B.C.Leaf",
    ]
    .into_iter()
    .enumerate()
    {
        let source = format!(
            "seed = None\nif False:\n    {import}\nclass A:\n    class B:\n        class C:\n            pass\n"
        );
        assert_matches_cpython(&source, &format!("deep_dotted_import_{case}.py"), &LEVELS);
    }
    let source = "if False:\n    from \u{fb00}.SercInner.Leaf import item\nclass ff:\n    class SercInner:\n        pass\n";
    assert_matches_cpython(source, "dotted_import_nfkc.py", &LEVELS);
    let source = "if False:\n    from SercOuter.__SercInner.Leaf import item\nclass SercOuter:\n    class __SercInner:\n        pass\n";
    assert_matches_cpython(source, "dotted_import_private.py", &LEVELS);
}

#[test]
fn discarded_star_imports_match_cpython() {
    for (case, source) in [
        "if False:\n    from package import *\nvalue = 1\n",
        "if False:\n    from package.child import *\nvalue = 1\n",
        "if False:\n    from . import *\nvalue = 1\n",
        "if False:\n    from . import name\nvalue = 1\n",
        "while False:\n    from package import *\nvalue = 1\n",
        "if True:\n    value = 1\nelse:\n    from package import *\n",
        "try:\n    if False:\n        from package.child import *\n    else:\n        value = 1\nexcept Exception:\n    value = 2\n",
        "try:\n    if True:\n        value = 1\n    else:\n        from . import name\nexcept Exception:\n    value = 2\n",
        "if enabled:\n    from package import *\nvalue = 1\n",
        "from package import *\nvalue = 1\n",
        "if False:\n    from package import name\nvalue = 1\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("star_import_{case}.py"), &LEVELS);
    }
}

#[test]
fn unprotected_conditional_returns_fuse_local_pairs_after_handlers() {
    for handler in [
        "except Exception:\n        recover()",
        "except Exception as error:\n        recover(error)",
    ] {
        for condition in ["current is not None", "current"] {
            for tail in [">= threshold", "+ threshold"] {
                let source = format!(
                    "def sample(current, threshold):\n    try:\n        prepare()\n    {handler}\n    return (current if {condition} else clock()) {tail}\n"
                );
                assert_matches_cpython(&source, "unprotected_conditional_return.py", &LEVELS);
            }
        }
    }
    for source in [
        "def sample(current, threshold):\n    try:\n        return current >= threshold\n    except Exception:\n        return False\n",
        "def sample(current, threshold):\n    try:\n        selected = current if current is not None else clock()\n    except Exception:\n        return False\n    return selected >= threshold\n",
        "def sample(current, threshold):\n    with manager():\n        return (current if current is not None else clock()) >= threshold\n",
    ] {
        assert_matches_cpython(source, "protected_local_pair_boundary.py", &LEVELS);
    }
}

#[test]
fn named_handler_loop_cleanup_after_elided_assertions_matches_cpython() {
    for (index, suffix) in [
        "assert error\n",
        "assert error\n        assert items\n",
        "pass\n        assert error\n",
        "assert error\n        pass\n",
        "pass\n",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        for definition in ["def", "async def"] {
            let source = format!(
                "{definition} f(items):
    try:
        action()
    except ValueError as error:
        for item in items:
            if item:
                break
        {suffix}"
            );
            assert_matches_cpython(&source, &format!("named_handler_loop_{index}.py"), &LEVELS);
        }
    }
}

#[test]
fn exceptional_finally_return_unwinds_managers_in_order() {
    const FINALLY: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            with inner:
                try:
                    return work()
                finally:
                    cleanup()
";
    const HANDLER: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            with inner:
                try:
                    return work()
                except ValueError:
                    return 'handled'
";
    const LOOP_OUTSIDE: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            for x in (1,):
                with inner:
                    return work()
";
    const LOOP_INSIDE: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            with inner:
                for x in (1,):
                    return work()
";
    const MANAGER_IN_HANDLER: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            try:
                work()
            except ValueError:
                with inner:
                    return work()
";
    const MANAGER_IN_HANDLER_IN_LOOP: &str = r"def f(outer, inner, work, cleanup):
    with outer:
        try:
            work()
        finally:
            for x in (1,):
                try:
                    work()
                except ValueError:
                    with inner:
                        return work()
";
    const TRACE: &str = r"
import itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
class WorkFault(Exception): pass
class CleanupFault(Exception): pass
class ExitFault(Exception): pass
def chain(error):
    seen, result = set(), []
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        result.append(type(error).__name__)
        error = error.__context__
    return result
rows = []
for first, second, cleanup_raises, inner_mode, outer_mode in itertools.product(
        (False, True), ('none', 'work', 'value'), (False, True),
        ('normal', 'suppress', 'raise'), ('normal', 'suppress', 'raise')):
    events = []
    class Manager:
        def __init__(self, name, mode):
            self.name, self.mode = name, mode
        def __enter__(self):
            events.append(('enter', self.name))
            return self
        def __exit__(self, typ, val, tb):
            events.append(('exit', self.name, typ.__name__ if typ else None, chain(val)))
            if self.mode == 'raise':
                raise ExitFault(self.name)
            return self.mode == 'suppress'
    calls = [0]
    def work():
        calls[0] += 1
        events.append(('work', calls[0]))
        if calls[0] == 1 and first:
            raise WorkFault('first')
        if calls[0] == 2 and second == 'work':
            raise WorkFault('second')
        if calls[0] == 2 and second == 'value':
            raise ValueError('second')
        return calls[0]
    def cleanup():
        events.append(('cleanup',))
        if cleanup_raises:
            raise CleanupFault()
    try:
        outcome = ('return', scope['f'](Manager('outer', outer_mode),
                                        Manager('inner', inner_mode), work, cleanup))
    except BaseException as error:
        outcome = ('raise', chain(error))
    rows.append((events, outcome))
print(json.dumps(rows))
";

    for source in [
        FINALLY,
        HANDLER,
        LOOP_OUTSIDE,
        LOOP_INSIDE,
        MANAGER_IN_HANDLER,
        MANAGER_IN_HANDLER_IN_LOOP,
    ] {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] = compile_both(
                    source,
                    "return_finally.py",
                    python_version,
                    optimization_level,
                );
                assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
                assert_outputs_match(
                    probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn exceptional_finally_return_inside_older_handler() {
    const GENERATOR: &str = r"def f(manager, work):
    try:
        yield 'ready'
    except ValueError:
        try:
            work()
        finally:
            with manager:
                return 7

def ordinary(manager, work):
    try:
        raise ValueError('trigger')
    except ValueError:
        try:
            work()
        finally:
            with manager:
                return 7
";
    const ASYNC: &str = r"async def f(outer, inner, work):
    async with outer:
        try:
            await work()
        except ValueError:
            try:
                await work()
            finally:
                async with inner:
                    return 7
";
    const GENERATOR_TRACE: &str = r"
import itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
def chain(error):
    seen, result = set(), []
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        result.append(type(error).__name__)
        error = error.__context__
    return result
rows = []
for action, work_fails, mode in itertools.product(
        ('resume', 'throw', 'close', 'ordinary'), (False, True),
        ('normal', 'suppress', 'raise')):
    events = []
    class Manager:
        def __enter__(self):
            events.append(('enter', type(sys.exception()).__name__))
            return self
        def __exit__(self, typ, val, tb):
            events.append(('exit', typ.__name__ if typ else None, chain(val)))
            if mode == 'raise':
                raise RuntimeError('exit')
            return mode == 'suppress'
    def work():
        events.append(('work', type(sys.exception()).__name__))
        if work_fails:
            raise ValueError('work')
    try:
        if action == 'ordinary':
            value = scope['ordinary'](Manager(), work)
        else:
            generator = scope['f'](Manager(), work)
            events.append(('first', next(generator)))
            if action == 'throw':
                value = generator.throw(ValueError('trigger'))
            elif action == 'close':
                value = generator.close()
            else:
                value = next(generator)
        outcome = ('value', value)
    except StopIteration as error:
        outcome = ('return', error.value, chain(error))
    except BaseException as error:
        outcome = ('raise', chain(error))
    rows.append((events, outcome))
print(json.dumps(rows))
";
    const ASYNC_TRACE: &str = r"
import asyncio, itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
def chain(error):
    seen, result = set(), []
    while error is not None and id(error) not in seen:
        seen.add(id(error))
        result.append(type(error).__name__)
        error = error.__context__
    return result
rows = []
for failures, action, mode in itertools.product(
        (1, 2), ('resume', 'throw', 'cancel', 'close'), ('normal', 'suppress', 'raise')):
    events, steps, calls = [], [], [0]
    class Pause:
        def __await__(self):
            yield 'suspended'
    class Manager:
        def __init__(self, name):
            self.name = name
        async def __aenter__(self):
            events.append(('enter', self.name))
            return self
        async def __aexit__(self, typ, val, tb):
            events.append(('exit', self.name, typ.__name__ if typ else None, chain(val)))
            if self.name == 'inner':
                await Pause()
                if mode == 'raise':
                    raise RuntimeError('exit')
            return self.name == 'inner' and mode == 'suppress'
    async def work():
        calls[0] += 1
        events.append(('work', calls[0]))
        if calls[0] <= failures:
            raise ValueError('work')
    coroutine = scope['f'](Manager('outer'), Manager('inner'), work)
    try:
        first = coroutine.send(None)
        steps.append(first)
        if first != 'suspended':
            raise RuntimeError('inner exit did not suspend')
        if action == 'throw':
            value = coroutine.throw(RuntimeError('throw'))
        elif action == 'cancel':
            value = coroutine.throw(asyncio.CancelledError('cancel'))
        elif action == 'close':
            value = coroutine.close()
        else:
            value = coroutine.send(None)
        outcome = ('value', value)
    except StopIteration as error:
        outcome = ('return', error.value)
    except BaseException as error:
        outcome = ('raise', chain(error))
    rows.append((events, steps, outcome))
print(json.dumps(rows))
";

    for (source, trace) in [(GENERATOR, GENERATOR_TRACE), (ASYNC, ASYNC_TRACE)] {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] = compile_both(
                    source,
                    "return_older_handler.py",
                    python_version,
                    optimization_level,
                );
                assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
                assert_outputs_match(
                    probe(index).args(["-I", "-S", "-B", "-c", trace]),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn optimized_footer_keeps_protected_manager_pass_exception_owner_on_313() {
    const SOURCE: &str = r"def f(manager, x, y):
    try:
        with manager:
            pass
            work()
    except Exception:
        recover()
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import dis, marshal, sys
events, hits = [], []
class Manager:
    def __enter__(self): events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + (kind.__name__ if kind else 'None'))
        return False
scope = {'work': lambda: events.append('work'), 'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and
           (i.positions.lineno, i.positions.end_lineno, i.positions.col_offset, i.positions.end_col_offset) == (4, 4, 12, 16)]
if len(targets) != 1: raise RuntimeError(('manager pass marker count', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'manager-pass-optimized-footer')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise ValueError('injected')
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    for x, y in [(False, False), (True, True)]:
        events.clear()
        hits.clear()
        result = f(Manager(), x, y)
        if hits != [target] or events != ['enter', 'exit:ValueError', 'recover'] or result is not None:
            raise RuntimeError((x, y, hits, events, result))
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert_eq!(actual, native, "{optimization_level:?}");
        assert_runs(
            probe(1)
                .args(["-I", "-S", "-B", "-c", TRACE])
                .stdout(Stdio::piped()),
            [&native, &actual],
        );
    }
}

#[test]
fn optimized_footer_keeps_split_manager_pass_exception_owner_on_313() {
    const SOURCE: &str = r"def f(manager, x, y):
    try:
        with manager:
            pass
            try:
                work()
            finally:
                cleanup()
    except Exception:
        recover()
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import dis, marshal, sys
events, hits = [], []
class Manager:
    def __enter__(self): events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + (kind.__name__ if kind else 'None'))
        return False
scope = {'work': lambda: events.append('work'), 'cleanup': lambda: events.append('cleanup'), 'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and
           (i.positions.lineno, i.positions.end_lineno, i.positions.col_offset, i.positions.end_col_offset) == (4, 4, 12, 16)]
if len(targets) != 1: raise RuntimeError(('manager pass marker count', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'manager-pass-optimized-footer')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise ValueError('injected')
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    for x, y in [(False, False), (True, True)]:
        events.clear()
        hits.clear()
        result = f(Manager(), x, y)
        if hits != [target] or events != ['enter', 'exit:ValueError', 'recover'] or result is not None:
            raise RuntimeError((x, y, hits, events, result))
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
events.clear()
result = f(Manager(), False, False)
if events != ['enter', 'work', 'cleanup', 'exit:None'] or result is not None:
    raise RuntimeError((events, result))
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert_eq!(actual, native, "{optimization_level:?}");
        assert_runs(
            probe(1)
                .args(["-I", "-S", "-B", "-c", TRACE])
                .stdout(Stdio::piped()),
            [&native, &actual],
        );
    }
}

#[test]
fn optimized_footer_after_legacy_manager_finally_preserves_jump_owner() {
    const SOURCE: &str = r"def f(manager, work, x, y):
    try:
        raise ValueError()
    except ValueError:
        with manager:
            pass
            try:
                work()
            finally:
                pass
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'JUMP_FORWARD' and i.positions.lineno == 10]
if len(targets) != 1:
    raise RuntimeError(('missing finalizer jump', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-manager-finally-footer')
try:
    for error_type in (RuntimeError, StopIteration):
        for suppress in (False, True):
            for x, y in ((False, False), (True, True)):
                events, hits = [], []
                class Manager:
                    def __enter__(self): events.append('enter')
                    def __exit__(self, typ, value, traceback):
                        events.append(('exit', typ))
                        return suppress
                def work(): events.append('work')
                def inject(code, offset):
                    if offset == target:
                        hits.append(offset)
                        raise error_type('injected')
                monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
                monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
                try:
                    result = f(Manager(), work, x, y)
                    outcome = None
                except BaseException as error:
                    result, outcome = None, type(error)
                finally:
                    monitor.set_local_events(5, f.__code__, 0)
                    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
                expected = None if suppress else error_type
                if hits != [target] or events != ['enter', 'work', ('exit', error_type)] or outcome is not expected or result is not None:
                    raise RuntimeError((hits, events, outcome, result, expected))
finally:
    monitor.free_tool_id(5)
";
    for (index, python_version) in [(1, PythonVersion::Py313)] {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "manager_finally_footer.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            assert_runs(
                probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn optimized_footer_after_exceptional_manager_return_unwind() {
    const SOURCE: &str = r"def f(outer, inner, work, cleanup, events, take, x, y):
    with outer:
        try:
            work()
        finally:
            with inner:
                try:
                    if take:
                        return work()
                finally:
                    cleanup()
    events.append('footer')
    if x:
        if y:
            pass
        assert y
";
    const TRACE: &str = r"
import itertools, json, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
rows, covered = [], set()
class ExitFault(Exception): pass
for fault, take, suppress, pair, inner_raises, outer_raises in itertools.product((False, True), repeat=6):
    events, calls = [], [0]
    class Manager:
        def __init__(self, name): self.name = name
        def __enter__(self):
            events.append(('enter', self.name))
            return self
        def __exit__(self, typ, value, traceback):
            events.append(('exit', self.name, typ.__name__ if typ else None))
            if (self.name == 'inner' and inner_raises) or (self.name == 'outer' and outer_raises):
                events.append(('raise-exit', self.name))
                raise ExitFault(self.name)
            return suppress and self.name == 'outer'
    def work():
        calls[0] += 1
        events.append(('work', calls[0]))
        if fault and calls[0] == 1:
            raise ValueError('work')
        return 10 + calls[0]
    def cleanup(): events.append(('cleanup',))
    try:
        result = scope['f'](Manager('outer'), Manager('inner'), work, cleanup,
                            events, take, pair, pair)
        outcome = ('return', result)
    except BaseException as error:
        outcome = ('raise', type(error).__name__)
    if take and not inner_raises and not outer_raises and outcome != ('return', 12):
        raise RuntimeError(('return path not reached', events, outcome))
    if not take and not inner_raises and not outer_raises and (not fault or suppress) and 'footer' not in events:
        raise RuntimeError(('footer not reached', events, outcome))
    if ('enter', 'inner') not in events or ('cleanup',) not in events:
        raise RuntimeError(('finalizer not reached', events, outcome))
    if inner_raises and ('raise-exit', 'inner') not in events:
        raise RuntimeError(('inner exit not reached', events, outcome))
    if outer_raises and ('raise-exit', 'outer') not in events:
        raise RuntimeError(('outer exit not reached', events, outcome))
    if 'footer' in events: covered.add('footer')
    if take and outcome == ('return', 12): covered.add('return')
    if fault and suppress and 'footer' in events: covered.add('suppressed')
    if ('raise-exit', 'inner') in events: covered.add('inner_exit_fault')
    if ('raise-exit', 'outer') in events: covered.add('outer_exit_fault')
    rows.append((events, outcome))
if covered != {'footer', 'return', 'suppressed', 'inner_exit_fault', 'outer_exit_fault'}:
    raise RuntimeError(('missing coverage', covered))
print(json.dumps(rows))
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "return_manager_footer.py",
            PythonVersion::Py313,
            optimization_level,
        );
        assert_eq!(actual, native, "{optimization_level:?}");
        assert_outputs_match(
            probe(1).args(["-I", "-S", "-B", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn legacy_nested_finally_pass_keeps_outer_owner() {
    const SOURCE: &str = "def f(work, cleanup, after):\n    try:\n        try:\n            work()\n        finally:\n            pass\n        after()\n    finally:\n        cleanup()\n";
    const TERMINAL: &str = "def f(work, cleanup):\n    try:\n        try:\n            work()\n        finally:\n            pass\n    finally:\n        cleanup()\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [instruction.offset for instruction in dis.get_instructions(f)
           if instruction.opname == 'NOP' and instruction.positions.lineno == 6]
if not targets:
    raise RuntimeError('missing pass marker')
target = min(targets)
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-finally-owner')
for exception in (ValueError, StopIteration):
    for cleanup_raises in (False, True):
        events, hits = [], []
        def work():
            events.append('work')
        def after():
            events.append('after')
        def cleanup():
            events.append('cleanup')
            if cleanup_raises:
                raise RuntimeError('cleanup')
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise exception('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            f(work, cleanup, after)
            result = None
        except BaseException as error:
            result = type(error)
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = RuntimeError if cleanup_raises else exception
        if hits != [target] or events != ['work', 'cleanup'] or result is not expected:
            raise RuntimeError((hits, events, result, expected))
monitor.free_tool_id(5)
";

    let versions = [PythonVersion::Py312, PythonVersion::Py313];
    for source in [
        TERMINAL,
        r"def f(work,cleanup,flag):
    try:
        try:
            work()
        finally:
            pass
        global marker
        assert flag
    finally:
        cleanup()
",
        r"def outer():
    marker = 0
    def f(work,cleanup,flag):
        try:
            try:
                work()
            finally:
                pass
            nonlocal marker
            1
        finally:
            cleanup()
    return f
",
    ] {
        assert_matches_cpython_versions(source, "legacy_finally.py", &LEVELS, &versions);
    }
    for (index, python_version) in versions.into_iter().enumerate() {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "legacy_finally.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            assert_runs(
                probe(index)
                    .args(["-I", "-S", "-B", "-c", TRACE])
                    .stdout(Stdio::piped()),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn legacy_normal_finalizer_pass_keeps_outer_handler() {
    const TRACE: &str = r"
import dis, marshal, sys

events = []
def work():
    events.append('work')
def after():
    events.append('after')
def recover():
    events.append('recover')
scope = {'work': work, 'after': after, 'recover': recover}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [instruction.offset for instruction in dis.get_instructions(f)
           if instruction.opname == 'NOP' and instruction.positions.lineno == 10]
if not targets:
    raise RuntimeError('missing pass marker')
target = min(targets)
generator, has_sibling = (argument == '1' for argument in sys.argv[1:])
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-normal-finalizer')
for fault in (ValueError, StopIteration):
    for flag in (False, True):
        events.clear()
        hits = []
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise fault('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            value = f(None, flag)
            if generator:
                value = next(value)
            result = ('value', value)
        except BaseException as error:
            result = ('error', type(error))
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = ('value', 1) if has_sibling else (
            'error', RuntimeError if generator and fault is StopIteration else fault)
        expected_events = ['work', 'recover'] if has_sibling else ['work']
        if hits != [target] or events != expected_events or result != expected:
            raise RuntimeError((hits, events, result, expected))
monitor.free_tool_id(5)
";
    for generator in [false, true] {
        for has_sibling in [false, true] {
            let sibling = if has_sibling { "        after()\n" } else { "" };
            let exit = if generator { "yield 1" } else { "return 1" };
            let source = format!(
                "def f(manager,flag):\n    try:\n        try:\n            work()\n        finally:\n            try:\n                if flag:\n                    pass\n            finally:\n                pass\n{sibling}    except Exception:\n        recover()\n    {exit}\n"
            );
            for optimization_level in LEVELS {
                let [native, actual] = compile_both(
                    &source,
                    "legacy_normal_finalizer.py",
                    PythonVersion::Py312,
                    optimization_level,
                );
                assert_eq!(
                    actual, native,
                    "{optimization_level:?}, {generator}, {has_sibling}"
                );
                assert_runs(
                    probe(0)
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .args([
                            if generator { "1" } else { "0" },
                            if has_sibling { "1" } else { "0" },
                        ])
                        .stdout(Stdio::piped()),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn legacy_generator_terminal_finally_keeps_enclosing_owner() {
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Manager:
    def __enter__(self):
        events.append('enter')
        return self
    def __exit__(self, typ, value, traceback):
        events.append(typ.__name__ if typ else None)
        return True
def recover():
    events.append('recover')
scope = {'recover': recover}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
line = int(sys.argv[1])
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname in ('NOP', 'JUMP_FORWARD') and i.positions.lineno == line]
if not targets:
    raise RuntimeError('missing finalizer exit')
target = min(targets)
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-finally-owner')
for fault in (ValueError, StopIteration):
    for flag in (False, True):
        events.clear()
        hits = []
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise fault('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            result = next(f(Manager(), flag))
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        if hits != [target] or events != ['enter', 'AssertionError', 'recover'] or result != 1:
            raise RuntimeError((hits, events, result))
monitor.free_tool_id(5)
";
    for nested in [false, true] {
        for generator in [false, true] {
            let finalbody = if nested {
                "            try:\n                if flag:\n                    pass\n            finally:\n                pass\n"
            } else {
                "            pass\n"
            };
            let exit = if generator { "yield 1" } else { "return 1" };
            for (body, trace) in [
                (
                    "with manager:\n                pass\n                assert False",
                    true,
                ),
                (
                    "with manager as value:\n                pass\n                assert False",
                    false,
                ),
                (
                    "with manager, manager:\n                pass\n                assert False",
                    false,
                ),
                ("with manager:\n                assert 0", false),
            ] {
                let source = format!(
                    "def f(manager,flag):\n    try:\n        try:\n            {body}\n        finally:\n{finalbody}    except Exception:\n        recover()\n    {exit}\n"
                );
                let marker = source
                    .lines()
                    .enumerate()
                    .filter_map(|(index, line)| (line.trim() == "pass").then_some(index + 1))
                    .last()
                    .unwrap();
                for (index, version) in [PythonVersion::Py312, PythonVersion::Py313]
                    .into_iter()
                    .enumerate()
                {
                    for level in LEVELS {
                        let [native, actual] =
                            compile_both(&source, "suppressed_assert.py", version, level);
                        assert_eq!(
                            actual, native,
                            "{version}, {level:?}, {nested}, {generator}"
                        );
                        if !trace
                            || version != PythonVersion::Py312
                            || level != OptimizationLevel::Zero
                            || !generator
                        {
                            continue;
                        }
                        assert_runs(
                            probe(index)
                                .args(["-I", "-S", "-B", "-c", TRACE, &marker.to_string()])
                                .stdout(Stdio::piped()),
                            [&native, &actual],
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn terminal_finally_pass_is_outside_its_cleanup_handler() {
    const SOURCE: &str = "def f(flag):\n    try:\n        def inner():\n            return flag\n        result = flag\n        pass\n    finally:\n        cleanup()\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Fault(Exception): pass
def cleanup(): events.append('cleanup')
scope = {'cleanup': cleanup}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
for opcode, line, expected in [('MAKE_FUNCTION', 3, ['cleanup', 'escaped']),
                               ('LOAD_DEREF', 5, ['cleanup', 'escaped']),
                               ('NOP', 6, ['escaped'])]:
    targets = [i.offset for i in dis.get_instructions(f)
               if i.opname == opcode and i.positions.lineno == line]
    assert len(targets) == 1, (opcode, line, targets)
    hits = []
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == targets[0]:
                hits.append(frame.f_lasti)
                raise Fault()
        return trace
    events.clear()
    sys.settrace(trace)
    try:
        f(True)
    except Fault:
        events.append('escaped')
    finally:
        sys.settrace(None)
    assert len(hits) == 1, (opcode, hits)
    assert events == expected, (opcode, events, expected)
print(json.dumps('ok'))
";
    for (index, (version, _, _)) in VERSIONS.iter().copied().enumerate().skip(2) {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "terminal_finally_pass.py",
                version,
                optimization_level,
            );
            let output = assert_outputs_match(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
            assert_eq!(output, b"\"ok\"\n");
        }
    }
}

#[test]
fn named_handler_setups_preserve_optimized_backedge_locations() {
    let source = "def f(items, first, second):\n    for item in items:\n        try:\n            work(item)\n        except ValueError as error:\n            recover(error)\n        if first:\n            assert item\n        if second:\n            assert item\n    return finish()\n";
    for source in [
        source.to_owned(),
        source.replace(
            "            recover(error)",
            "            try:\n                recover(error)\n            except TypeError as other:\n                consume(other)",
        ),
        source.replace(
            "        if first:",
            "        except TypeError as other:\n            recover(other)\n        if first:",
        ),
        source.replace(
            "        if first:",
            "        try:\n            value = convert(item)\n        except TypeError as other:\n            value = str(other)\n        if first:",
        ),
        source.replace("    return finish()", "        pass\n    return finish()"),
        source.replace(
            "    return finish()",
            "        consume(item)\n    return finish()",
        ),
    ] {
        assert_matches_cpython(&source, "named_handler_backedge.py", &LEVELS);
    }
}

#[test]
fn preceding_exception_setup_does_not_own_later_backedge() {
    const SOURCE: &str = "def f(xs, work, handler):\n    try:\n        work()\n    except ValueError:\n        handler()\n    for x in xs:\n        if x:\n            assert False\n        if x:\n            assert False\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
backedges = [i.offset for i in dis.get_instructions(f)
             if i.opname == 'JUMP_BACKWARD' and i.positions.lineno is None]
if len(backedges) != 1:
    raise RuntimeError(('backedges', backedges))
target = backedges[0]
if any(e.start <= target < e.end for e in dis.Bytecode(f).exception_entries):
    raise RuntimeError('earlier handler owns later backedge')
class Fault(Exception): pass
monitor = sys.monitoring
monitor.use_tool_id(5, 'backedge-owner')
for fail in (False, True):
    events, hits = [], []
    def work():
        events.append('work')
        if fail:
            raise ValueError()
    def handler():
        events.append('handler')
    def trace(code, offset):
        if offset == target:
            hits.append(offset)
            raise Fault()
    monitor.register_callback(5, monitor.events.INSTRUCTION, trace)
    monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
    try:
        try:
            f([True, True], work, handler)
            raise RuntimeError('backedge was not reached')
        except Fault:
            pass
    finally:
        monitor.set_local_events(5, f.__code__, 0)
        monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    expected = ['work', 'handler'] if fail else ['work']
    if events != expected or hits != [target]:
        raise RuntimeError((events, hits, expected))
monitor.free_tool_id(5)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] = compile_both(
            SOURCE,
            "preceding_exception_trace.py",
            PythonVersion::Py312,
            level,
        );
        assert_eq!(actual, native);
        assert_runs(
            probe(0).args(["-I", "-S", "-B", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn generator_backedge_after_exception_setup_has_no_line() {
    const SOURCE: &str = "def f(xs, work, recover):\n try:\n  work()\n except ValueError:\n  recover()\n for x in xs:\n  if x:\n   assert False\n  if x:\n   assert False\n yield 1\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'JUMP_BACKWARD' and i.positions.lineno is None]
if len(targets) != 1:
    raise RuntimeError(('backedges', targets))
target = targets[0]
class Fault(Exception): pass
monitor = sys.monitoring
monitor.use_tool_id(5, 'generator-backedge')
for fail in (False, True):
    for exception in (Fault, StopIteration):
        events, hits, traces = [], [], []
        def work():
            events.append('work')
            if fail:
                raise ValueError()
        def recover():
            events.append('recover')
        def inject(code, offset):
            if code is f.__code__ and offset == target:
                hits.append(offset)
                raise exception('injected')
        def tracer(frame, event, arg):
            if frame.f_code is f.__code__ and event in ('exception', 'return'):
                traces.append((event, frame.f_lineno))
            return tracer
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        sys.settrace(tracer)
        try:
            try:
                list(f([True, True], work, recover))
                raise RuntimeError('backedge was not reached')
            except Fault:
                if exception is not Fault:
                    raise
            except RuntimeError as e:
                if exception is not StopIteration or not isinstance(e.__cause__, StopIteration):
                    raise
        finally:
            sys.settrace(None)
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = ['work', 'recover'] if fail else ['work']
        if events != expected or hits != [target] or traces[-2:] != [('exception', None), ('return', None)]:
            raise RuntimeError((events, hits, traces, expected))
monitor.free_tool_id(5)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] =
            compile_both(SOURCE, "generator_backedge.py", PythonVersion::Py312, level);
        assert_eq!(actual, native);
        assert_runs(
            probe(0).args(["-I", "-S", "-B", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn folded_match_guard_preserves_original_nop() {
    let source = r#"class Example:
    @property
    def case(self):
        match self.state.data.kind:
            case Kind.FIRST if True:
                assert isinstance(self.state.data, First)
            case Kind.SECOND:
                work(self.state.data)
            case Kind.THIRD:
                work(self.state.data)
            case _:
                raise AssertionError(f"unknown: {self.state.data.kind}")
        return self.state.data
"#;
    for source in [
        source.to_owned(),
        source.replace("return self.state.data", "return (self, value)"),
        source.replace("return self.state.data", "return self.state.data.payload"),
        source.replace(
            "return self.state.data",
            "return (\n            self.state.data\n        )",
        ),
        source.replace("if True:", "if (\n                True\n            ):"),
        source.replace("if True:", "if valid(self):"),
        source.replace("assert isinstance(self.state.data, First)", "pass"),
        source.replace("work(self.state.data)", "assert self.state.data"),
    ] {
        assert_matches_cpython(&source, "folded_match_guard.py", &LEVELS);
    }
}

#[test]
fn erased_match_cases_inherit_copied_return_locations() {
    let source = r#"class Example:
    @property
    def case(self):
        match self.state.data.kind:
            case Kind.FIRST:
                assert isinstance(self.state.data, First)
            case Kind.SECOND:
                assert isinstance(self.state.data, Second)
            case Kind.THIRD:
                assert isinstance(self.state.data, Third)
            case _:
                raise AssertionError(f"unknown: {self.state.data.kind}")
        return self.state.data
"#;
    for source in [
        source.to_owned(),
        source.replace("return self.state.data", "return self.state"),
        source.replace("return self.state.data", "return self.state.data.payload"),
        source.replace("case Kind.FIRST:", "case None:"),
        source.replace("case Kind.FIRST:", "case [value]:"),
        source.replace("case Kind.FIRST:", "case Kind.FIRST if valid(self):"),
        source.replace("assert isinstance(self.state.data, First)", "pass"),
        source.replace(
            "        return self.state.data",
            "        work(self)\n        return self.state.data",
        ),
    ] {
        assert_matches_cpython(&source, "copied_match_return.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_else_preserves_nested_backedge_donor() {
    let source = r#"async def consume(records):
    done = False
    for record in records:
        if not record:
            continue
        if not done:
            record = record[2:]
            try:
                event = parse(record)
            except ValueError:
                continue
            if event.kind == "piece":
                accept(record)
            elif event.kind == "end":
                result = finish(record)
                done = True
                assert result.valid
            else:
                assert False
        else:
            assert record == "end"
"#;
    for source in [
        source.to_owned(),
        source.replace("async def", "def"),
        source.replace("                assert result.valid\n", ""),
        source.replace(
            "elif event.kind == \"end\":",
            "elif event.kind == \"end\" or ready(record):",
        ),
        source.replace(
            "elif event.kind == \"end\":",
            "elif (\n                event.kind\n                == \"end\"\n            ):",
        ),
        source.replace(
            "                assert False",
            "                reject(record)",
        ),
    ] {
        assert_matches_cpython(&source, "coroutine_erased_else_donor.py", &LEVELS);
    }
}

#[test]
fn elided_inner_else_in_for_else_matches_cpython() {
    for (prefix, finalbody) in [
        ("", "        finish()\n"),
        (
            "            if c:\n                break\n",
            "        finish()\n",
        ),
        (
            "            if c:\n                continue\n",
            "        finish()\n",
        ),
        (
            "",
            "        try:\n            finish()\n        finally:\n            cleanup()\n",
        ),
    ] {
        let source = format!(
            "def f(xs, a, b, c):\n    z = 0\n    for x in xs:\n        if a:\n{prefix}            if b:\n                z = x\n            else:\n                assert b\n        else:\n            work(x)\n    else:\n{finalbody}"
        );
        assert_matches_cpython_versions(
            &source,
            "elided_inner_else_for_else.py",
            &LEVELS,
            &VERSIONS.map(|(version, _, _)| version),
        );
    }
}

#[test]
fn elided_inner_else_for_else_preserves_backedge_trace() {
    const SOURCE: &str = "def f(xs, a, b, c):\n    z = 0\n    for x in xs:\n        if a:\n            if c:\n                break\n            if b:\n                z = x\n            else:\n                assert b\n        else:\n            work(x)\n    else:\n        finish()\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
events = []
def finish(): events.append('finish')
def work(x): events.append(('work', x))
scope.update(finish=finish, work=work)
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname.startswith('JUMP_BACKWARD') and i.positions.lineno == 7]
if len(targets) != 1:
    raise RuntimeError(('backedge', targets))
target = targets[0]
if any(e.start <= target < e.end for e in dis.Bytecode(f).exception_entries):
    raise RuntimeError('backedge unexpectedly has an exception owner')
class Fault(Exception): pass
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(target)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    try:
        f([1, 2], True, False, False)
        raise RuntimeError('injection missed')
    except Fault:
        pass
finally:
    sys.settrace(None)
if hits != [target] or events:
    raise RuntimeError((hits, events))
f([1, 2], True, False, True)
if events:
    raise RuntimeError(('break ran loop else', events))
f([1, 2], True, False, False)
if events != ['finish']:
    raise RuntimeError(('normal loop else', events))
";
    for python_version in [PythonVersion::Py314, PythonVersion::Py315] {
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == python_version)
            .unwrap();
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let [native, actual] = compile_both(
                SOURCE,
                "elided_inner_else_for_else.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native);
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn elided_assertions_preserve_terminal_branch_return_locations() {
    for (index, source) in [
        "def f(first, second, obj):\n    if first:\n        if second:\n            value = obj.first\n        else:\n            value = obj.second\n        assert value\n",
        "def f(first, second):\n    if first:\n        while second:\n            second = step()\n        assert second\n",
        "def f(first, second, obj):\n    if first:\n        work()\n    else:\n        if second:\n            value = obj.first\n        else:\n            value = obj.second\n        assert value\n",
        "def f(first, second):\n    if first:\n        work()\n    else:\n        while second:\n            second = step()\n        assert second\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("terminal_branch_return_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn terminating_loop_branch_before_elided_assert_matches_cpython() {
    for terminal in [
        "raise ValueError()",
        "raise ValueError() from flag",
        "return item",
        "return",
        "return item or flag",
        "return item if flag else items",
    ] {
        for otherwise in ["", "        else:\n            assert items\n"] {
            let source = format!(
                "def f(items, flag):
    for item in items:
        if flag:
            if item is None:
                {terminal}
            assert item
{otherwise}"
            );
            assert_matches_cpython(&source, "terminating_elided_loop_branch.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_if_before_elided_assert_keeps_manager_exception_owner_312() {
    const SOURCE: &str = "def f(x):\n if x:\n  with context():\n   call()\n else:\n  if x:\n   assert x\n  assert x\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + kind.__name__)
        return False
scope = {'context': Manager, 'call': lambda: events.append('body')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
target = next(i.offset for i in dis.get_instructions(f)
              if i.opname == 'LOAD_GLOBAL' and i.argval == 'call')
fired = [False]
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == target:
            fired[0] = True
            raise ValueError('injected')
    return trace
sys.settrace(trace)
try:
    f(False)
except AssertionError:
    pass
sys.settrace(None)
events.clear()
sys.settrace(trace)
try:
    f(True)
except ValueError:
    events.append('escaped')
finally:
    sys.settrace(None)
assert fired[0]
print(json.dumps(events))
";
    for optimization_level in LEVELS {
        let [native, actual] = compile_both(
            SOURCE,
            "terminal_elided_assert.py",
            PythonVersion::Py312,
            optimization_level,
        );
        let output =
            assert_outputs_match(probe(0).args(["-I", "-S", "-c", TRACE]), [&native, &actual]);
        assert_eq!(output, b"[\"enter\", \"exit:ValueError\", \"escaped\"]\n");
    }
}

#[test]
fn elided_loop_arms_inside_managers_match_cpython() {
    for test in [
        "item",
        "item == 1",
        "item is None",
        "item and flag",
        "item or flag",
    ] {
        for prefix in [
            "",
            "            first = setup()\n            second = setup()\n",
        ] {
            let source = format!(
                "def f(manager, items, flag):
    with manager:
        with manager:
{prefix}            for item in items:
                if {test}:
                    assert flag
                    assert item
                else:
                    assert flag
                    assert item
"
            );
            assert_matches_cpython(&source, "elided_manager_loop.py", &LEVELS);
        }
    }
}

#[test]
fn elided_manager_loop_arms_preserve_conditional_successor_nops() {
    for gap in ["", "            assert a\n"] {
        for successor in [
            "if b:\n                assert b\n            else:\n                assert a",
            "if b:\n                break",
            "if b and c:\n                work()",
        ] {
            let source = format!(
                "def f(cm, items, a, b, c):\n    with cm:\n        for item in items:\n            if a:\n                assert a\n            else:\n                assert b\n{gap}            {successor}\n"
            );
            assert_matches_cpython(&source, "elided_manager_conditional_successor.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_elided_loop_arms_inside_managers_match_cpython() {
    for manager in ["with cm", "async with cm"] {
        for condition in ["item == 1", "first or second"] {
            let source = format!(
                "async def f(cm, items, first, second):\n    {manager}:\n        for item in items:\n            if {condition}:\n                assert item\n            else:\n                assert items\n"
            );
            assert_matches_cpython(&source, "coroutine_elided_manager_loop.py", &LEVELS);
        }
    }
}

#[test]
fn optimized_elided_join_preserves_exception_owners() {
    const SOURCE: &str = r"
def sync(flag):
    with manager('outer'):
        with manager('inner'):
            if flag:
                assert flag
            else:
                assert not flag
            return 1

def terminal(flag):
    with manager('terminal'):
        if flag:
            assert flag
        else:
            assert not flag

async def async_terminal(flag):
    async with manager('async'):
        if flag:
            assert flag
        else:
            assert not flag

def same_line(flag):
    with manager('same-line'):
        if flag: assert flag
        else: assert not flag

def multiple(flag):
    with manager('left'), manager('right'):
        if flag:
            assert flag
        else:
            assert not flag

def gen(flag):
    with manager('generator'):
        if flag:
            assert flag
        else:
            assert not flag
        yield 1
";
    const TRACE_JOIN: &str = r"
import asyncio, dis, marshal, sys

events = []
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append((self.name, 'enter'))
    def __exit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False
    async def __aenter__(self):
        events.append((self.name, 'enter'))
    async def __aexit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False

scope = {'manager': Manager}
exec(marshal.loads(sys.stdin.buffer.read()), scope)

def trace_join(fn, injected):
    instructions = list(dis.get_instructions(fn))
    offsets = [b.offset for a, b in zip(instructions, instructions[1:])
               if a.opname == 'NOT_TAKEN' and b.opname == 'NOP']
    assert len(offsets) == 1, offsets
    def trace(frame, event, arg):
        if frame.f_code is not fn.__code__:
            return None
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == offsets[0] and injected:
            events.append('hit')
            raise injected('injected')
        return trace
    return trace

for name, injected, expected in [
    ('sync', ValueError, [('outer', 'enter'), ('inner', 'enter'), 'hit',
                          ('inner', 'exit', 'ValueError'), ('outer', 'exit', 'ValueError'),
                          ('caught', 'ValueError', None)]),
    ('terminal', ValueError, [('terminal', 'enter'), 'hit',
                              ('terminal', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('async_terminal', ValueError, [('async', 'enter'), 'hit',
                                    ('async', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('same_line', ValueError, [('same-line', 'enter'), 'hit',
                               ('same-line', 'exit', 'ValueError'), ('caught', 'ValueError', None)]),
    ('multiple', ValueError, [('left', 'enter'), ('right', 'enter'), 'hit',
                              ('right', 'exit', 'ValueError'), ('left', 'exit', 'ValueError'),
                              ('caught', 'ValueError', None)]),
    ('gen', StopIteration, [('generator', 'enter'), 'hit',
                            ('generator', 'exit', 'StopIteration'),
                            ('caught', 'RuntimeError', 'StopIteration')]),
]:
    fn = scope[name]
    def invoke():
        result = fn(True)
        if name == 'gen':
            return next(result)
        if name == 'async_terminal':
            return asyncio.run(result)
        return result
    # Initialize opcode tracing for this code object before injecting an exception.
    sys.settrace(trace_join(fn, None))
    try:
        invoke()
    finally:
        sys.settrace(None)
    events.clear()
    sys.settrace(trace_join(fn, injected))
    try:
        invoke()
    except BaseException as error:
        events.append(('caught', type(error).__name__,
                       type(error.__cause__).__name__ if error.__cause__ else None))
    finally:
        sys.settrace(None)
    assert events == expected, (events, expected)
";
    for (version, level) in [
        (PythonVersion::Py314, OptimizationLevel::One),
        (PythonVersion::Py314, OptimizationLevel::Two),
        (PythonVersion::Py315, OptimizationLevel::One),
        (PythonVersion::Py315, OptimizationLevel::Two),
    ] {
        let [native, actual] = compile_both(SOURCE, "optimized_join.py", version, level);
        assert_eq!(actual, native);
        let index = VERSIONS
            .iter()
            .position(|(target, _, _)| *target == version)
            .unwrap();
        assert_runs(
            probe(index)
                .args(["-I", "-S", "-X", "utf8", "-c", TRACE_JOIN])
                .stdout(Stdio::piped()),
            [&native, &actual],
        );
    }
}

#[test]
fn protected_optimized_conditional_keeps_nested_exception_owners() {
    const SOURCE: &str = r"events = []
class Fault(Exception): pass
class M:
 def __enter__(self): events.append('enter'); return self
 def __exit__(self, typ, value, tb): events.append('exit:' + (typ.__name__ if typ else 'none')); return False
def step(): events.append('step')
def other(): events.append('other')
def f(a):
 try:
  if a:
   with M():
    try:
     step()
    except KeyError:
     other()
  else:
   other()
  assert a
 except Fault:
  events.append('handler')
";
    const FOLDED_SOURCE: &str = r"events = []
class Fault(Exception): pass
class M:
 def __enter__(self): events.append('enter'); return self
 def __exit__(self, typ, value, tb): events.append('exit:' + (typ.__name__ if typ else 'none')); return False
def step(): events.append('step')
def other(): events.append('other')
def f(a):
 try:
  if True:
   if a:
    with M():
     try:
      step()
     except KeyError:
      other()
   else:
    other()
  else:
   other()
  assert a
 except Fault:
  events.append('handler')
";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.load(sys.stdin.buffer), scope)
f, fault, events = scope['f'], scope['Fault'], scope['events']
cases = [('POP_JUMP', int(sys.argv[1]), False, ['handler']),
         ('CALL', int(sys.argv[2]), True, ['enter', 'exit:Fault', 'handler'])]
for prefix, line, argument, expected in cases:
    matches = [instruction.offset for instruction in dis.get_instructions(f)
               if instruction.opname.startswith(prefix) and instruction.positions.lineno == line]
    if len(matches) != 1:
        raise RuntimeError((prefix, line, matches))
    target, hits = matches[0], []
    events.clear()
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == target:
                hits.append(target)
                raise fault('injected')
        return trace
    try:
        sys.settrace(trace)
        result = f(argument)
    finally:
        sys.settrace(None)
    if hits != [target] or result is not None or events != expected:
        raise RuntimeError((prefix, hits, result, events, expected))
";
    for source in [SOURCE, FOLDED_SOURCE] {
        let lines = ["if a:", "step()"].map(|needle| {
            (source
                .lines()
                .position(|line| line.trim() == needle)
                .unwrap()
                + 1)
            .to_string()
        });
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let [native, actual] = compile_both(source, "protected_owner.py", version, level);
                assert_eq!(actual, native, "{version}, {level:?}");
                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                assert_runs(
                    probe(index)
                        .args(["-I", "-S", "-B", "-c", TRACE])
                        .args(&lines)
                        .stdout(Stdio::piped()),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn folded_protected_nested_terminal_joins_match_cpython() {
    for (index, source) in [
        "def f(a, b):\n try:\n  if True:\n   if b:\n    work()\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if True:\n   if b:\n    work()\n   else:\n    other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if True:\n   while b:\n    break\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/folded_protected_nested_{index}.py"),
            &LEVELS[1..],
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn protected_elided_assertion_preserves_terminal_control_flow() {
    for (index, source) in [
        "def f(a, ys):\n try:\n  if not a:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(obj, ys):\n try:\n  if obj.flag:\n   x = [i for i in ys]\n  assert obj\n except ValueError:\n  work()\n",
        "def f(a, b, ys):\n try:\n  if a and b:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if bool(a):\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, obj, ys):\n try:\n  if a:\n   obj.value = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, obj, ys):\n try:\n  if a:\n   obj[0] = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   x, = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   x = [i for i in ys]\n  else:\n   x = []\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if a:\n   work()\n  elif b:\n   other()\n  else:\n   third()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if a == b:\n   work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   return 1\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if a:\n   if b:\n    work()\n  assert a\n  assert b\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if True:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if False:\n   work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if __debug__:\n   x = [i for i in ys]\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   with manager():\n    work()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   try:\n    work()\n   except TypeError:\n    other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if a:\n   try:\n    work()\n   finally:\n    other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, ys):\n try:\n  if a:\n   for y in ys:\n    work()\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/protected_terminal_elided_{index}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn folded_protected_branch_before_elided_assertion_keeps_exit_location() {
    let versions = VERSIONS.map(|(version, _, _)| version);
    for (index, source) in [
        "def f(a):\n try:\n  if True:\n   pass\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
        "def f(a, b):\n try:\n  if 1:\n   pass\n  else:\n   pass\n  assert a\n  assert b\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if True:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if False:\n   work()\n  else:\n   pass\n  assert a\n except ValueError:\n  work()\n",
        "def f(a):\n try:\n  if __debug__:\n   work()\n  else:\n   pass\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/folded_protected_exit_{index}.py"),
            &LEVELS,
            &versions,
        );
    }
    for (index, source) in [
        "def f(a):\n try:\n  try:\n   if True:\n    pass\n   else:\n    other()\n   assert a\n  except ValueError:\n   work()\n except TypeError:\n  other()\n",
        "def f(a, obj):\n with obj:\n  try:\n   if True:\n    pass\n   else:\n    other()\n   assert a\n  except ValueError:\n   work()\n",
        "def f(a, xs):\n for x in xs:\n  try:\n   if True:\n    pass\n   else:\n    other()\n   assert a\n  except ValueError:\n   work()\n",
        "def f(a):\n yield 1\n try:\n  if True:\n   pass\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
        "async def f(a):\n try:\n  if True:\n   pass\n  else:\n   other()\n  assert a\n except ValueError:\n  work()\n",
    ].into_iter().enumerate() {
        assert_matches_cpython_versions(
            source,
            &format!("sub/folded_protected_context_{index}.py"),
            &LEVELS[1..],
            &versions,
        );
    }
}

#[test]
fn folded_try_pass_after_branch_keeps_existing_exception_owner() {
    const TRACE: &str = r"
import dis, marshal, sys, types
sys.modules['synthetic_module'] = types.ModuleType('synthetic_module')
events = []
class Manager:
    def __enter__(self):
        events.append('enter')
        return self
    def __exit__(self, kind, value, traceback):
        events.append('exit:' + (kind.__name__ if kind else 'none'))
        return False
scope = {'recover': lambda: events.append('recover'), 'other': lambda: events.append('other')}
exec(marshal.load(sys.stdin.buffer), scope)
f = scope['outer']()
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == int(sys.argv[1])]
if len(targets) != 1:
    raise RuntimeError(('expected one Pass NOP', targets))
target, hits = targets[0], []
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise ValueError('injected')
monitor = sys.monitoring
monitor.use_tool_id(5, 'folded-pass-owner')
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    values = list(f(sys.argv[2] == 'true', Manager(), []))
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
if hits != [target] or values != [1] or events != ['enter', 'recover', 'exit:none']:
    raise RuntimeError((hits, values, events))
";
    for (body, target_line, argument) in [
        ("if a:\n      pass", "9", "true"),
        (
            "if a:\n      pass\n     else:\n      pass\n     pass",
            "11",
            "false",
        ),
        ("if a: pass\n     pass; pass", "8", "true"),
    ] {
        let source = format!(
            "from synthetic_module import *\ndef outer():\n marker = 1\n def f(a, manager, xs):\n  with manager:\n   try:\n    if True:\n     {body}\n    else:\n     other()\n    assert a\n   except ValueError:\n    recover()\n  for x in xs:\n   if x:\n    assert False\n   if x:\n    assert False\n  yield 1\n return f\n"
        );
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let [native, actual] =
                    compile_both(&source, "folded_pass_owner.py", version, level);
                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                assert_runs(
                    probe(index)
                        .args(["-I", "-S", "-B", "-c", TRACE, target_line, argument])
                        .stdout(Stdio::piped()),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn optimized_legacy_await_assertions_preserve_coroutine_symbols() {
    let sources = [
        "def f():\n    assert await g()\n",
        "def f():\n    assert condition, (await g())\n",
        "def f():\n    assert 1 and (await g())\n",
        "def f():\n    assert [await g() for x in xs]\n",
        "def f():\n    assert [x for x in await g()]\n",
        "def f():\n    assert [x async for x in xs]\n",
        "def f():\n    assert {x async for x in xs}\n",
        "def f():\n    assert {x: x async for x in xs}\n",
        "def f():\n    assert (x for x in await g())\n",
        "def f():\n    assert (await g() for x in xs)\n",
        "def f():\n    assert (x async for x in xs)\n",
        "def f():\n    assert [x for x in (x async for x in xs)]\n",
        "def f():\n    assert (lambda: (await g()))\n",
        "def f():\n    assert (lambda x=(await g()): x)\n",
        "def f():\n    assert [(lambda: (await g())) for x in xs]\n",
        "def f():\n    assert (lambda x=[y async for y in ys]: x)\n",
        "def f():\n    assert (yield 1), (await g())\n",
        "def f():\n    assert (yield from (await g()))\n",
        "def f():\n    assert (await (yield from g()))\n",
        "def f():\n    assert (x async for x in await g())\n",
        "def f():\n    assert [await x async for x in xs]\n",
        "def f():\n    assert (lambda x=(await g()): (yield 1))\n",
        "def f():\n    if condition:\n        assert await g()\n    return 1\n",
        "def f():\n    if False:\n        assert await g()\n    return 1\n",
        "def outer():\n    def inner():\n        assert await g()\n    return inner\n",
        "class C:\n    def method(self):\n        assert await g()\n",
        "def f():\n    assert condition\n",
        "def f():\n    assert (lambda: (await g()))\n    yield 1\n",
        "def f():\n    assert await g()\n    def inner(): yield 1\n    return 3\n",
        "def f():\n    assert await g()\n    yield 1\n    return\n",
        "def f():\n    assert await g()\n    yield from xs\n",
        "def ordinary():\n    assert condition\n    yield 1\n    return 3\ndef special():\n    assert await g()\n",
        "def ordinary():\n    assert condition\n    yield 1\n    return 3\n# async await\n",
    ];
    for version in [PythonVersion::Py312, PythonVersion::Py313] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            for source in sources {
                let filename = "legacy_assert_await.py";
                let expected = oracle(source.as_bytes(), filename, version, level);
                assert!(
                    expected.status.success(),
                    "{version} {level:?}: {source}: {expected:?}"
                );
                let actual = compile_with_options(
                    source,
                    filename,
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap()
                .marshal();
                assert_eq!(actual, expected.stdout, "{version} {level:?}: {source}");
            }
        }
    }
}

#[test]
fn optimized_legacy_await_and_yield_reject_value_returns() {
    for source in [
        "def f():\n    assert await g()\n    yield 1\n    return 3\n",
        "def f():\n    assert await g()\n    return (yield 1)\n",
        "def f():\n    assert await g()\n    yield 1\n    if False:\n        return 3\n",
        "def f():\n    assert await g()\n    def inner(x=(yield 1)): pass\n    return 3\n",
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let expected = oracle(source.as_bytes(), "legacy_assert_await.py", version, level);
                assert!(!expected.status.success());
                assert!(
                    String::from_utf8_lossy(&expected.stderr)
                        .contains("'return' with value in async generator")
                );
                let error = compile_with_options(
                    source,
                    "legacy_assert_await.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_err();
                let serc::CompileError::InvalidSyntax { message, range } = error else {
                    panic!("{version} {level:?}: {error:?}");
                };
                assert_eq!(message, "`return` with value in async generator");
                assert_eq!(range.start().to_usize(), source.rfind("return ").unwrap());
                assert_eq!(range.end().to_usize(), source.trim_end().len());
            }
        }
    }
}

#[test]
fn optimized_legacy_await_return_preserves_symbol_error_precedence() {
    for (source, messages) in [
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    global x\n    x = 2\n    global x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    print(x)\n    global x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    nonlocal x\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(yield x) for x in xs]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [x for x in (x := xs)]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(x := 2) for x in xs]\n",
            Some((
                "assignment expression cannot rebind comprehension iteration variable 'x'",
                "assignment expression cannot rebind comprehension variable",
            )),
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    x = 2\n    global x\n    [(yield y) for y in ys]\n",
            None,
        ),
        (
            "def f():\n    assert await g()\n    yield 1\n    return 3\n    [(yield y) for y in ys]\n    x = 2\n    global x\n",
            None,
        ),
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let expected = oracle(source.as_bytes(), "legacy_assert_await.py", version, level);
                let stderr = String::from_utf8_lossy(&expected.stderr);
                let expected_message = stderr
                    .lines()
                    .find_map(|line| line.strip_prefix("SyntaxError: "))
                    .unwrap_or_else(|| panic!("the pinned oracle failed unexpectedly: {stderr}"));
                let serc_message = if let Some((native, serc)) = messages {
                    // Preserve Serc's existing wording while checking both diagnostics.
                    assert_eq!(expected_message, native);
                    serc
                } else {
                    expected_message
                };
                let error = compile_with_options(
                    source,
                    "legacy_assert_await.py",
                    &CompileOptions {
                        python_version: version,
                        optimization_level: level,
                        ..CompileOptions::default()
                    },
                )
                .unwrap_err();
                assert!(
                    error.to_string().ends_with(serc_message),
                    "{version} {level:?}: {source}: expected {serc_message:?}, got {error}"
                );
            }
        }
    }
}

#[test]
fn erased_module_comprehension_cells_precede_synthetic_annotations() {
    for future in ["", "from __future__ import annotations\n"] {
        for annotation in ["value: int\n", "if flag:\n    value: int\n"] {
            for body in [
                "assert [(lambda: item) for item in items]\n",
                "assert [(lambda: z) for z in items]\nassert [(lambda: a) for a in items]\n",
                "out = [(lambda: z) for z in items]\nassert [(lambda: a) for a in items]\n",
                "assert [(lambda: z) for z in items]\nout = [(lambda: a) for a in items]\n",
                "assert [(lambda: __conditional_annotations__) for __conditional_annotations__ in items]\n",
                "assert [(lambda: z) for z in items]\nassert [(lambda: __conditional_annotations__) for __conditional_annotations__ in items]\n",
            ] {
                assert_matches_cpython(
                    &format!("{future}{annotation}{body}"),
                    "erased_module_cells.py",
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn debug_annotations_preserve_postponed_source_spelling() {
    let source =
        "value: __debug__ = 1\ndef function(arg: __debug__) -> tuple[__debug__]:\n    return arg\n";
    assert_matches_cpython(source, "optimized_annotations.py", &LEVELS);
    assert_matches_cpython(
        "assert condition\nx: int = 1\n",
        "optimized_assertion_annotation.py",
        &LEVELS,
    );
    assert_matches_cpython(
        &format!("from __future__ import annotations\n{source}"),
        "optimized_future_annotations.py",
        &LEVELS,
    );
}

#[test]
fn optimized_assertion_syntax_matches_the_target_version() {
    for source in [
        "assert (await value)\n",
        "def f():\n    assert (await g())\n",
        "assert (__debug__ := False)\n",
        "assert func(__debug__=1)\n",
        "assert func(x=1, x=2)\n",
        "assert [0 for x.__debug__ in []]\n",
        "assert (yield, 1)\n",
        "def f():\n    assert (value := 1)\n    global value\n",
    ] {
        for (python_version, _, _) in VERSIONS {
            for optimization_level in LEVELS {
                let expected = oracle(
                    source.as_bytes(),
                    "invalid_optimized_assert.py",
                    python_version,
                    optimization_level,
                );
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                let actual = compile_with_options(source, "invalid_optimized_assert.py", &options);
                if expected.status.success() {
                    assert_eq!(
                        actual.unwrap().marshal(),
                        expected.stdout,
                        "{python_version}, {optimization_level:?}, {source:?}"
                    );
                } else {
                    let stderr = String::from_utf8_lossy(&expected.stderr);
                    let expected_message = stderr
                        .lines()
                        .find_map(|line| line.strip_prefix("SyntaxError: "))
                        .unwrap_or_else(|| {
                            panic!("the pinned oracle failed unexpectedly: {stderr}")
                        });
                    let error = actual.unwrap_err();
                    assert!(
                        error.to_string().ends_with(expected_message),
                        "{python_version}, {optimization_level:?}, {source:?}: \
                         expected {expected_message:?}, got {error}"
                    );
                }
            }
        }
    }
}

#[test]
fn source_decoding_preserves_optimization_options() {
    let source = b"# coding: latin-1\r\n\"caf\xe9\"\r\nassert __debug__\r\nvalue = __debug__\r\n";
    let filename = "optimized_encoding.py";
    for (python_version, _, _) in VERSIONS {
        for optimization_level in LEVELS {
            let options = CompileOptions {
                python_version,
                optimization_level,
                ..CompileOptions::default()
            };
            let expected = oracle(source, filename, python_version, optimization_level);
            assert!(
                expected.status.success(),
                "{}",
                String::from_utf8_lossy(&expected.stderr)
            );
            for actual in [
                compile_bytes_with_options(source, filename, &options).unwrap(),
                compile_bytes_with_path_and_options(source, Path::new(filename), &options).unwrap(),
            ] {
                assert_eq!(
                    actual.marshal(),
                    expected.stdout,
                    "{python_version}, {optimization_level:?}"
                );
            }
        }
    }
}

#[test]
fn default_options_preserve_unoptimized_behavior() {
    let source = "\"module docstring\"\nassert condition\nvalue = __debug__\n";
    let default = CompileOptions::default();
    assert_eq!(default.optimization_level, OptimizationLevel::Zero);
    assert_eq!(
        compile(source, "default_optimization.py")
            .unwrap()
            .marshal(),
        compile_with_options(source, "default_optimization.py", &default)
            .unwrap()
            .marshal()
    );
}

#[test]
fn optimized_deep_expression_traversal_and_cleanup() {
    const CHILD_ENV: &str = "SERC_TEST_OPTIMIZED_DEEP_EXPRESSIONS";
    if std::env::var_os(CHILD_ENV).is_some() {
        // Exercise direct destruction at the short-expression boundary and guarded destruction
        // of longer chains, both with ample stack and with a caller that must grow its stack.
        for terms in [126, 127, 128, 129, 2_048] {
            let expression = std::iter::repeat_n("x", terms)
                .collect::<Vec<_>>()
                .join("+");
            let source = format!("result = {expression}\n");
            let expected = compile(&source, "cleanup_boundary.py").unwrap().marshal();
            for stack_size in [128 * 1024, 8 * 1024 * 1024] {
                let source = source.clone();
                let actual = std::thread::Builder::new()
                    .stack_size(stack_size)
                    .spawn(move || compile(&source, "cleanup_boundary.py").unwrap().marshal())
                    .unwrap()
                    .join()
                    .unwrap();
                assert_eq!(actual, expected);
            }
        }
        let expression = std::iter::repeat_n("value", 2_048)
            .collect::<Vec<_>>()
            .join(" + ");
        let source = format!(
            "\"module docstring\"\nvalue = 1\nassert {expression}\nresult = __debug__ + {expression}\n"
        );
        for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let options = CompileOptions {
                optimization_level,
                ..CompileOptions::default()
            };
            let module = compile_with_options(&source, "deep_optimized.py", &options).unwrap();
            assert!(!module.marshal().is_empty());
        }
        return;
    }

    // A stack-overflow regression must fail this test without aborting the entire test binary.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "optimized_deep_expression_traversal_and_cleanup",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "optimized expression subprocess failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn optimized_empty_manager_bodies_match_cpython() {
    for (header, manager) in [
        ("def f():", "with"),
        ("async def f():", "with"),
        ("async def f():", "async with"),
    ] {
        for body in [
            "        assert predicate()\n",
            "        assert predicate()\n        assert other()\n",
            "        pass\n",
            "        global marker\n        pass\n",
            "        global marker\n        assert predicate()\n",
        ] {
            for continuation in ["", "    after = work()\n"] {
                let source = format!(
                    "{header}\n    before = 1\n    {manager} manager(\n        1,\n        option=True,\n    ) as context:\n{body}{continuation}"
                );
                assert_matches_cpython(&source, "case.py", &LEVELS);
            }
        }
    }

    assert_matches_cpython(
        "def outer():\n    captured = 1\n    def f(m):\n        with m as token:\n            nonlocal captured\n            assert predicate()\n        return captured, token\n    return f\n",
        "case.py",
        &LEVELS,
    );
}

#[test]
fn modern_manager_finally_jump_and_copied_exit_keep_owners() {
    const JUMP_SOURCE: &str = "def f(manager, work):\n    try:\n        raise ValueError()\n    except ValueError:\n        with manager:\n            try:\n                work()\n            finally:\n                pass\n";
    const DIRECT_SOURCE: &str = "def f(manager, work):\n    with manager:\n        try:\n            work()\n        finally:\n            pass\n";
    const PREFIX_SOURCE: &str = "def f(manager, work):\n    with manager:\n        pass\n        try:\n            work()\n        finally:\n            pass\n";
    const COPY_SOURCE: &str = "def f(manager, work, flag):\n    try:\n        if flag:\n            return work()\n        work()\n    finally:\n        with manager:\n            try:\n                work()\n            finally:\n                pass\n";
    const ASSERT_SOURCE: &str = "def f(manager, work, flag):\n    with manager:\n        pass\n        try:\n            work()\n        finally:\n            pass\n            assert flag\n";
    const TRACE: &str = r"
import dis, marshal, sys
mode = sys.argv[1]
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if (i.opname == 'JUMP_FORWARD' and i.positions.lineno == 9 if mode == 'jump'
               else i.opname == 'NOT_TAKEN' and i.positions.lineno == 8 if mode == 'assert'
               else i.opname == 'NOP' and i.positions.lineno == (6 if mode == 'direct' else 3 if mode == 'prefix' else 11))]
if mode == 'assert':
    targets = targets[:1]
if (len(targets) < 2 if mode == 'copy' else len(targets) != 1):
    raise RuntimeError(('missing finalizer marker', mode, targets))
monitor = sys.monitoring
monitor.use_tool_id(5, 'modern-manager-finally-owner')
for flag in ((True, False) if mode == 'copy' else (False,)):
    target = targets[1 if mode == 'copy' and not flag else 0]
    for exception in (RuntimeError, StopIteration):
        for suppress in (False, True):
            events, hits = [], []
            class Manager:
                def __enter__(self):
                    events.append('enter')
                def __exit__(self, typ, value, traceback):
                    events.append(('exit', typ))
                    return suppress
            def work():
                events.append('work')
                return 7
            def inject(code, offset):
                if code is f.__code__ and offset == target:
                    hits.append(offset)
                    raise exception('injected')
            monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
            monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
            try:
                if mode in ('copy', 'assert'):
                    f(Manager(), work, flag)
                else:
                    f(Manager(), work)
                outcome = None
            except BaseException as error:
                outcome = type(error)
            finally:
                monitor.set_local_events(5, f.__code__, 0)
                monitor.register_callback(5, monitor.events.INSTRUCTION, None)
            if mode in ('jump', 'prefix') or (mode == 'assert' and sys.version_info[:2] == (3, 14)):
                expected_events = (['enter', 'work', ('exit', exception)] if mode in ('jump', 'assert')
                                   else ['enter', ('exit', exception)])
                expected_outcome = None if suppress else exception
            else:
                expected_events = (['enter', 'work'] if mode in ('direct', 'assert')
                                   else ['work', 'enter', 'work'])
                expected_outcome = exception
            if hits != [target] or events != expected_events or outcome is not expected_outcome:
                raise RuntimeError((mode, hits, events, outcome, expected_outcome))
monitor.free_tool_id(5)
";

    for (index, python_version) in [(2, PythonVersion::Py314), (3, PythonVersion::Py315)] {
        for optimization_level in LEVELS {
            for (mode, source) in [
                ("jump", JUMP_SOURCE),
                ("direct", DIRECT_SOURCE),
                ("prefix", PREFIX_SOURCE),
                ("copy", COPY_SOURCE),
                ("assert", ASSERT_SOURCE),
            ] {
                if mode == "assert" && optimization_level != OptimizationLevel::Zero {
                    continue;
                }
                let [native, actual] = compile_both(
                    source,
                    "modern_manager_finally.py",
                    python_version,
                    optimization_level,
                );
                assert_runs(
                    probe(index).args(["-I", "-S", "-B", "-c", TRACE, mode]),
                    [&native, &actual],
                );
            }
        }
    }
}

#[test]
fn legacy_manager_finally_jump_keeps_manager_owner() {
    const SOURCE: &str = "def f(manager, work):\n    try:\n        raise ValueError()\n    except ValueError:\n        with manager:\n            pass\n            try:\n                work()\n            finally:\n                pass\n";
    const TRACE: &str = r"
import dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'JUMP_FORWARD' and i.positions.lineno == 10]
if len(targets) != 1:
    raise RuntimeError(('missing finalizer jump', targets))
target = targets[0]
monitor = sys.monitoring
monitor.use_tool_id(5, 'legacy-manager-finally-owner')
for error_type in (RuntimeError, StopIteration):
    for suppress in (False, True):
        events, hits = [], []
        class Manager:
            def __enter__(self):
                events.append('enter')
            def __exit__(self, typ, value, traceback):
                events.append(('exit', typ))
                return suppress
        def work():
            events.append('work')
        def inject(code, offset):
            if offset == target:
                hits.append(offset)
                raise error_type('injected')
        monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
        monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
        try:
            f(Manager(), work)
            outcome = None
        except BaseException as error:
            outcome = type(error)
        finally:
            monitor.set_local_events(5, f.__code__, 0)
            monitor.register_callback(5, monitor.events.INSTRUCTION, None)
        expected = None if suppress else error_type
        if hits != [target] or events != ['enter', 'work', ('exit', error_type)] or outcome is not expected:
            raise RuntimeError((hits, events, outcome, expected))
monitor.free_tool_id(5)
";

    for (index, python_version) in [PythonVersion::Py312, PythonVersion::Py313]
        .into_iter()
        .enumerate()
    {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                SOURCE,
                "legacy_manager_finally.py",
                python_version,
                optimization_level,
            );
            assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            assert_runs(
                probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn elided_prefix_preserves_finally_manager_exit() {
    let source = "def run(worker):\n    with outer():\n        with inner():\n            item = worker.read()\n            assert item\n            pending = worker.start()\n            try:\n                consume(pending)\n            finally:\n                worker.finish()\n            assert item\n";
    let without_prefix_assert = source.replacen("            assert item\n", "", 1);
    for (index, source) in [
        source.to_owned(),
        without_prefix_assert.clone(),
        source.replacen("            assert item\n", "            pass\n", 1),
        source.replacen(
            "            assert item\n",
            "            inspect(item)\n",
            1,
        ),
        without_prefix_assert.replace(
            "            pending = worker.start()\n",
            "            pending = worker.start()\n            assert item\n",
        ),
        source.replacen(
            "            assert item\n",
            "            assert item\n            assert worker\n",
            1,
        ),
        format!("{source}            inspect(item)\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("elided_prefix_finally_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn completed_prefix_preserves_finally_manager_exit() {
    for function in ["def", "async def"] {
        for prefix in [
            "        from package import item\n        saved = obj.value\n",
            "        with manager:\n            prepare()\n        saved = obj.value\n",
            "        with acquire() as resource:\n            prepare(resource)\n        from package import item\n        saved = obj.value\n",
        ] {
            let source = format!(
                "{function} f(manager, obj):\n    with manager:\n{prefix}        try:\n            result = obj.work()\n        finally:\n            obj.value = saved\n        assert result\n        assert obj\n"
            );
            assert_matches_cpython(&source, "completed_prefix_finally.py", &LEVELS);
        }
    }
}

#[test]
fn nested_definition_before_finally_preserves_manager_exit() {
    for definition in [
        "async def child():\n            yield flag",
        "async def child(*, item: Item) -> Items[Item]:\n            yield flag",
        "def child():\n            return flag",
    ] {
        let source = format!(
            "async def f(manager, task, flag):\n    with manager:\n        {definition}\n        configure(child)\n        try:\n            assert flag\n        finally:\n            release()\n            result = await task\n        assert result\n"
        );
        assert_matches_cpython(&source, "nested_definition_finally_exit.py", &LEVELS);
    }
    assert_matches_cpython(
        "async def f(manager, task, flag):\n    with manager:\n        'marker'\n        configure(flag)\n        try:\n            assert flag\n        finally:\n            release()\n            result = await task\n        assert result\n",
        "literal_prefix_finally_exit.py",
        &LEVELS,
    );
}

#[test]
fn elided_assertion_after_manager_finally_preserves_preceding_pass_owner() {
    const SOURCE: &str = "def f(cm, x):\n    with cm:\n        pass\n        try:\n            work()\n        finally:\n            cleanup()\n        assert x\n";
    const TRACE: &str = r"
import dis, json, marshal, sys
events = []
class Fault(Exception): pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
scope = {'work': lambda: events.append('work'), 'cleanup': lambda: events.append('cleanup')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == 3]
if len(targets) != 1:
    raise RuntimeError('expected one authored pass NOP')
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == targets[0]:
            hits.append(True)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    try:
        f(Manager(), True)
    except Fault:
        events.append('escaped')
finally:
    sys.settrace(None)
if len(hits) != 1:
    raise RuntimeError('authored pass NOP was not reached')
print(json.dumps(events))
";
    for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate().skip(2) {
        for optimization_level in &LEVELS[1..] {
            let [native, actual] = compile_both(
                SOURCE,
                "manager_finally_assert.py",
                python_version,
                *optimization_level,
            );
            assert_outputs_match(
                probe(index).args(["-I", "-S", "-B", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn empty_try_in_conditional_finally_compacts_setup_before_exit() {
    const SOURCE: &str = "async def f(manager, items, work, flag):\n    async with manager:\n        for item in items:\n            try:\n                await work(item)\n            finally:\n                if flag:\n                    value = work()\n                    count = value / 2\n                    try:\n                        assert count == item\n                    except AssertionError:\n                        pass\n";
    for body in ["assert count == item", "pass", "work(count)"] {
        let source = SOURCE.replace("assert count == item", body);
        assert_matches_cpython(&source, "empty_try_finally_exit.py", &LEVELS);
    }
}

#[test]
fn nested_erased_conditions_preserve_original_function_return() {
    let source = "async def f(state):\n    result = await prepare()\n    if state.previous is not None:\n        assert result\n    if state.outer is not None:\n        if state.inner is None:\n            assert result\n        else:\n            assert state.inner\n        assert result\n";
    for (index, source) in [
        source.to_owned(),
        source
            .replace("async def", "def")
            .replace("await prepare()", "prepare()"),
        source.replace(
            "    if state.previous is not None:\n        assert result\n",
            "",
        ),
        source.replace("state.outer is not None", "state.outer is None"),
        source.replace("state.inner is None", "state.inner is not None"),
        format!("{source}    else:\n        assert result\n"),
        source
            .replace("\n    ", "\n        ")
            .replacen('\n', "\n    with manager:\n", 1),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("nested_erased_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_suffix_preserves_original_function_return() {
    let source = "async def f(first, second):\n    await prepare()\n    if second:\n        assert second\n    if first and not second:\n        assert first\n    else:\n        assert second\n    assert first\n";
    for (index, source) in [
        source.to_owned(),
        source
            .replace("async def", "def")
            .replace("await prepare()", "prepare()"),
        source.replace("    if second:\n        assert second\n", ""),
        "async def f(first, second):\n    await prepare()\n    if second:\n        assert second\n    while first:\n        assert second\n    assert second\n".to_owned(),
        "async def f(first, second):\n    await prepare()\n    if second:\n        assert second\n    while first:\n        if second:\n            break\n        work()\n    assert second\n".to_owned(),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_suffix_return_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn protected_erased_conditionals_preserve_repeated_join_predecessors() {
    let source = "async def f(manager, earlier, value, items):\n    async with manager:\n        await prepare()\n        if earlier:\n            assert ready()\n        if value is not None:\n            assert ready()\n        else:\n            assert other()\n        for item in items:\n            await consume(item)\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("async with manager:", "with manager:"),
        source.replace("            assert ready()", "            prepare()"),
        source.replace("        if earlier:\n            assert ready()\n", ""),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_erased_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditionals_preserve_repeated_join_predecessors_in_suspended_scopes() {
    let source = "async def f(flag, items):\n    await prepare()\n    if other:\n        assert ready()\n    if flag:\n        assert ready()\n    else:\n        assert other()\n    value = consume(items)\n    for item in items:\n        assert item\n";
    for (index, source) in [
        source.to_owned(),
        source.replace("    await prepare()\n", ""),
        source
            .replace("async def", "def")
            .replace("    await prepare()\n", ""),
        source
            .replace("async def", "def")
            .replace("    await prepare()", "    yield prepare()"),
        source.replace("        assert other()", "        consume(items)"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("erased_condition_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn empty_try_before_manager_cleanup_keeps_its_exit_unowned() {
    let source = "def f(manager):\n    try:\n        prepare()\n        with manager as resource:\n            resource.first()\n            resource.second()\n            try:\n                assert resource.check()\n            except Error:\n                pass\n        after()\n    finally:\n        finish()\n";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "                assert resource.check()",
            "                pass",
        ),
        source.replace(
            "                assert resource.check()",
            "                resource.check()",
        ),
        source.replace("        after()\n", ""),
        source.replace(
            "                pass\n",
            "                pass\n            resource.third()\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("empty_try_manager_cleanup_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn optimized_empty_try_loop_keeps_enclosing_exception_owner() {
    const TRACE_BACKEDGE: &str = r"
import dis, json, marshal, sys

events = []
def recover():
    events.append('recover')
def cleanup():
    events.append('cleanup')
def log():
    events.append('log')
scope = dict(recover=recover, cleanup=cleanup, log=log)
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
backedge = next(i.offset for i in dis.get_instructions(f) if i.opname == 'JUMP_BACKWARD')
fired = [False]
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        if event == 'call':
            frame.f_trace_opcodes = True
        elif event == 'opcode' and frame.f_lasti == backedge:
            fired[0] = True
            raise ValueError('injected')
    return trace
sys.settrace(trace)
# Prime opcode tracing, then reset it before the execution under test.
f([])
sys.settrace(None)
events.clear()
sys.settrace(trace)
try:
    f([1])
except ValueError:
    events.append('escaped')
finally:
    sys.settrace(None)
assert fired[0]
print(json.dumps(events))
";
    assert_matches_cpython(
        "async def f(xs):\n    for x in xs:\n        try:\n            assert x\n        except:\n            log()\n",
        "case.py",
        &LEVELS,
    );
    let cases = [
        (
            "def f(xs):\n    try:\n        for x in xs:\n            try:\n                assert x\n            except:\n                log()\n    except Exception:\n        recover()\n",
            &b"[\"recover\"]\n"[..],
        ),
        (
            "def f(xs):\n    try:\n        for x in xs:\n            try:\n                assert x\n            except:\n                log()\n    finally:\n        cleanup()\n",
            &b"[\"cleanup\", \"escaped\"]\n"[..],
        ),
    ];
    for (source, expected_events) in cases {
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] =
                    compile_both(source, "case.py", python_version, optimization_level);
                let output = assert_outputs_match(
                    probe(index).args(["-I", "-S", "-c", TRACE_BACKEDGE]),
                    [&native, &actual],
                );
                assert_eq!(
                    output, expected_events,
                    "{python_version}, {optimization_level:?}"
                );
                assert_eq!(actual, native, "{python_version}, {optimization_level:?}");
            }
        }
    }
}

#[test]
fn optimized_match_guards_preserve_default_locations() {
    for (pattern, guard) in [("y", "__debug__"), ("_", "__debug__"), ("y", "False")] {
        let source = format!(
            "def f(x):\n    match x:\n        case {pattern} if {guard}:\n            return 1\n        case _:\n            return 2\n"
        );
        assert_matches_cpython(&source, "match_guard.py", &LEVELS);
    }

    for source in [
        r#"def f(x, flag, other):
    match x:
        case y if (
            __debug__
        ):
            raise RuntimeError("guard")
        case _:
            raise ValueError("default")
"#,
        r#"def f(x, manager, extra):
    try:
        trigger()
    except* ValueError as e:
        match x:
            case y if (
                __debug__
            ):
                raise RuntimeError("guard")
            case _:
                raise ValueError("default")
    return 9
"#,
        r#"def f(x, manager, extra):
    with manager:
        try:
            trigger()
        finally:
            match x:
                case y if (
                    __debug__
                ):
                    raise RuntimeError("guard")
                case _:
                    raise ValueError("default")
    return 9
"#,
    ] {
        assert_matches_cpython(source, "match_guard_control.py", &LEVELS);
    }
}

#[test]
fn elided_else_preserves_async_comprehension_exit() {
    for comprehension in [
        "[item async for item in items]",
        "{item async for item in items}",
        "{item: item async for item in items}",
    ] {
        for otherwise in ["assert flag", "value = items"] {
            for continuation in ["return await consume(value)", "return value"] {
                let source = format!(
                    "async def f(flag, items):\n    if flag:\n        value = {comprehension}\n    else:\n        {otherwise}\n    {continuation}\n"
                );
                assert_matches_cpython(&source, "async_comprehension_exit.py", &LEVELS);
            }
        }
    }
}

#[test]
fn async_comprehension_preserves_conditional_exception_owner() {
    const TRACE_CONDITION: &str = r"
import dis, marshal, sys

scope = {'work': lambda: None}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOT_TAKEN' and i.positions.lineno == int(sys.argv[1])]
assert len(targets) == 1, targets
target = targets[0]
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(frame.f_lasti)
            raise ValueError('injected')
    return trace
def drive(x):
    try:
        f(x, [('k', 1), ('j', 2)], {}).send(None)
    except StopIteration:
        return 'completed'
    except ValueError:
        return 'escaped'
    return 'suspended'
sys.settrace(trace)
warm = drive(False)
sys.settrace(None)
hits.clear()
sys.settrace(trace)
try:
    outcome = drive(True)
finally:
    sys.settrace(None)
assert warm == 'completed', warm
assert hits == [target], hits
print(outcome)
";
    let cases = [
        ("{**{k: 1 for k in xs}}", "", 4, false),
        ("[{k: 1 for k in xs}]", "", 4, false),
        ("[x, *[k for k in xs], x]", "", 4, false),
        ("({k: 1 for k in xs}, 1)", "", 4, false),
        ("(x, {k: 1 for k in xs})", "", 4, false),
        ("([x, *xs], [x, *[k for k in xs]])", "", 4, false),
        ("([x, *[k for k in xs]], [x, *xs])", "", 4, false),
        ("(*({k: 1 for k in xs},), x)", "", 4, false),
        ("([k for k in xs], (*xs, 1, 2))", "", 4, false),
        ("([k for k in xs], (*[1, 2], [x, *xs]))", "", 4, false),
        ("{**dict(xs)}", "", 4, true),
        ("{**{k: 1 for k in xs}}", "    pass\n", 5, false),
        ("{**{k: 1 for k in xs}}", "    pass; pass\n", 5, true),
        (
            "{**{k: 1 for k in xs}}",
            "    try:\n        work()\n    finally:\n        pass\n",
            8,
            false,
        ),
    ];
    for (expression, prefix, condition_line, protected_314) in cases {
        let source = format!(
            "async def f(x, xs, d):\n    v = {expression}\n{prefix}    try:\n        if x:\n            pass\n    except Exception:\n        pass\n"
        );
        for (index, (python_version, _, _)) in VERSIONS.iter().copied().enumerate() {
            for optimization_level in LEVELS {
                let [native, actual] =
                    compile_both(&source, "case.py", python_version, optimization_level);
                assert_eq!(
                    actual, native,
                    "{python_version}, {optimization_level:?}, {expression}"
                );
                if matches!(python_version, PythonVersion::Py314 | PythonVersion::Py315) {
                    let expected = if protected_314 && python_version == PythonVersion::Py314 {
                        &b"completed\n"[..]
                    } else {
                        &b"escaped\n"[..]
                    };
                    let output = assert_outputs_match(
                        probe(index)
                            .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                            .arg(condition_line.to_string()),
                        [&native, &actual],
                    );
                    assert_eq!(
                        output, expected,
                        "{python_version}, {optimization_level:?}, {expression}"
                    );
                }
            }
        }
    }
}

#[test]
fn discarded_async_yield_preserves_comprehension_exception_owners() {
    for prefix in [
        "    assert value\n    assert obj\n",
        "",
        "    pass\n",
        "    consume(value)\n",
        "    yield value\n",
        "    await later()\n",
    ] {
        let source = format!(
            "async def f(obj, value):\n    value = {{**{{key: 0 for key in obj}}, \"a\": 0, \"b\": 0}}\n    yield value\n{prefix}    try:\n        if obj.flag:\n            consume(value)\n        yield value\n        await later()\n    except Exception:\n        handle()\n"
        );
        assert_matches_cpython(&source, "discarded_async_yield.py", &LEVELS);
    }
}

#[test]
fn comprehension_collection_continuations_preserve_exception_owners() {
    for expression in [
        "[x, *[k for k in xs], x]",
        "({k: 1 for k in xs}, 1)",
        "([x, *xs], [x, *[k for k in xs]])",
        "(*({k: 1 for k in xs},), x)",
        "([k for k in xs], (*[1, 2], [x, *xs]))",
    ] {
        let source = format!(
            "def f(x, xs, d):\n    v = {expression}\n    try:\n        if x:\n            pass\n    except Exception:\n        pass\n"
        );
        assert_matches_cpython(&source, "case.py", &LEVELS);
    }
}

#[test]
fn elided_assertion_identifiers_preserve_frozenset_identity() {
    const SOURCE: &str = r#"
def first():
    sink(items={"shared_member", "second_member", "third_member"})

def second():
    sink(extra=None, items={"shared_member", "second_member", "third_member"})

def discarded_assert():
    assert context.shared_member
"#;
    const TEXT_ORACLE: &str = r"
import marshal, sys
source = sys.stdin.buffer.read().decode('utf-8')
code = compile(source, sys.argv[2], 'exec', dont_inherit=True, optimize=int(sys.argv[1]))
sys.stdout.buffer.write(marshal.dumps(code))
";
    const IDENTITY_ORACLE: &str = r"
import marshal, sys, types
module = marshal.loads(sys.stdin.buffer.read())
functions = {c.co_name: c for c in module.co_consts if isinstance(c, types.CodeType)}
sets = [next(c for c in functions[name].co_consts if isinstance(c, frozenset))
        for name in ('first', 'second')]
assert sets[0] == sets[1]
assert (sets[0] is sets[1]) == (sys.argv[1] == 'shared')
";
    let cases = [
        (SOURCE.to_string(), false),
        (SOURCE.replace("    assert context.shared_member", "    pass"), true),
        (SOURCE.replace("    assert context.shared_member", "    context.shared_member"), false),
        (SOURCE.replace("extra=None, ", ""), true),
        (SOURCE.replace("\"shared_member\"", "\"Key_member\"").replace("context.shared_member", "context.\u{212a}ey_member"), false),
        (SOURCE.replace("assert context.shared_member", "assert (lambda: context.shared_member)()"), false),
        (SOURCE.replace("shared_member", "_Box__member").replace("def discarded_assert():\n    assert context._Box__member", "class Box:\n    def f(self):\n        return self.__member\n\n    def discarded_assert(self):\n        assert context._Box__member"), false),
        (SOURCE.replace("def discarded_assert():\n    assert context.shared_member", "assert context.shared_member"), false),
    ];
    for (source, shared) in cases {
        for (version, _, _) in VERSIONS {
            for level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let options = CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                };
                let expected = oracle(source.as_bytes(), "case.py", version, level);
                assert!(
                    expected.status.success(),
                    "{}",
                    String::from_utf8_lossy(&expected.stderr)
                );
                let actual = compile_bytes_with_options(source.as_bytes(), "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(actual, expected.stdout, "{version}, {level:?}, bytes");

                let index = VERSIONS
                    .iter()
                    .position(|(candidate, _, _)| *candidate == version)
                    .unwrap();
                let level_arg = if level == OptimizationLevel::One {
                    "1"
                } else {
                    "2"
                };
                let expected_text = run_successfully(
                    probe(index)
                        .args([
                            "-I",
                            "-S",
                            "-X",
                            "utf8",
                            "-c",
                            TEXT_ORACLE,
                            level_arg,
                            "case.py",
                        ])
                        .stdout(Stdio::piped()),
                    source.as_bytes(),
                );
                let actual_text = compile_with_options(&source, "case.py", &options)
                    .unwrap()
                    .marshal();
                assert_eq!(actual_text, expected_text, "{version}, {level:?}, text");

                assert_runs(
                    probe(index)
                        .args([
                            "-I",
                            "-S",
                            "-X",
                            "utf8",
                            "-c",
                            IDENTITY_ORACLE,
                            if shared { "shared" } else { "distinct" },
                        ])
                        .stdout(Stdio::piped()),
                    [&expected.stdout, &actual],
                );
            }
        }
    }
}

#[test]
fn elided_non_ascii_identifier_preserves_discarded_builders() {
    const PREFIX: &str = "def marker():\n    assert probe.élève\n\n";
    let discarded = [
        "if False:\n    def hidden(value):\n        return value + 1\n",
        "saved = (\"élève\",)\nif False:\n    class Hidden:\n        def method(self):\n            return (\"élève\",)\n",
        "if False:\n    async def hidden(items):\n        async for item in items:\n            try:\n                yield item\n            finally:\n                cleanup()\n",
    ];
    for body in discarded {
        assert_matches_cpython(&format!("{PREFIX}{body}"), "case.py", &LEVELS);
    }
}

#[test]
fn comprehension_branches_preserve_backedge_store_locations() {
    let source = "def check_groups(groups, flag):\n    for group in groups:\n        if flag:\n            selected = [item for item in group if ready(item)]\n            assert selected\n        else:\n            selected = [item for item in group if item.active]\n            assert selected\n        assert len(selected) == 1\n";
    for source in [
        source.to_owned(),
        source.replace("[item for item in group", "[index for index, item in enumerate(group)"),
        source.replace("[item for item in group", "[index for index, item, extra in triples(group)"),
        source.replace("[item for item in group", "[(item, other) for item in group for other in group"),
        source.replace("[item for item in group if item.active]", "[\n                item\n                for item in group\n                if item.active\n            ]"),
        source.replace("[item for item in group if ready(item)]", "{item for item in group if ready(item)}").replace("[item for item in group if item.active]", "{item for item in group if item.active}"),
        source.replace("[item for item in group if ready(item)]", "{item: item for item in group if ready(item)}").replace("[item for item in group if item.active]", "{item: item for item in group if item.active}"),
        source.replace("selected =", "item ="),
        source.replace("selected =", "flag.selected ="),
        source.replace("        assert len(selected) == 1", "        consume(selected)"),
    ] {
        assert_matches_cpython(&source, "comprehension_backedge_store.py", &LEVELS);
    }
}

#[test]
fn elided_assertions_after_comprehension_branches_keep_store_return_locations() {
    for prefix in ["", "async "] {
        for comprehension in [
            "[item for item in items]",
            "{item for item in items}",
            "{item: item for item in items}",
            "[\n            item\n            for item in items\n        ]",
        ] {
            for branch_tail in ["", "        assert result\n"] {
                let source = format!(
                    "{prefix}def f(items, flag):\n    if flag:\n        result = {comprehension}\n{branch_tail}    assert flag\n"
                );
                assert_matches_cpython(&source, "comprehension_branch_return.py", &LEVELS);
            }
        }
    }
}

#[test]
fn short_circuit_prefix_preserves_nested_return_donors() {
    let source = r"def validate(kind):
    if kind == 1:
        enabled = ready() and first() in (1, 2)
        assert enabled
        active = ready()
        if active:
            left = first()
            right = second()
            assert left
            assert right
        else:
            assert enabled
            left = first() if ready() else 1
        assert left
    else:
        raise ValueError(kind)
";
    for source in [
        source.to_owned(),
        source.replace(
            "ready() and first() in (1, 2)",
            "ready() or first() in (1, 2)",
        ),
        source.replace(
            "ready() and first() in (1, 2)",
            "(ready() or other()) and first() in (1, 2)",
        ),
        source.replace("ready() and first() in (1, 2)", "ready()"),
    ] {
        assert_matches_cpython(&source, "short_circuit_prefix_return.py", &LEVELS);
    }
}

#[test]
fn manager_comprehensions_preserve_shared_conditional_returns() {
    let source = r"async def sample(flag, items):
    if flag:
        async with manager():
            assert [item async for item in items] == []
        assert flag
";
    for asynchronous in [false, true] {
        for nested in [false, true] {
            for assigned in [false, true] {
                let mut source = source.to_string();
                if nested {
                    source = source.replace(
                        "            assert [",
                        "            async with other():\n                assert [",
                    );
                }
                if !asynchronous {
                    source = source.replace("async with", "with");
                }
                if assigned {
                    source = source.replace("        assert flag\n", "        result = flag\n");
                }
                assert_matches_cpython(
                    &source,
                    &format!("manager_comprehension_exit_{asynchronous}_{nested}_{assigned}.py"),
                    &LEVELS,
                );
            }
        }
    }
}

#[test]
fn suspended_comprehension_siblings_preserve_return_locations() {
    let source = r"async def collect(flag, items):
    if flag:
        with manager():
            result = [item async for item in items]
    else:
        result = [item async for item in items]
    assert result
";
    for source in [
        source.to_owned(),
        source.replace(
            "[item async for item in items]",
            "{item async for item in items}",
        ),
        source.replace(
            "[item async for item in items]",
            "{item: item async for item in items}",
        ),
        source.replace(
            "[item async for item in items]",
            "[await transform(item) for item in items]",
        ),
        source.replace("async for", "for"),
        source.replace("assert result", "consume(result)"),
    ] {
        assert_matches_cpython(&source, "suspended_comprehension_siblings.py", &LEVELS);
    }
}

#[test]
fn coroutine_comprehension_siblings_preserve_return_locations() {
    let source = "async def collect(flag, items, fields):\n    await prepare()\n    if flag:\n        with manager():\n            result = [tuple(item for field in fields) for item in items]\n    else:\n        result = [\n            tuple(item for field in fields)\n            for item in items\n        ]\n    assert result\n";
    for source in [
        source.to_owned(),
        source.replace("tuple(item for field in fields)", "item"),
        source.replace(
            "with manager():",
            "with manager() as active, second_manager() as other:",
        ),
        source.replace("    assert result\n", "    consume(result)\n"),
        "async def collect(flag, items):\n    await prepare()\n    if flag:\n        work()\n    else:\n        result = [item for item in items]\n    assert flag\n".to_owned(),
        "async def collect(flag, other, items):\n    await prepare()\n    if flag:\n        result = [item for item in items]\n    elif other:\n        result = {item for item in items}\n    else:\n        result = {item: item for item in items}\n    assert result\n".to_owned(),
    ] {
        assert_matches_cpython(&source, "coroutine_comprehension_siblings.py", &LEVELS);
    }
}

#[test]
fn chained_generator_loop_tail_preserves_stop_iteration_wrapper() {
    const SOURCE: &str = "def test(xs, y, z):\n    yield 0\n    for x in xs:\n        if x:\n            h()\n        elif x < y < z:\n            yield x\n        else:\n            assert z\n";
    const TRACE_ORACLE: &str = r"
import dis, marshal, sys
namespace = {'h': lambda: None}
exec(marshal.loads(sys.stdin.buffer.read()), namespace)
function = namespace['test']
instructions = list(dis.get_instructions(function))
targets = [instructions[i].offset for i in range(1, len(instructions))
           if instructions[i].opname == 'JUMP_BACKWARD'
           and instructions[i - 1].opname == 'POP_TOP'
           and instructions[i].positions.lineno == 6]
assert len(targets) == 1, targets
generator = function([0], -1, 3)
assert next(generator) == 0

def trace(frame, event, arg):
    if frame.f_code is function.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == targets[0]:
            raise StopIteration('trace interruption')
    return trace

sys.settrace(trace)
try:
    next(generator)
except RuntimeError as exc:
    assert isinstance(exc.__cause__, StopIteration), exc.__cause__
else:
    raise AssertionError('exception was not propagated')
finally:
    sys.settrace(None)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let expected = oracle(SOURCE.as_bytes(), "generator_chain.py", version, level);
            assert!(expected.status.success(), "{version}, {level:?}");
            let actual = compile_with_options(
                SOURCE,
                "generator_chain.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            assert_runs(
                probe(index)
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                    .stdout(Stdio::piped()),
                [&expected.stdout, &actual],
            );
        }
    }
}

#[test]
fn protected_loop_elided_else_excludes_conditional_cleanup() {
    const SOURCE: &str = "def test(xs, manager, flag):\n    try:\n        for x in xs:\n            if x:\n                with manager as value:\n                    pass\n                    pass\n                    global marker\n            else:\n                assert flag\n    finally:\n        cleanup()\n    return\n";
    const TRACE_ORACLE: &str = r"
import contextlib, dis, marshal, sys
class Interrupted(Exception):
    pass

events = []
namespace = {'cleanup': lambda: events.append('cleanup')}
exec(marshal.loads(sys.stdin.buffer.read()), namespace)
function = namespace['test']
targets = [instruction.offset for instruction in dis.get_instructions(function)
           if instruction.positions.lineno == 4
           and instruction.opname.startswith('POP_JUMP')]
assert len(targets) == 1, targets
for value in (False, True):
    hit = False
    def trace(frame, event, arg):
        global hit
        if frame.f_code is function.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == targets[0]:
                hit = True
                raise Interrupted()
        return trace
    sys.settrace(trace)
    try:
        function([value], contextlib.nullcontext(), True)
    except Interrupted:
        pass
    else:
        raise AssertionError('exception was not propagated')
    finally:
        sys.settrace(None)
    assert hit, 'conditional was not executed'
    assert not events, events
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let expected = oracle(SOURCE.as_bytes(), "protected_loop.py", version, level);
            assert!(expected.status.success(), "{version}, {level:?}");
            let actual = compile_with_options(
                SOURCE,
                "protected_loop.py",
                &CompileOptions {
                    python_version: version,
                    optimization_level: level,
                    ..CompileOptions::default()
                },
            )
            .unwrap()
            .marshal();
            assert_eq!(actual, expected.stdout, "{version}, {level:?}");
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            assert_runs(
                probe(index)
                    .args(["-I", "-S", "-X", "utf8", "-c", TRACE_ORACLE])
                    .stdout(Stdio::piped()),
                [&expected.stdout, &actual],
            );
        }
    }
}

#[test]
fn nested_try_fallthrough_preserves_conditional_exception_owners() {
    const EXACT: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            x = 0\n        break\n";
    const FRESH: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            d.clear()\n        break\n";
    const RETAINED: &str = "def f(a,b,c,d):\n    while c:\n        try:\n            try:\n                pass; pass\n                if a:\n                    pass\n            except Exception:\n                pass\n        finally:\n            d.clear()\n        break\n";
    const TRACE_CONDITION: &str = r"
import dis, marshal, sys

events = []
class Injected(Exception):
    pass
class Clean:
    def clear(self):
        events.append('cleanup')
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOT_TAKEN' and i.positions.lineno == int(sys.argv[1])]
assert len(targets) == 1, targets
target = targets[0]
hits = []
monitor = sys.monitoring
monitor.use_tool_id(5, 'conditional-owner')
def inject(code, offset):
    if offset == target:
        hits.append(offset)
        raise Injected()
monitor.register_callback(5, monitor.events.INSTRUCTION, inject)
monitor.set_local_events(5, f.__code__, monitor.events.INSTRUCTION)
try:
    f(True, False, True, Clean())
    outcome = 'returned'
except Injected:
    outcome = 'escaped'
finally:
    monitor.set_local_events(5, f.__code__, 0)
    monitor.register_callback(5, monitor.events.INSTRUCTION, None)
    monitor.free_tool_id(5)
assert hits == [target], hits
print(outcome, events)
";

    assert_matches_cpython(EXACT, "fallthrough_nested_try.py", &LEVELS);
    for (source, line, expected) in [
        (FRESH, 5, &b"escaped []\n"[..]),
        (RETAINED, 6, &b"returned ['cleanup']\n"[..]),
    ] {
        for optimization_level in LEVELS {
            let [native, actual] = compile_both(
                source,
                "fallthrough_nested_try.py",
                PythonVersion::Py314,
                optimization_level,
            );
            let output = assert_outputs_match(
                probe(2)
                    .args(["-I", "-S", "-B", "-c", TRACE_CONDITION])
                    .arg(line.to_string()),
                [&native, &actual],
            );
            assert_eq!(output, expected, "{optimization_level:?}, line {line}");
        }
    }
}

#[test]
fn terminal_matches_before_erased_assertions_keep_case_returns() {
    for (case, source) in [
        "def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b):\n    await action()\n    match a:\n        case 1:\n            if (value := action()):\n                consume(value)\n            assert b\n        case 2:\n            action()\n    assert a\n",
        "async def f(a, b):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                action()\n            assert b\n    assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("terminal_match_before_assertions_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn nonlinear_wildcard_match_tails_preserve_false_edge_returns() {
    for prefix in ["def", "async def"] {
        for (case, tail) in [
            "assert b",
            "global token",
            "global token\n            assert b",
            "assert b\n            global token",
            "value: int",
            "(value): int",
            "nonlocal token",
        ]
        .into_iter()
        .enumerate()
        {
            let source = format!(
                "{prefix} f(a, b, c):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                if c:\n                    action()\n                action()\n            {tail}\n    assert a\n"
            );
            let source = if tail == "nonlocal token" {
                format!(
                    "def outer():\n    token = 0\n    {}",
                    source.replace('\n', "\n    ")
                )
            } else {
                source
            };
            assert_matches_cpython_versions(
                &source,
                &format!("nonlinear_wildcard_match_tail_{case}.py"),
                &LEVELS,
                &[PythonVersion::Py314, PythonVersion::Py315],
            );
        }
    }
}

#[test]
fn nonlinear_match_cases_keep_their_shared_exits() {
    for (case, source) in [
        "def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                if c:\n                    action()\n                action()\n            while b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                if c:\n                    action()\n                action()\n            while b:\n                action()\n            assert b\n    assert a\n",
        "async def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            if a:\n                value = 1\n            else:\n                if b:\n                    if c:\n                        value = 2\n                    else:\n                        value = 3\n                else:\n                    value = 4\n                assert value\n    assert a\n",
        "async def f(a, b, c, d, obj, items, cm):\n    match a:\n        case 1:\n            action()\n        case _:\n            if a:\n                value = 1\n            else:\n                if b:\n                    if c:\n                        value = 2\n                    else:\n                        value = 3\n                else:\n                    value = 4\n                assert value\n    assert a\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("nonlinear_match_case_{case}.py"),
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }
}

#[test]
fn async_match_exhaustion_keeps_case_success_interrupt_check() {
    for pattern in [
        "First()",
        "First() as selected",
        "(First() | Second()) as selected",
        "_",
    ] {
        for body in ["yield item", "await consume(item)"] {
            let source = format!(
                "async def sample(value, items):
    match value:
        case {pattern}:
            async for item in items:
                {body}
    finish()
"
            );
            assert_matches_cpython_versions(
                &source,
                "async_match_exhaustion.py",
                &LEVELS,
                &[PythonVersion::Py313],
            );
        }
    }
}

#[test]
fn completed_managers_preserve_coroutine_match_returns() {
    for manager in [
        "with manager as stream:\n                saved = stream.read()",
        "with manager:\n                saved = read()",
    ] {
        for suffix in ["assert value", "pass", "after()"] {
            let source = format!(
                "async def f(kind, manager, value):\n    match kind:\n        case 1:\n            {manager}\n            if change := compare(saved, value):\n                consume(change)\n            assert change == []\n        case 2:\n            with manager as stream:\n                stream.write(value)\n    {suffix}\n"
            );
            assert_matches_cpython(&source, "completed_manager_match.py", &LEVELS);
        }
    }
}

#[test]
fn nested_match_arms_preserve_tuple_return_copy_budget() {
    for otherwise in ["assert False", "pass", "result = [7]; tag = 'seven'"] {
        for returned in ["result, tag", "result", "result, tag, kind"] {
            let source = format!(
                "def f(kind, first, second):\n    tag = 'unset'\n    match kind:\n        case None:\n            if first:\n                if second:\n                    result = [1]\n                    tag = 'one'\n                elif first:\n                    result = [2]\n                    tag = 'two'\n                else:\n                    result = [3]\n                    tag = 'three'\n            elif second:\n                result = [4]\n                tag = 'four'\n            else:\n                result = [5]\n                tag = 'five'\n        case 'other':\n            result = [6]\n            tag = 'six'\n        case _:\n            {otherwise}\n    return {returned}\n"
            );
            assert_matches_cpython(&source, "nested_match_return_budget.py", &LEVELS);
        }
    }
}

#[test]
fn coroutine_match_cases_preserve_nested_condition_return_donors() {
    for prefix in [
        "",
        "    with cm:\n        with obj:\n            async for item in items:\n                action()\n            assert items\n",
    ] {
        for suffix in ["", "    assert a\n", "    after()\n"] {
            let source = format!(
                "async def f(a, b, c, obj, items, cm):\n{prefix}    match a:\n        case 1:\n            if b:\n                if c:\n                    action()\n                action()\n            assert b\n{suffix}"
            );
            assert_matches_cpython(&source, "nested_match_returns.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_match_case_conditionals_keep_returns_before_erased_assertions() {
    for (case, source) in [
        "def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n",
        "def f(a, b):\n    match a:\n        case 1:\n            if (value := action()):\n                consume(value)\n            assert b\n        case 2:\n            with manager():\n                action()\n",
        "async def f(a, b):\n    match a:\n        case 1:\n            if b:\n                action()\n            assert b\n",
        "def f(a, b):\n    match a:\n        case 1:\n            action()\n        case _:\n            if b:\n                action()\n            assert b\n",
        "def f(a, b):\n    match a:\n        case 1 | 2:\n            if b:\n                action()\n            assert b\n        case _:\n            pass\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython_versions(
            source,
            &format!("terminal_match_case_{case}.py"),
            &LEVELS,
            &VERSIONS.map(|(version, _, _)| version),
        );
    }
}

#[test]
fn optimized_with_assertion_join_before_call_matches_cpython() {
    for condition in ["x", "not x", "x and y", "x < y", "f(x)", "x is None"] {
        let source = format!(
            "def f(x, y, callback, manager):\n    with manager:\n        if {condition}:\n            assert x\n        else:\n            assert y\n        callback(x)\n"
        );
        assert_matches_cpython_versions(
            &source,
            "optimized_with_join.py",
            &LEVELS,
            &VERSIONS.map(|(version, _, _)| version),
        );
    }

    assert_matches_cpython_versions(
        "def f(x, y, callback, manager1, manager2):\n    with manager1, manager2:\n        if y:\n            if x:\n                assert x\n            else:\n                assert y\n            callback(x if y else x)\n",
        "optimized_with_nested_join.py",
        &LEVELS,
        &[PythonVersion::Py314],
    );
}

#[test]
fn nested_pass_exit_retains_enclosing_try_owner() {
    const SOURCE: &str = "def f(flag, items):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n        work()\n    except Fault:\n        events.append('caught')\n";
    const TRACE: &str = r"
import dis, marshal, sys
class Fault(Exception):
    pass
events = []
scope = {'Fault': Fault, 'events': events}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
targets = [i.offset for i in dis.get_instructions(f)
           if i.opname == 'NOP' and i.positions.lineno == 4]
assert len(targets) == 1, targets
target = targets[0]
hits = []
def trace(frame, event, arg):
    if frame.f_code is f.__code__:
        frame.f_trace_opcodes = True
        if event == 'opcode' and frame.f_lasti == target:
            hits.append(target)
            raise Fault()
    return trace
sys.settrace(trace)
try:
    f(True, [1])
finally:
    sys.settrace(None)
assert hits == [target], hits
assert events == ['caught'], events
";
    for (version, index) in [(PythonVersion::Py314, 2), (PythonVersion::Py315, 3)] {
        for optimization_level in LEVELS {
            let [native, actual] =
                compile_both(SOURCE, "nested_pass_exit.py", version, optimization_level);
            assert_eq!(actual, native, "{version} {optimization_level:?}");
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn nested_pass_exit_successors_match_cpython() {
    for successor in ["out = 1", "del item", "if flag:\n            assert flag"] {
        let source = format!(
            "def f(flag, item):\n    try:\n        try:\n            pass\n        except ValueError:\n            pass\n        {successor}\n    except Fault:\n        events.append('caught')\n"
        );
        assert_matches_cpython_versions(
            &source,
            "nested_pass_successor.py",
            &LEVELS,
            &[PythonVersion::Py314, PythonVersion::Py315],
        );
    }

    assert_matches_cpython_versions(
        "def f():\n    try:\n        try: pass\n        except ValueError:\n            pass\n        work()\n    except Fault:\n        events.append('caught')\n",
        "nested_same_line_pass.py",
        &LEVELS,
        &[PythonVersion::Py314, PythonVersion::Py315],
    );
}

#[test]
fn nonterminal_pass_finally_preserves_handler_for_all_targets() {
    for (function, trailer) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield 1\n"),
        ("async def", "    yield 1\n"),
    ] {
        let source = format!(
            "{function} f():\n    try:\n        try:\n            work()\n        finally:\n            pass\n        after()\n    except:\n        recover()\n{trailer}"
        );
        assert_matches_cpython(&source, "nonterminal_pass_finally.py", &LEVELS);
    }
}

#[test]
fn optimized_with_assertion_join_before_noncall_matches_cpython() {
    for successor in [
        "",
        "        value = 1\n",
        "        del flag\n",
        "        return flag\n",
        "        assert flag\n",
        "        if flag:\n            value = 1\n",
    ] {
        let source = format!(
            "def f(flag, manager):\n    with manager:\n        if flag:\n            assert flag\n        else:\n            assert flag\n{successor}"
        );
        assert_matches_cpython_versions(
            &source,
            "optimized_with_join_noncall.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
    // The short-circuit edge also reaches this join and must not retain the NOP.
    assert_matches_cpython_versions(
        "def f(a, b, manager):\n    with manager:\n        if a or b:\n            assert a\n        else:\n            assert b\n        value = a\n",
        "optimized_with_shared_join.py",
        &LEVELS,
        &[PythonVersion::Py314],
    );
}

#[test]
fn optimized_with_assertion_join_before_noncall_keeps_manager_owner() {
    const SOURCE: &str = r"
def terminal(flag):
    with manager('terminal'):
        if flag:
            assert flag
        else:
            assert flag

def assigned(flag):
    with manager('outer'), manager('inner'):
        if flag: assert flag
        else: assert flag
        value = 1

def deleted(flag):
    with manager('delete'):
        if flag:
            assert flag
        else:
            assert flag
        del flag
";
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Fault(Exception):
    pass
class Manager:
    def __init__(self, name):
        self.name = name
    def __enter__(self):
        events.append((self.name, 'enter'))
    def __exit__(self, typ, value, tb):
        events.append((self.name, 'exit', typ.__name__ if typ else None))
        return False
scope = {'manager': Manager}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, expected in [
    ('terminal', [('terminal', 'enter'), 'hit', ('terminal', 'exit', 'Fault')]),
    ('assigned', [('outer', 'enter'), ('inner', 'enter'), 'hit',
                  ('inner', 'exit', 'Fault'), ('outer', 'exit', 'Fault')]),
    ('deleted', [('delete', 'enter'), 'hit', ('delete', 'exit', 'Fault')]),
]:
    fn = scope[name]
    instructions = list(dis.get_instructions(fn))
    offsets = [b.offset for a, b in zip(instructions, instructions[1:])
               if a.opname == 'NOT_TAKEN' and b.opname == 'NOP']
    assert len(offsets) == 1, (name, offsets)
    events.clear()
    def trace(frame, event, arg):
        if frame.f_code is fn.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            fn(True)
            raise AssertionError('injection did not escape')
        except Fault:
            pass
    finally:
        sys.settrace(None)
    assert events == expected, (name, events, expected)
";
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] = compile_both(
            SOURCE,
            "optimized_with_join_owner.py",
            PythonVersion::Py314,
            level,
        );
        assert_eq!(actual, native);
        let index = VERSIONS
            .iter()
            .position(|(version, _, _)| *version == PythonVersion::Py314)
            .unwrap();
        assert_runs(
            probe(index).args(["-I", "-S", "-c", TRACE]),
            [&native, &actual],
        );
    }
}

#[test]
fn protected_with_elided_if_keeps_manager_join_and_exception_owner() {
    const SOURCE: &str = r"
def f(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
        work()
    except Fault:
        recover()

def with_successor(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
            work()
    except Fault:
        recover()

def reverse(flag, manager):
    with manager:
        try:
            if flag:
                assert flag
            else:
                assert flag
        except Fault:
            recover()

def nested_reverse(flag, manager):
    with manager:
        try:
            try:
                if flag:
                    assert flag
                else:
                    assert flag
            except Fault:
                recover()
        except TypeError:
            work()

def except_star(flag, manager):
    try:
        with manager:
            if flag:
                assert flag
            else:
                assert not flag
    except* Fault:
        recover()
";
    const TRACE: &str = r"
import dis, marshal, sys

events = []
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
scope = {'Fault': Fault, 'work': lambda: events.append('work'),
         'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, flag, expected in (
    ('f', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
    ('with_successor', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
    ('reverse', False, ['enter', 'hit', ('exit', 'Fault'), 'escaped']),
    ('nested_reverse', False, ['enter', 'hit', ('exit', 'Fault'), 'escaped']),
    ('except_star', True, ['enter', 'hit', ('exit', 'Fault'), 'recover']),
):
    f = scope[name]
    events.clear()
    instructions = list(dis.get_instructions(f))
    line = next(i.positions.lineno for i in instructions if i.opname == 'POP_JUMP_IF_FALSE')
    offsets = [i.offset for i in instructions if i.opname == 'NOP' and i.positions.lineno == line]
    assert len(offsets) == 1, (name, offsets)
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            f(flag, Manager())
        except Fault:
            events.append('escaped')
    finally:
        sys.settrace(None)
    assert events == expected, (name, events, expected)
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        assert_matches_cpython_versions(SOURCE, "protected_with_join.py", &LEVELS, &[version]);
        for level in [OptimizationLevel::One, OptimizationLevel::Two] {
            let [native, actual] = compile_both(SOURCE, "protected_with_join.py", version, level);
            assert_eq!(actual, native);
            let index = VERSIONS
                .iter()
                .position(|(candidate, _, _)| *candidate == version)
                .unwrap();
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn protected_with_assertion_join_pass_has_no_exception_owner() {
    const SOURCE: &str = concat!(
        "def f(flag, manager):\n    try:\n        with manager:\n            if flag:\n                assert flag\n            else:\n                assert not flag\n            pass\n        work()\n    except Fault:\n        recover()\n",
        "\ndef g(flag, outer, inner):\n    try:\n        with outer:\n            with inner:\n                if flag:\n                    assert flag\n                else:\n                    assert not flag\n                pass\n            work()\n    except Fault:\n        recover()\n",
    );
    const TRACE: &str = r"
import dis, marshal, sys
events = []
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
    def __exit__(self, typ, value, tb):
        events.append('exit')
        return False
scope = {'Fault': Fault, 'work': lambda: events.append('work'),
         'recover': lambda: events.append('recover')}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
for name, offset, args, expected in [
    ('f', 7, (True, Manager()), ['enter', 'hit']),
    ('g', 8, (True, Manager(), Manager()), ['enter', 'enter', 'hit']),
]:
    fn = scope[name]
    offsets = [instruction.offset for instruction in dis.get_instructions(fn)
               if instruction.opname == 'NOP'
               and instruction.positions.lineno == fn.__code__.co_firstlineno + offset]
    if len(offsets) != 1:
        raise AssertionError((name, offsets))
    def trace(frame, event, arg):
        if frame.f_code is fn.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == offsets[0]:
                events.append('hit')
                raise Fault()
        return trace
    events.clear()
    sys.settrace(trace)
    try:
        try:
            fn(*args)
            raise AssertionError('injection did not escape')
        except Fault:
            pass
    finally:
        sys.settrace(None)
    if events != expected:
        raise AssertionError((name, events))
";
    for version in [PythonVersion::Py314, PythonVersion::Py315] {
        for level in LEVELS {
            let [native, actual] =
                compile_both(SOURCE, "protected_with_join_pass.py", version, level);
            assert_eq!(actual, native);
            let index = VERSIONS
                .iter()
                .position(|(target, _, _)| *target == version)
                .unwrap();
            assert_runs(
                probe(index).args(["-I", "-S", "-c", TRACE]),
                [&native, &actual],
            );
        }
    }
}

#[test]
fn optimized_assertions_after_shallow_async_for_keep_manager_exit_location() {
    for (case, source) in [
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            work(item)\n        assert items\n        assert cm\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            value = item\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            continue\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            assert item\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        assert items\n        pass\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        pass\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        async for item in items:\n            pass\n        assert items\n        work(items)\n",
        "async def f(cm, items):\n    async for item in items:\n        pass\n    assert items\n",
        "async def f(cm, items):\n    async with cm:\n        async for item in items:\n            pass\n        assert items\n",
        "async def f(cm, items):\n    with cm:\n        for item in items:\n            pass\n        assert items\n",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("shallow_async_for_manager_exit_{case}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn optimized_assertions_after_async_for_preserve_manager_exit_marker() {
    const SOURCE: &str = "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            pass\n        assert xs\n";
    const TRACE: &str = r"
import asyncio, dis, marshal, sys
scope = {}
exec(marshal.loads(sys.stdin.buffer.read()), scope)
f = scope['f']
instructions = list(dis.get_instructions(f))
end = next(i for i, instruction in enumerate(instructions)
           if instruction.opname == 'END_ASYNC_FOR')
assert instructions[end + 1].opname == 'NOP'
class Fault(Exception):
    pass
class Manager:
    def __enter__(self):
        events.append('enter')
        return self
    def __exit__(self, typ, value, tb):
        events.append(('exit', typ.__name__ if typ else None))
        return False
class Iter:
    def __aiter__(self):
        return self
    async def __anext__(self):
        raise StopAsyncIteration
for target, expected in (
    (instructions[end].offset, ['enter', ('exit', 'Fault'), 'escaped']),
    (instructions[end + 1].offset, ['enter', 'escaped']),
):
    events, hits = [], []
    def trace(frame, event, arg):
        if frame.f_code is f.__code__:
            frame.f_trace_opcodes = True
            if event == 'opcode' and frame.f_lasti == target:
                hits.append(target)
                raise Fault()
        return trace
    sys.settrace(trace)
    try:
        try:
            asyncio.run(f(Iter(), Manager(), None, None))
        except Fault:
            events.append('escaped')
    finally:
        sys.settrace(None)
    assert hits == [target] and events == expected, (hits, events, expected)
";
    for source in [
        SOURCE,
        "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            work(x)\n        assert xs; assert a\n",
        "async def f(xs,a,b,c):\n    async with a:\n        with b:\n            u = 1\n            async for x in xs:\n                pass\n            assert xs\n",
        "async def f(xs,a,b,c):\n    async with a:\n        u = 1\n        async for x in xs:\n            pass\n        assert xs\n",
        "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            if x:\n                break\n            work(x)\n        assert xs\n",
        "async def f(xs,a,b,c):\n    with a:\n        u = 1\n        async for x in xs:\n            pass\n        assert xs\n        work(a)\n",
    ] {
        assert_matches_cpython_versions(
            source,
            "async_for_assert.py",
            &LEVELS,
            &[PythonVersion::Py314],
        );
    }
    for level in [OptimizationLevel::One, OptimizationLevel::Two] {
        let [native, actual] =
            compile_both(SOURCE, "async_for_assert.py", PythonVersion::Py314, level);
        assert_eq!(actual, native);
        assert_runs(probe(2).args(["-I", "-S", "-c", TRACE]), [&native, &actual]);
    }
}

#[test]
fn optimized_footer_after_instruction_free_branch_tail() {
    let template = "def f(x, y, z, value):\n    if x:\n        if y:\n            work()\n        TAIL\n    else:\n        other()\n    after()\n    if y:\n        if z:\n            pass\n        assert z\n";
    for tail in [
        "assert value",
        "global marker",
        "pass\n        global marker",
    ] {
        let source = template.replace("TAIL", tail);
        assert_matches_cpython_versions(
            &source,
            "tail_and_footer.py",
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn optimized_footer_after_dotted_import_in_same_module() {
    for import in [
        "if False:\n    from SercOuter.SercInner.Leaf import item\n",
        "from SercOuter.SercInner import item\n",
        "if False:\n    import SercOuter.SercInner.Leaf\n",
    ] {
        let source = format!(
            "{import}class SercOuter:\n    class SercInner:\n        pass\ndef f(x, y):\n    if x:\n        if y:\n            pass\n        assert y\n"
        );
        assert_matches_cpython_versions(
            &source,
            "dotted_import_footer.py",
            &LEVELS,
            &[PythonVersion::Py313],
        );
    }
}

#[test]
fn instruction_free_branch_tails_preserve_join_locations() {
    let template = "def f(x, y, value):\n    if x:\n        if y:\n            work()\n        TAIL\n    else:\n        other()\n    after()\n";
    for tail in [
        "assert value",
        "global marker",
        "pass\n        global marker",
    ] {
        let source = template.replace("TAIL", tail);
        assert_matches_cpython(&source, "branch_tail.py", &LEVELS);
    }
    // The same join may require an EXTENDED_ARG for a long jump.
    let long_jump = template
        .replace("TAIL", "assert value")
        .replace("        other()\n", &"        other()\n".repeat(90));
    assert_matches_cpython(&long_jump, "branch_tail.py", &LEVELS);

    for source in [
        r"def enclosing():
    cell = None
    def f(x, y, value, manager):
        with manager:
            if x:
                if y:
                    work()
                nonlocal cell
                assert value
            else:
                other()
            after()
    return f
",
        r"async def f(x, y, value):
    try:
        if x:
            if y:
                work()
            assert value
        else:
            other()
        after()
    finally:
        cleanup()
",
        r"def f(x, y, items):
    if x:
        for item in items:
            if y:
                break
            work()
        else:
            other()
        global marker
    else:
        other()
    after()
",
        r"def f(x, y, z):
    if x:
        while y:
            if z:
                break
            work()
        else:
            other()
        global marker
    else:
        other()
    after()
",
    ] {
        assert_matches_cpython(source, "branch_tail.py", &LEVELS);
    }
}

#[test]
fn erased_branch_assertions_preserve_copied_footer_donors() {
    for (case, source) in [
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
            assert value
        else:
            assert choice
            value = build() if flag else choice
        assert flag
    else:
        return build()
",
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
            assert value
        else:
            assert choice
            value = build()
        assert flag
    else:
        return build()
",
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
        else:
            value = build() if flag else choice
        assert flag
    else:
        return build()
",
        r"def example(flag, choice, build):
    if flag:
        if choice:
            value = build()
        else:
            value = build()
        assert flag
    else:
        return build()
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("copied_binary_footer_{case}.py"), &LEVELS);
    }
}

#[test]
fn nested_leaf_returns_preserve_repeated_scope_footer() {
    for (case, source) in [
        r"def f(a, b, c):
    if a:
        assert a
    if b:
        if c:
            return None
",
        r"def f(a, b, c):
    if a:
        assert a
    if b:
        if c:
            return 1
",
        r"def f(a, b, c):
    if a:
        assert a
    if b:
        if c:
            return a
",
        r"def f(a, b, c):
    if a:
        pass
    if b:
        if c:
            return None
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(source, &format!("nested_leaf_footer_{case}.py"), &LEVELS);
    }
}

#[test]
fn normal_finalizer_none_marker_reuses_original_local_load_owner() {
    let source = r"async def f(outer, inner):
    async with outer:
        resource = None
        try:
            resource = await setup()
            await prepare(resource)
            async with inner:
                value = None
                pending = await begin()
                try:
                    await work()
                    response = await read()
                    value = consume(response)
                finally:
                    remaining = await finish(pending)
            assert value is not None
            check(remaining, value)
            assert response
        finally:
            if resource is not None:
                try:
                    await cleanup(resource)
                except Error:
                    report()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("            check(remaining, value)\n", ""),
        source.replace("            assert value is not None\n", "").replace("            assert response\n", ""),
        source.replace("                try:\n                    await cleanup(resource)\n                except Error:\n                    report()\n", "                await cleanup(resource)\n"),
    ].into_iter().enumerate() {
        assert_matches_cpython(&source, &format!("normal_finalizer_local_load_owner_{index}.py"), &LEVELS);
    }
}

#[test]
fn erased_finalizer_arms_preserve_iterator_backedges() {
    let source = r#"async def sample(items, work, cleanup, flag):
    try:
        await work()
    finally:
        cleanup()
        for item in items:
            assert item.ready
            if flag in {"first", "second"} and item.code == 7:
                assert item.closed is None
            else:
                assert item.closed is not None
        assert flag
"#;
    for (name, source) in [
        ("compound", source.to_owned()),
        (
            "simple",
            source.replace("flag in {\"first\", \"second\"} and item.code == 7", "flag"),
        ),
        (
            "synchronous",
            source.replace("async def", "def").replace("await ", ""),
        ),
        (
            "return_copy",
            source.replace(
                "        await work()",
                "        await work()\n        if flag:\n            return",
            ),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("erased_finalizer_iterator_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_finalizer_iterator_preserves_pending_return_value() {
    let source = r#"async def sample(items, work, cleanup, flag):
    try:
        await work()
        if flag:
            return flag
    finally:
        cleanup()
        for item in items:
            assert item.ready
            if flag in {"first", "second"} and item.code == 7:
                assert item.closed is None
            else:
                assert item.closed is not None
        assert flag
"#;
    for (name, source) in [
        ("name", source.to_owned()),
        ("call", source.replace("return flag", "return work()")),
        (
            "await",
            source.replace("return flag", "return await work()"),
        ),
        (
            "synchronous",
            source.replace("async def", "def").replace("await ", ""),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("erased_finalizer_preserved_value_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_finalizer_iterator_preserves_unwound_manager_return() {
    let source = r#"async def sample(items, manager, work, cleanup, flag):
    try:
        async with manager():
            await work()
            if flag:
                return
    finally:
        cleanup()
        for key, item in items:
            assert item.ready
            if flag in {"first", "second"} and item.code == 7:
                assert item.closed is None
            else:
                assert item.closed is not None
        assert flag
"#;
    for (name, source) in [
        ("bare", source.to_owned()),
        ("name_target", source.replace("for key, item", "for item")),
        ("sync_manager", source.replace("async with", "with")),
        (
            "value",
            source.replace("                return\n", "                return flag\n"),
        ),
        (
            "two",
            source.replace("async with manager():", "async with manager(), manager():"),
        ),
        (
            "three",
            source.replace("async with manager():", "async with manager(), manager(), manager():"),
        ),
        (
            "four",
            source.replace("async with manager():", "async with manager(), manager(), manager(), manager():"),
        ),
        (
            "nested",
            source.replace(
                "        async with manager():\n            await work()\n            if flag:\n                return\n",
                "        async with manager():\n            async with manager():\n                await work()\n                if flag:\n                    return\n",
            ),
        ),
    ] {
        assert_matches_cpython(
            &source,
            &format!("erased_finalizer_unwound_manager_{name}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_iterator_else_stays_inside_enclosing_finally() {
    let source = r"def f(items, expected, count):
    try:
        result = prepare()
    except Failure:
        raise
    else:
        if count > 1:
            for item in items:
                value = item.read()
                current = extract(value)
                if expected != checksum(current):
                    report(expected, current)
                else:
                    assert result == value
    finally:
        close()
";
    for (index, source) in [
        source.to_owned(),
        source.replace(
            "                else:\n                    assert result == value\n",
            "                else:\n                    pass\n",
        ),
        source.replace(
            "                    assert result == value\n",
            "                    consume(value)\n",
        ),
        source.replace(
            "                    assert result == value\n",
            "                    assert result == value\n                finish(item)\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("finally_iterator_else_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn protected_iterator_tail_preserves_completed_nested_conditionals() {
    let source = r"def sample(items, flag, work, finish):
    try:
        for item in items:
            item = work(item)
            assert flag
            if item is None:
                if flag:
                    value = work(item)
                    assert value
                    value = work(value)
                else:
                    assert flag
            else:
                assert flag
        assert flag
    finally:
        finish()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("                else:\n                    assert flag\n", ""),
        source.replace(
            "                else:\n                    assert flag\n",
            "                else:\n                    work(flag)\n",
        ),
        source.replace("            else:\n                assert flag\n", ""),
        source.replace(
            "                if flag:\n                    value = work(item)\n                    assert value\n                    value = work(value)\n                else:\n                    assert flag\n",
            "                work(item)\n",
        ),
        source.replace("    finally:\n        finish()\n", "    except LookupError:\n        pass\n"),
        source.replace("                if flag:\n", "                if item is flag:\n"),
        source.replace(
            "                    value = work(item)\n",
            "                    assert item\n                    value = work(item)\n",
        ),
        source.replace("            if item is None:\n", "            if item is not None:\n"),
        source.replace(
            "            else:\n                assert flag\n",
            "            else:\n                work(flag)\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_iterator_nested_conditionals_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_conditional_join_precedes_named_awaitable_finalizer() {
    let source = r"async def f(resource, cleanup):
    try:
        await work(resource)
        if resource.expected is not None:
            assert resource.value is not None
            assert resource.expected in resource.value
        else:
            assert resource.value is None
    finally:
        await cleanup
";
    for (index, source) in [
        source.to_owned(),
        source.replace("f(resource, cleanup)", "f(resource)"),
        source.replacen(
            "    try:\n",
            "    def capture():\n        return cleanup\n    try:\n",
            1,
        ),
        source.replace(
            "        await cleanup\n",
            "        if resource:\n            await cleanup\n        await another\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("named_awaitable_finalizer_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_iterator_arms_stay_inside_enclosing_except_and_finally() {
    let source = r"def normalize(groups, enabled):
    for group in groups:
        try:
            for value in group:
                item = prepare(value)
                consume(item)
                if enabled:
                    assert item
                else:
                    assert group
            consume(group)
        except LookupError:
            recover()
        finally:
            cleanup(group)
";
    for (index, source) in [
        source.to_owned(),
        source.replace("        except LookupError:\n            recover()\n", ""),
        source.replace(
            "item = prepare(value)",
            "item = prepare(value) if enabled else fallback(value)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("protected_erased_iterator_arms_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn managed_coroutine_erased_shared_arm_preserves_copied_loop_exits() {
    let source = r"async def f(items, outer, manager, enabled, extra):
    async with outer:
        with manager:
            for item in items:
                if item.kind:
                    if enabled or extra:
                        assert item.valid
                    else:
                        value = item.value
                        assert value
                else:
                    consume(item)
";
    for source in [
        source.to_owned(),
        source.replace("async with outer:", "with outer:"),
        source.replace("with manager:", "with manager, other:"),
        source.replace("enabled or extra", "enabled or extra or item.third"),
    ] {
        assert_matches_cpython(&source, "managed_coroutine_shared_loop_exit.py", &LEVELS);
    }
}

#[test]
fn coroutine_erased_join_before_nested_iterator() {
    let source = r"async def f(items, later):
    with manager():
        await prepare()
        for item in items:
            if item.ready:
                assert item.valid
            else:
                assert item.other
            for entry in later:
                consume(entry)
";
    for source in [
        source.to_owned(),
        source.replace("in later:", "in source.items:"),
        source.replace("in later:", "in entries():"),
        source.replace("with manager():", "with manager(), manager():"),
    ] {
        assert_matches_cpython(&source, "coroutine_iterator_successor.py", &LEVELS);
    }
}

#[test]
fn protected_iterator_tail_accepts_completed_conditional_work() {
    let source = r"async def f(items, outer, manager, config, key, legacy):
    async with outer:
        with manager:
            for item in items:
                if item.kind:
                    consume(item)
                else:
                    value = item.value
                    if config is not None and key in config:
                        start = 1 if item.first else 6
                        if legacy:
                            assert value == start
                        else:
                            assert value != start
                    else:
                        assert value
";
    for source in [
        source.to_owned(),
        source.replace("1 if item.first else 6", "first if item.first else second"),
        source.replace(
            "1 if item.first else 6",
            "left() if item.first else right()",
        ),
        source.replace("if legacy:", "if item.legacy:"),
    ] {
        assert_matches_cpython(&source, "protected_iterator_completed_work.py", &LEVELS);
    }
}

#[test]
fn erased_match_case_threads_through_enclosing_branch() {
    for calls in [2, 30] {
        for pattern in ["A() | B()", "A()"] {
            let mut source = format!(
                "def f(subject, enabled, emit):\n    if enabled:\n        match subject:\n            case {pattern}:\n                assert False\n            case C():\n"
            );
            for index in 0..calls {
                writeln!(source, "                emit({index})").unwrap();
            }
            source.push_str(
                "            case _:\n                emit(subject)\n    else:\n        emit(enabled)\n    emit(subject)\n    emit(enabled)\n    return subject\n",
            );
            assert_matches_cpython(&source, "erased_match_branch.py", &LEVELS);
        }
    }
}

#[test]
fn terminal_nested_erased_if_keeps_finalizer_join() {
    let source = r"async def f(first, second):
    try:
        if first:
            consume(first)
            assert first
        else:
            consume(second)
            assert second
            if second:
                assert first
            else:
                assert second
    finally:
        cleanup()
";
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def"),
        source.replace("    finally:", "            assert first\n    finally:"),
        source.replace(
            "            if second:",
            "            if first:\n                consume(first)\n            elif second:",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("terminal_nested_finalizer_join_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn boolean_call_prefix_keeps_terminal_finalizer_join() {
    let source = r#"async def f(first, second, mapping):
    try:
        if first:
            consume(first)
            assert first
        else:
            value = coerce(mapping.get("entry") or {})
            assert value
            if second:
                assert first
            else:
                assert second
    finally:
        cleanup()
"#;
    for (index, source) in [
        source.to_owned(),
        source.replace("async def", "def"),
        source.replace(" or {}", " and {}"),
        source.replace("coerce(mapping", "factory.coerce(value=mapping"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("boolean_call_finalizer_prefix_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertions_preserve_populated_iterator_branch_joins() {
    let source = r"def collect(groups):
    result = 0
    for group in groups:
        for left, right in group:
            if left is None or right is None:
                assert left is right
            else:
                result = max(result, abs(left - right))
                assert result >= 0
    return result
";
    for source in [
        source.to_owned(),
        source.replace(
            "                result = max(result, abs(left - right))\n                assert result >= 0",
            "                assert left is not None\n                result = max(result, abs(left - right))",
        ),
        source.replace(
            "                assert result >= 0",
            "                assert result >= 0\n                consume(result)",
        ),
        source.replace("result = max(result, abs(left - right))", "consume(left, right)"),
        source.replace(" or right is None", " and right is None"),
        source.replace(" or right is None", ""),
        source.replace("                assert result >= 0\n", ""),
        source.replace("assert left is right", "consume(left, right)"),
    ] {
        assert_matches_cpython(&source, "populated_iterator_branch_join.py", &LEVELS);
    }
}

#[test]
fn erased_disjunction_preserves_nested_iterator_branch_joins() {
    let branch = r"if probe.accepts((1, 2)) or value == 0 or flag:
    assert all(element != 7 for element in group)
else:
    assert 1 not in group
    assert 2 not in group
    if value == 3:
        assert 4 in group
";
    let indent = |body: &str| {
        body.lines().fold(String::new(), |mut result, line| {
            writeln!(result, "    {line}").unwrap();
            result
        })
    };
    for (depth, branch) in [
        (2, branch.to_owned()),
        (2, branch.replace(" or flag", "")),
        (0, branch.to_owned()),
        (1, branch.to_owned()),
        (
            2,
            branch.replace("    if value == 3:\n        assert 4 in group\n", ""),
        ),
        (
            2,
            branch.replace(
                "assert all(element != 7 for element in group)",
                "probe.consume(value)",
            ),
        ),
    ]
    .into_iter()
    .chain(
        [
            "probe.accepts(5) and not probe.accepts(6)",
            "probe.accepts(5) or not probe.accepts(6)",
            "probe.accepts(5)",
        ]
        .map(|value| {
            (
                2,
                format!("assert 5 in group\nassert 6 in group\nflag = {value}\n{branch}"),
            )
        }),
    ) {
        let body = match depth {
            0 => branch,
            1 => format!("for value in group:\n{}", indent(&branch)),
            2 => format!(
                "for group in batches:\n{}",
                indent(&format!("for value in group:\n{}", indent(&branch))),
            ),
            _ => unreachable!(),
        };
        let source = format!(
            "def collect(batches, group, value, probe, flag):\n{}    return None\n",
            indent(&body),
        );
        assert_matches_cpython(&source, "erased_disjunction_loop_join.py", &LEVELS);
    }
}

#[test]
fn erased_protected_iterator_suffix_preserves_tail_condition() {
    let source = r"def nested_erased_tail(items, enabled, ready, sink, cleanup):
    try:
        for item in items:
            sink(item)
            if enabled:
                first = sink(item)
                second = sink(item)
                if ready == 0:
                    sink(first)
                assert ready
    finally:
        cleanup()
        cleanup()
";
    let unprotected = r"def nested_unprotected_tail(items, enabled, ready, sink):
    for item in items:
        sink(item)
        if enabled:
            first = sink(item)
            second = sink(item)
            if ready == 0:
                sink(first)
            assert ready
";
    let conditional = "                if ready == 0:\n                    sink(first)\n";
    let nested_cleanup = "                try:\n                    if ready == 0:\n                        sink(first)\n                    assert ready\n                finally:\n                    cleanup()\n";
    let forms = [
        source.to_owned(),
        source
            .replace(
                "nested_erased_tail(items, enabled, ready, sink, cleanup)",
                "nested_store_tail(items, enabled, ready, output, sink, cleanup)",
            )
            .replace("sink(first)", "output[0] = first"),
        source
            .replace("nested_erased_tail", "nested_plain_tail")
            .replace("                assert ready\n", ""),
        source
            .replace("nested_erased_tail", "nested_live_tail")
            .replace("assert ready", "sink(ready)"),
        unprotected.to_owned(),
        source
            .replace("nested_erased_tail", "flat_erased_tail")
            .replace(conditional, ""),
        source
            .replace("nested_erased_tail", "nested_cleanup_tail")
            .replace(
                &format!("{conditional}                assert ready\n"),
                nested_cleanup,
            ),
        source
            .replace("nested_erased_tail", "iterator_else_tail")
            .replace(
                "    finally:\n",
                "        else:\n            sink(ready)\n    finally:\n",
            ),
    ];
    for (index, source) in forms.into_iter().enumerate() {
        assert_matches_cpython(
            &source,
            &format!("erased_protected_iterator_suffix_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn erased_assertions_preserve_populated_scope_footer() {
    for source in [
        r"def finish(left, right, sink):
    if left or right:
        assert left
    else:
        sink(right)
        assert right
",
        r"def finish(left, right, sink):
    if left and right:
        assert right
        sink(left)
    else:
        assert left
",
    ] {
        assert_matches_cpython(source, "populated_scope_footer.py", &LEVELS);
    }
}

#[test]
fn filtered_dict_store_preserves_protected_iterator_backedge() {
    let source = r"def collect(batches, selected, produce, accept, sink, cleanup):
    try:
        for batch in batches:
            for key, value in batch:
                if key not in selected and value in selected:
                    snapshot = {slot: data for slot, data in produce(batch)}
                    assert snapshot
                    selected[key] = value
                    sink(snapshot)
                else:
                    assert key != value
    finally:
        cleanup()
";
    let filtered = source.replace("produce(batch)}", "produce(batch) if data}");
    let unprotected = filtered
        .replace("    try:\n", "")
        .replace("    finally:\n        cleanup()\n", "")
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    for source in [
        source.to_owned(),
        filtered.clone(),
        filtered.replace(" if data}", " if data is not None}"),
        filtered.replace(" if data}", " if not data}"),
        filtered.replace(" if data}", " if accept(data)}"),
        filtered.replace(" if data}", " if slot in selected}"),
        filtered.replace("assert key != value", "sink(key)"),
        unprotected,
    ] {
        assert_matches_cpython(&source, "filtered_protected_iterator.py", &LEVELS);
    }
}

#[test]
fn completed_set_comparison_preserves_erased_iterator_edges() {
    for (index, source) in [
        r"async def erased_sets(batches, left, right, finish):
    try:
        for batch in batches:
            if {item.code for item in left} == {item.code for item in right}:
                assert batch == 0
            else:
                assert batch > 0
    finally:
        finish()
",
        r"async def nested_erased_sets(batches, left, right, ready, report, finish):
    try:
        for batch in batches:
            if ready:
                if {item.code for item in left} == {item.code for item in right}:
                    assert batch == 0
                else:
                    assert batch > 0
            else:
                report(batch)
    finally:
        finish()
",
        r"def synchronous_sets(batches, left, right, finish):
    try:
        for batch in batches:
            if {item.code for item in left} == {item.code for item in right}:
                assert batch == 0
            else:
                assert batch > 0
    finally:
        finish()
",
        r"async def unprotected_sets(batches, left, right):
    for batch in batches:
        if {item.code for item in left} == {item.code for item in right}:
            assert batch == 0
        else:
            assert batch > 0
",
        r"async def ordinary_operands(batches, left, right, finish):
    try:
        for batch in batches:
            if left == right:
                assert batch == 0
            else:
                assert batch > 0
    finally:
        finish()
",
        r"async def live_alternate(batches, left, right, report, finish):
    try:
        for batch in batches:
            if {item.code for item in left} == {item.code for item in right}:
                assert batch == 0
            else:
                report(batch)
    finally:
        finish()
",
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            source,
            &format!("completed_set_comparison_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn copied_return_cleanup_preserves_earlier_loop_exclusions() {
    let source = r"def reduced_return(choice, item, owner, operation, cleanup, recover):
    value = None
    try:
        try:
            if choice:
                return {'result': operation(item)}
            value = operation(item)
        finally:
            for entry in (owner.left, owner.middle, owner.right):
                if entry is not None:
                    cleanup(entry)
    except ValueError:
        return None
    except TypeError:
        return item
    return value
";
    let unprotected = r"def no_outer_handler(choice, item, owner, operation, cleanup, recover):
    value = None
    try:
        if choice:
            return {'result': operation(item)}
        value = operation(item)
    finally:
        for entry in (owner.left, owner.middle, owner.right):
            if entry is not None:
                cleanup(entry)
    return value
";
    let cleanup_loop = r"            for entry in (owner.left, owner.middle, owner.right):
                if entry is not None:
                    cleanup(entry)
";
    for (index, source) in [
        source.to_owned(),
        unprotected.to_owned(),
        source.replace("reduced_return", "no_early_return").replace(
            "return {'result': operation(item)}",
            "value = {'result': operation(item)}",
        ),
        source
            .replace("reduced_return", "boolean_guard")
            .replace("entry is not None", "entry"),
        source
            .replace("reduced_return", "linear_cleanup")
            .replace(cleanup_loop, "            cleanup(owner.right)\n"),
    ]
    .into_iter()
    .enumerate()
    {
        assert_matches_cpython(
            &source,
            &format!("copied_return_cleanup_loop_{index}.py"),
            &LEVELS,
        );
    }
}

#[test]
fn nested_iterator_value_return_preserves_cleanup_exit_locations() {
    let source = "def sample(items, action):\n    for outer in items:\n        for inner in items:\n            try:\n                return []\n            finally:\n                try:\n                    action()\n                except Exception:\n                    action()\n    return []\n";
    for (name, source) in [
        ("value", source.to_owned()),
        ("bare", source.replacen("return []", "return", 1)),
        (
            "one_loop",
            source.replace("    for outer in items:", "    if items:"),
        ),
    ] {
        assert_matches_cpython(&source, &format!("nested_iterator_exit_{name}.py"), &LEVELS);
    }
}

#[test]
fn managed_finalizer_loops_preserve_empty_borrow_exit() {
    for manager in ["", "with acquire():", "with acquire(), acquire():"] {
        for prefix in ["def", "async def"] {
            for tail in [
                "",
                "        for item in remaining:\n            pass\n",
                "        for item in remaining:\n            finish(item)\n",
                "        for item in remaining:\n            if failure:\n                finish(item)\n",
            ] {
                let suite = format!(
                    "    try:\n        pass\n    except BaseException:\n        pass\n    finally:\n        for item in groups:\n            try:\n                pass\n            except BaseException as error:\n                failure = failure or error\n{tail}"
                );
                let suite = if manager.is_empty() {
                    suite
                } else {
                    let indented = suite.lines().fold(String::new(), |mut output, line| {
                        writeln!(output, "    {line}").unwrap();
                        output
                    });
                    format!("    {manager}\n{indented}")
                };
                let source = format!(
                    "{prefix} probe(groups, remaining, failure):\n{suite}    if failure:\n        raise failure\n"
                );
                assert_matches_cpython(&source, "managed_finalizer_loops.py", &LEVELS);
            }
        }
    }
}
