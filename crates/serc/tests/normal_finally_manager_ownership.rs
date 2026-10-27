#[allow(dead_code)]
mod common;

#[test]
fn completed_guard_preserves_manager_owner_on_later_cleanup_exit() {
    for asynchronous in [false, true] {
        for cleanup in [
            "pass",
            "work()",
            "try:\n                    work()\n                except Exception:\n                    handle()",
        ] {
            let definition = if asynchronous { "async def" } else { "def" };
            let suspension = if asynchronous { "await " } else { "" };
            let source = format!(
                "{definition} f(first, second):\n    with manager():\n        try:\n            {suspension}work()\n        except Exception:\n            handle()\n        finally:\n            if first is not None:\n                {cleanup}\n            if second:\n                try:\n                    work()\n                    {suspension}close()\n                except Exception as error:\n                    handle(error)\n"
            );
            common::assert_matches_cpython_marshal(&source, "completed_finally_guard.py");
        }
    }
}

#[test]
fn awaited_guarded_recovery_preserves_manager_owner_on_later_cleanup_exit() {
    let source = r"async def f(first, second):
    with manager():
        try:
            await work()
        except Exception as error:
            handle(error)
        finally:
            if first is not None:
                try:
                    await close()
                except Exception:
                    handle()
            if second:
                try:
                    work()
                    await close()
                except Exception as error:
                    handle(error)
";
    common::assert_matches_cpython_marshal(source, "awaited_finally_guards.py");
}

#[test]
fn a_fallthrough_finally_guard_keeps_the_enclosing_manager_on_its_jump() {
    for (body, guard, cleanup) in [
        ("return value", "ready", "observe(value)"),
        ("return value", "False", "observe(value)"),
        ("observe(value)", "ready", "return value"),
    ] {
        let source = format!(
            "def read(outer, inner, ready, value):\n    with outer:\n        try:\n            with inner:\n                {body}\n        finally:\n            if {guard}:\n                {cleanup}\n"
        );
        common::assert_matches_cpython_marshal(&source, "guarded_manager_exit.py");
    }
}
