use crate::common;

#[test]
fn matches_cpython_retained_normal_exit_into_finally_loop() {
    for (prefix, manager, produce) in [
        ("", "with", "yield pending[0]"),
        ("async ", "async with", "yield await pending[0]"),
    ] {
        let source = format!(
            "{prefix}def f(manager, pending, exhausted):\n    {manager} manager:\n        try:\n            while pending or not exhausted:\n                if pending:\n                    {produce}\n                    pending.pop()\n                else:\n                    exhausted = True\n        except GeneratorExit:\n            pass\n        finally:\n            for task in pending:\n                task.cancel()\n"
        );
        common::assert_matches_cpython_marshal(&source, "retained_finally_loop.py");
    }
}

#[test]
fn preserves_empty_normal_finally_arrays_and_erased_exit_jumps() {
    for (name, body, cleanup) in [
        (
            "empty_cleanup_array",
            "while pending or not exhausted:\n    consume(pending)\n    exhausted = True",
            "while pending:\n    consume(pending.pop())",
        ),
        (
            "erased_normal_exit",
            "consume(pending)",
            "for task in pending:\n    task.cancel()",
        ),
        (
            "conditional_exit",
            "if exhausted:\n    consume(pending)\nelse:\n    consume(pending)",
            "for task in pending:\n    task.cancel()",
        ),
        (
            "copied_return_cleanup",
            "if exhausted:\n    return pending\nconsume(pending)",
            "for task in pending:\n    task.cancel()",
        ),
    ] {
        let source = format!(
            "def f(pending, exhausted):\n    try:\n        {}\n    except Error:\n        pass\n    finally:\n        {}\n",
            body.replace('\n', "\n        "),
            cleanup.replace('\n', "\n        "),
        );
        common::assert_matches_cpython_marshal(&source, &format!("retained_finally_{name}.py"));
    }
}

#[test]
fn matches_cpython_ownerless_finally_pop_after_unique_while_exit() {
    for (filename, source) in [
        (
            "nested_call_nop_and_while_finally_owner_target.py",
            r"def f(locales, jobs, errors, outputs, metrics, progress, executor):
    for locale in locales:
        pending = {}
        def drain(block=False):
            for future in list(pending):
                try:
                    value, metric = future.result()
                except Exception as exc:
                    errors.append(exc)
                else:
                    if value is None:
                        errors.append(value)
                    elif value:
                        outputs.append(value)
                        if metric is not None:
                            metrics.append(metric)
                finally:
                    progress.update(1)
                    del pending[future]
        try:
            for job in jobs:
                future = executor.submit(work, job)
                pending[future] = True
                drain(block=False)
            while pending:
                drain(block=True)
        except KeyboardInterrupt:
            for future in pending:
                future.cancel()
            try:
                executor.shutdown(wait=False, cancel_futures=True)
            except TypeError:
                executor.shutdown(wait=False)
            progress.close()
            return
        finally:
            try:
                executor.shutdown(wait=True)
            except Exception:
                pass
            try:
                progress.close()
            except Exception:
                pass
        outputs.append(locale)
",
        ),
        (
            "while_finally_owner_with_captured_pending_target.py",
            r"def f(locales, jobs, errors, outputs, metrics, progress, executor):
    for locale in locales:
        pending = {}
        def drain(block=False):
            for future in list(pending):
                outputs.append(future.result())
                del pending[future]
        try:
            for job in jobs:
                future = executor.submit(work, job)
                pending[future] = True
                drain(block=False)
            while pending:
                drain(block=True)
        except KeyboardInterrupt:
            for future in pending:
                future.cancel()
            try:
                executor.shutdown(wait=False, cancel_futures=True)
            except TypeError:
                executor.shutdown(wait=False)
            progress.close()
            return
        finally:
            try:
                executor.shutdown(wait=True)
            except Exception:
                pass
            try:
                progress.close()
            except Exception:
                pass
        outputs.append(locale)
",
        ),
        (
            "nested_call_nop_linear_finally_owner_control.py",
            r"def f(locales, jobs, errors, outputs, metrics, progress, executor):
    for locale in locales:
        pending = {}
        def drain(block=False):
            for future in list(pending):
                try:
                    value, metric = future.result()
                except Exception as exc:
                    errors.append(exc)
                else:
                    if value is None:
                        errors.append(value)
                    elif value:
                        outputs.append(value)
                        if metric is not None:
                            metrics.append(metric)
                finally:
                    progress.update(1)
                    del pending[future]
        try:
            for job in jobs:
                future = executor.submit(work, job)
                pending[future] = True
                drain(block=False)
            drain(block=True)
        except KeyboardInterrupt:
            for future in pending:
                future.cancel()
            try:
                executor.shutdown(wait=False, cancel_futures=True)
            except TypeError:
                executor.shutdown(wait=False)
            progress.close()
            return
        finally:
            try:
                executor.shutdown(wait=True)
            except Exception:
                pass
            try:
                progress.close()
            except Exception:
                pass
        outputs.append(locale)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
