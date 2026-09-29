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
fn copied_finally_one_global_fold_control() {
    let source = SOURCE
        .replace("stamp_global()", "stamp()")
        .replace("describe_global(PATH)", "describe(PATH)");
    common::assert_matches_cpython_marshal(&source, "copied_finally_one_global_fold_control.py");
}

#[test]
fn matches_cpython_manager_return_attribute_guard_pop_slots() {
    for calls in 0..=6 {
        let arguments = if calls == 0 {
            "value".to_owned()
        } else {
            (0..calls)
                .map(|index| format!("work{index}()"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let source = format!(
            "def f(self, flag, value):\n    with self.lock:\n        try:\n            if flag:\n                chosen = 1\n            else:\n                chosen = 2\n            prepare({arguments})\n            return Result.make(value)\n        except Exception:\n            self.reset()\n            raise\n        finally:\n            if not self.reuse:\n                self.reset()\n"
        );
        common::assert_matches_cpython_marshal(&source, "manager_attribute_guard_pop_slots.py");
    }
}
