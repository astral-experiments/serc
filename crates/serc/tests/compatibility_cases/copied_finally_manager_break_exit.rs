use crate::common;

#[test]
fn matches_cpython_marshal_for_copied_finally_manager_break_exit() {
    for body in [
        "            if flag:\n                break\n            work(item)\n",
        "            work(item)\n        except Error as error:\n            if flag:\n                break\n            report(error)\n",
    ] {
        for prefix in ["", "            prepare(item)\n"] {
            for following in ["", "    finish()\n"] {
                let source = format!(
                    "def f(items, flag, manager):\n    for item in items:\n        try:\n{body}        finally:\n{prefix}            with manager:\n                release(item)\n{following}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "copied_finally_manager_break_exit.py",
                );
            }
        }
    }
}

#[test]
fn preserves_generator_finally_owners_after_manager_break_cleanup() {
    for (filename, source) in [
        (
            "async_generator_manager_break_finally_truth.py",
            r"async def f(manager, work, flag, cleanup):
    while True:
        try:
            async with manager:
                yield await work()
                break
        except Exception:
            await work()
        finally:
            if flag:
                cleanup()
",
        ),
        (
            "generator_manager_break_finally_method.py",
            r"def f(manager, resource, value):
    while True:
        try:
            with manager:
                yield value
                break
        except (ValueError, OSError):
            continue
        finally:
            if resource.exists():
                resource.unlink()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
