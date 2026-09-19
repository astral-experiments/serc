use crate::common;

#[test]
fn matches_cpython_marshal_for_ordinary_return_unpack_target() {
    let source = r#"async def collect_attempt(config, callback):
    connection = None
    resource = None
    result = {}
    try:
        connection = await open_connection(config)
        resource = await open_resource(connection)
        await resource.__aenter__()
        result["seen"] = await wait_for_start(resource)
        if not result["seen"]:
            result["reason"] = "not_started"
            return result
        result["ready"] = await wait_for_ready(connection)
        result["complete"] = finish_state(result)
        result["completed"] = now()
        emit("complete", **result)
        return result
    except BaseException as error:
        result["error"] = format_error(error)
        result["traceback"] = traceback.format_exc(limit=8)
        result["valid"] = False
        result["reason"] = "error"
        emit("error", **result)
        return result
    finally:
        if resource is not None:
            try:
                await resource.__aexit__(None, None, None)
                result["resource_closed"] = True
            except BaseException as error:
                result["resource_closed"] = False
                result["resource_error"] = format_error(error)
        if connection is not None:
            result["close_started"] = now()
            try:
                await connection.close()
                result["close_success"] = True
            except BaseException as error:
                result["close_success"] = False
                result["close_error"] = format_error(error)
            result["close_ended"] = now()
        emit("final", **result)
"#;
    common::assert_matches_cpython_marshal(source, "ordinary_return_unpack_target.py");
}

#[test]
fn matches_cpython_marshal_for_ordinary_return_unpack_local_callback_control() {
    let source = r#"async def collect_attempt(config, callback):
    connection = None
    resource = None
    result = {}
    try:
        connection = await open_connection(config)
        resource = await open_resource(connection)
        await resource.__aenter__()
        result["seen"] = await wait_for_start(resource)
        if not result["seen"]:
            result["reason"] = "not_started"
            return result
        result["ready"] = await wait_for_ready(connection)
        result["complete"] = finish_state(result)
        result["completed"] = now()
        callback("complete", **result)
        return result
    except BaseException as error:
        result["error"] = format_error(error)
        result["traceback"] = traceback.format_exc(limit=8)
        result["valid"] = False
        result["reason"] = "error"
        emit("error", **result)
        return result
    finally:
        if resource is not None:
            try:
                await resource.__aexit__(None, None, None)
                result["resource_closed"] = True
            except BaseException as error:
                result["resource_closed"] = False
                result["resource_error"] = format_error(error)
        if connection is not None:
            result["close_started"] = now()
            try:
                await connection.close()
                result["close_success"] = True
            except BaseException as error:
                result["close_success"] = False
                result["close_error"] = format_error(error)
            result["close_ended"] = now()
        emit("final", **result)
"#;
    common::assert_matches_cpython_marshal(
        source,
        "ordinary_return_unpack_local_callback_control.py",
    );
}

#[test]
fn matches_cpython_marshal_for_poll_conditional_owner_target() {
    let source = r#"def run():
    context = Context.create()
    try:
        context.command("setup", "--root", str(context.root), "--count", "1")
        config = codec.decode(
            context.command(
                "--actor",
                "actor-a",
                "acquire",
                "--root",
                str(context.root),
                "--format",
                "data",
                "--id",
                "0",
            ).output
        )
        context.set_stamp(config["name"], BASELINE)
        process = context.spawn(
            "watch",
            config["name"],
            "--root",
            str(context.root),
            "--actor",
            "actor-a",
            "--token",
            config["token"],
            "--duration",
            "1800",
            "--interval",
            "1",
            "--limit",
            "2",
            "--format",
            "data",
        )
        try:
            ready = wait_until(
                lambda: (
                    not bool(context.row(config["name"]).get("expired"))
                    and str(context.row(config["name"]).get("updated") or "") != BASELINE
                ),
                timeout_seconds=5,
            )
            require(ready, "operation did not finish")
            payload = codec.decode(
                context.checked_command(
                    "refresh",
                    "--root",
                    str(context.root),
                    "--format",
                    "data",
                    "--age",
                    "0",
                    "--strict",
                ).output
            )
            require(not bool(payload.get("changed")), "unexpected update")
            output, error = process.communicate(timeout=5)
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
        require(not error.strip(), f"operation reported: {error.strip()}")
        records = parse_lines(output)
        row = context.row(config["name"])
        require(not bool(row.get("expired")), "record expired")
        return {
            "scenario": "poll_cleanup",
            "ok": True,
            "records": len(records),
            "reason": payload["reason"],
            "transitions": ["ready", "updated"],
        }
    finally:
        context.cleanup()
"#;
    common::assert_matches_cpython_marshal(source, "poll_conditional_owner_target.py");
}

#[test]
fn matches_cpython_marshal_for_poll_conditional_owner_reversed_control() {
    let source = r#"def run():
    context = Context.create()
    try:
        context.command("setup", "--root", str(context.root), "--count", "1")
        config = codec.decode(
            context.command(
                "--actor",
                "actor-a",
                "acquire",
                "--root",
                str(context.root),
                "--format",
                "data",
                "--id",
                "0",
            ).output
        )
        context.set_stamp(config["name"], BASELINE)
        process = context.spawn(
            "watch",
            config["name"],
            "--root",
            str(context.root),
            "--actor",
            "actor-a",
            "--token",
            config["token"],
            "--duration",
            "1800",
            "--interval",
            "1",
            "--limit",
            "2",
            "--format",
            "data",
        )
        try:
            ready = wait_until(
                lambda: (
                    not bool(context.row(config["name"]).get("expired"))
                    and str(context.row(config["name"]).get("updated") or "") != BASELINE
                ),
                timeout_seconds=5,
            )
            require(ready, "operation did not finish")
            payload = codec.decode(
                context.checked_command(
                    "refresh",
                    "--root",
                    str(context.root),
                    "--format",
                    "data",
                    "--age",
                    "0",
                    "--strict",
                ).output
            )
            require(not bool(payload.get("changed")), "unexpected update")
            output, error = process.communicate(timeout=5)
        finally:
            if None is process.poll():
                process.terminate()
                process.wait(timeout=5)
        require(not error.strip(), f"operation reported: {error.strip()}")
        records = parse_lines(output)
        row = context.row(config["name"])
        require(not bool(row.get("expired")), "record expired")
        return {
            "scenario": "poll_cleanup",
            "ok": True,
            "records": len(records),
            "reason": payload["reason"],
            "transitions": ["ready", "updated"],
        }
    finally:
        context.cleanup()
"#;
    common::assert_matches_cpython_marshal(source, "poll_conditional_owner_reversed_control.py");
}

#[test]
fn matches_cpython_marshal_for_named_computed_return_ternary_owner_target() {
    let source = r#"async def run(*, request, config, context, records, session=None, cancelled=None):
    parsed = None
    status = "success"
    error_text = None
    report = should_report()
    started = clock.now() if report else 0.0
    try:
        result = await Runner.run(
            get_worker(prompt=config.prompt, model=config.model, tools=context.tools),
            input=request.messages,
            context=context,
            session=session,
            max_turns=LIMIT,
        )
        parsed = cast(Output, result.output)
        try:
            parsed = reconcile(output=parsed, records=records)
        except InvalidResult as issue:
            status = "failed"
            error_text = repr(issue)
            if report:
                report_failure(
                    run_id=request.run_id,
                    model=config.model,
                    error=issue,
                    started_at=started,
                )
            return Result(
                status="stalled",
                reason="mismatch",
                loaded=list(context.loaded),
                declared=list(context.declared),
                completed=list(context.completed),
                pass_count=1,
                call_count=records.call_count,
                results=list(records.results),
                updates=dict(context.cache),
            )
        parsed = merge(output=parsed, facts=context.facts)
        if report:
            report_success(
                run_id=request.run_id,
                model=config.model,
                result=result,
                output=parsed,
                started_at=started,
            )
        return Result(status="final", output=parsed)
    except InputChanged as issue:
        status = "superseded"
        error_text = repr(issue)
        return stalled(reason="changed", context=context, records=records)
    except Cancelled as issue:
        status = "cancelled"
        error_text = repr(issue)
        return stalled(reason="cancelled", context=context, records=records)
    except Exception as issue:
        if caused_by(issue, InputChanged):
            status = "superseded"
            error_text = repr(issue)
            return stalled(reason="changed", context=context, records=records)
        if cancelled is not None and cancelled():
            status = "cancelled"
            error_text = repr(issue)
            return stalled(reason="cancelled", context=context, records=records)
        status = "failed"
        error_text = repr(issue)
        if report:
            report_failure(
                run_id=request.run_id,
                model=config.model,
                error=issue,
                started_at=started,
            )
        raise
    finally:
        audit = normalize(parsed) if parsed is not None else None
        await executor.execute(
            append_audit,
            Audit(
                source=request.source,
                run_id=request.run_id,
                context=request.context,
                decision=DECISION,
                prompt_id=config.prompt_id,
                prompt_version=config.prompt_version,
                model=config.model,
                input_hash=request.input_hash,
                raw=codec.dumps(audit.dump() if audit is not None else {}),
                parsed=audit.dump(mode="data") if audit is not None else {},
                status=status,
                error=error_text,
                module=MODULE,
                function="run",
                version=config.version,
            ),
            queue=QUEUE,
            timeout=duration(seconds=30),
        )
"#;
    common::assert_matches_cpython_marshal(source, "named_computed_return_ternary_owner_target.py");
}

#[test]
fn matches_cpython_marshal_for_named_computed_return_ternary_owner_reversed_control() {
    let source = r#"async def run(*, request, config, context, records, session=None, cancelled=None):
    parsed = None
    status = "success"
    error_text = None
    report = should_report()
    started = clock.now() if report else 0.0
    try:
        result = await Runner.run(
            get_worker(prompt=config.prompt, model=config.model, tools=context.tools),
            input=request.messages,
            context=context,
            session=session,
            max_turns=LIMIT,
        )
        parsed = cast(Output, result.output)
        try:
            parsed = reconcile(output=parsed, records=records)
        except InvalidResult as issue:
            status = "failed"
            error_text = repr(issue)
            if report:
                report_failure(
                    run_id=request.run_id,
                    model=config.model,
                    error=issue,
                    started_at=started,
                )
            return Result(
                status="stalled",
                reason="mismatch",
                loaded=list(context.loaded),
                declared=list(context.declared),
                completed=list(context.completed),
                pass_count=1,
                call_count=records.call_count,
                results=list(records.results),
                updates=dict(context.cache),
            )
        parsed = merge(output=parsed, facts=context.facts)
        if report:
            report_success(
                run_id=request.run_id,
                model=config.model,
                result=result,
                output=parsed,
                started_at=started,
            )
        return Result(status="final", output=parsed)
    except InputChanged as issue:
        status = "superseded"
        error_text = repr(issue)
        return stalled(reason="changed", context=context, records=records)
    except Cancelled as issue:
        status = "cancelled"
        error_text = repr(issue)
        return stalled(reason="cancelled", context=context, records=records)
    except Exception as issue:
        if caused_by(issue, InputChanged):
            status = "superseded"
            error_text = repr(issue)
            return stalled(reason="changed", context=context, records=records)
        if cancelled is not None and cancelled():
            status = "cancelled"
            error_text = repr(issue)
            return stalled(reason="cancelled", context=context, records=records)
        status = "failed"
        error_text = repr(issue)
        if report:
            report_failure(
                run_id=request.run_id,
                model=config.model,
                error=issue,
                started_at=started,
            )
        raise
    finally:
        audit = normalize(parsed) if None is not parsed else None
        await executor.execute(
            append_audit,
            Audit(
                source=request.source,
                run_id=request.run_id,
                context=request.context,
                decision=DECISION,
                prompt_id=config.prompt_id,
                prompt_version=config.prompt_version,
                model=config.model,
                input_hash=request.input_hash,
                raw=codec.dumps(audit.dump() if audit is not None else {}),
                parsed=audit.dump(mode="data") if audit is not None else {},
                status=status,
                error=error_text,
                module=MODULE,
                function="run",
                version=config.version,
            ),
            queue=QUEUE,
            timeout=duration(seconds=30),
        )
"#;
    common::assert_matches_cpython_marshal(
        source,
        "named_computed_return_ternary_owner_reversed_control.py",
    );
}
