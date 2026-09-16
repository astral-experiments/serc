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
