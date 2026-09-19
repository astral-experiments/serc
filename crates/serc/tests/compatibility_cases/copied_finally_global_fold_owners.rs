use crate::common;

const SOURCE: &str = r#"def run(plan, execute, preflight_only, values, stamp, describe):
    acquire()
    resource = None
    try:
        prepare()
        if not plan and not execute:
            record({"state": "planned", "time": stamp_global(), "path": describe_global(PATH)})
            return 0
        configure()
        if preflight_only:
            record({"state": "ready", "time": stamp_global(), "path": describe_global(PATH)})
            return 0
        resource = create()
        for value in values:
            step(resource, value)
        finish(stamp_global(), describe_global(PATH))
        return 0
    except Exception as error:
        report(error)
        try:
            record(error)
        except Exception:
            pass
        return 1
    finally:
        if resource is not None:
            try:
                resource.stop()
            except Exception:
                pass
        release()
"#;

#[test]
fn copied_finally_three_global_folds() {
    common::assert_matches_cpython_marshal(SOURCE, "copied_finally_three_global_folds_target.py");
}

#[test]
fn copied_finally_one_global_fold_control() {
    let source = SOURCE
        .replace("stamp_global()", "stamp()")
        .replace("describe_global(PATH)", "describe(PATH)");
    common::assert_matches_cpython_marshal(&source, "copied_finally_one_global_fold_control.py");
}

#[test]
fn copied_return_imported_call_reuses_original_protected_array() {
    for (filename, source) in [
        (
            "imported_module_return_finally_parent_owner_multiline_keywords_and_text.py",
            r#"import service

def f(records, directory, options, submitted, counters):
    pending = {}
    try:
        for record in records:
            pending[record] = submit(record)
        while pending:
            for ref in drain(pending):
                handle(ref)
            report(pending)
        report_status(
            directory=directory,
            options=options,
            status="finished",
            submitted=submitted,
            pending=0,
            counters=counters,
        )
        report_progress(
            options=options,
            submitted=submitted,
            pending=0,
            counters=counters,
        )
        report()
        report(
            "Finished work. "
            f"Wrote {len(good(directory))} results and "
            f"{len(bad(directory))} failures under {directory}."
        )
        return 0
    except KeyboardInterrupt:
        for ref in pending:
            service.cancel(ref, force=True)
        if options is not None:
            report(options)
        return 130
    except Exception as error:
        if options is not None:
            report(error)
        raise
    finally:
        if service.is_initialized():
            service.shutdown()
"#,
        ),
        (
            "imported_module_return_finally_parent_owner_keywords_and_text.py",
            r#"import service

def f(records, directory, options, submitted, counters):
    pending = {}
    try:
        for record in records:
            pending[record] = submit(record)
        while pending:
            for ref in drain(pending):
                handle(ref)
            report(pending)
        report_status(directory=directory, options=options, status="finished",
                      submitted=submitted, pending=0, counters=counters)
        report_progress(options=options, submitted=submitted, pending=0, counters=counters)
        report()
        report(
            "Finished work. "
            f"Wrote {len(good(directory))} results and "
            f"{len(bad(directory))} failures under {directory}."
        )
        return 0
    except KeyboardInterrupt:
        for ref in pending:
            service.cancel(ref, force=True)
        if options is not None:
            report(options)
        return 130
    except Exception as error:
        if options is not None:
            report(error)
        raise
    finally:
        if service.is_initialized():
            service.shutdown()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn copied_return_local_truth_preserves_the_authored_return_owner() {
    for (filename, source) in [
        (
            "coroutine_return_local_truth_parent_global_folds.py",
            r"async def f(flag, ready):
    try:
        if ready:
            report(first())
            report(second())
            report(third())
            return
        await work()
    except Exception as error:
        report(error)
    finally:
        if flag:
            await cleanup()
",
        ),
        (
            "coroutine_return_local_truth_parent_short_control.py",
            r"async def f(flag, ready):
    try:
        if ready:
            report(first())
            return
        await work()
    except Exception as error:
        report(error)
    finally:
        if flag:
            await cleanup()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_copied_return_attribute_truth_parent_owners() {
    for (filename, source) in [
        (
            "coroutine_return_attribute_truth_parent_constant_folds.py",
            r#"async def f(value, runner):
    try:
        if value:
            print("!" * 80)
            print(value, "skipping")
            print("!" * 80)
            return
        await work()
    except Exception as error:
        print(error)
    finally:
        if runner.manager and runner.manager.session is not None:
            await runner.manager.session.stop()
        await runner.close()
"#,
        ),
        (
            "coroutine_return_attribute_truth_parent_literal_control.py",
            r#"async def f(value, runner):
    try:
        if value:
            print("skipping")
            print(value, "skipping")
            print("skipping")
            return
        await work()
    except Exception as error:
        print(error)
    finally:
        if runner.manager and runner.manager.session is not None:
            await runner.manager.session.stop()
        await runner.close()
"#,
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
