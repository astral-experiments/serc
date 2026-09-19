use crate::common;

const SOURCE: &str = r#"def run(plan, execute, preflight_only, values, stamp, describe, kind, text, formatter, sink):
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
        write_error(f"ERROR: {kind_global(error).__name__}: {formatter.safe(text_global(error))}", file=sink.stderr)
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
fn named_return_three_global_folds() {
    common::assert_matches_cpython_marshal(SOURCE, "named_return_three_global_folds_target.py");
}

#[test]
fn named_return_one_global_fold_control() {
    let source = SOURCE
        .replace("kind_global(error)", "kind(error)")
        .replace("text_global(error)", "text(error)");
    common::assert_matches_cpython_marshal(&source, "named_return_one_global_fold_control.py");
}
