async def run(*, request, config, context, records, session=None, cancelled=None):
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
