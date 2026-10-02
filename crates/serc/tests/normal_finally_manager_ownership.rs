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
