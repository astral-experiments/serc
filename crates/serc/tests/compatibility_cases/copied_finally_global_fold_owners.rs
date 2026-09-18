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
