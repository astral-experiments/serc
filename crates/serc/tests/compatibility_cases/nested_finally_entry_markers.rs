use crate::common;

#[test]
fn matches_cpython_nested_finally_entry_after_terminating_handlers() {
    for (header, prefix) in [
        ("def", ""),
        ("async def", "    await suspend()\n"),
        ("def", "    yield value\n"),
        ("async def", "    yield value\n"),
    ] {
        for body in [
            "        call()\n",
            "        value = call()\n",
            "        for item in value:\n            call()\n",
        ] {
            for handler in ["raise", "return"] {
                let source = format!(
                    "{header} f(value):\n{prefix}    try:\n{body}    except Exception:\n        {handler}\n    finally:\n        try:\n            finish()\n        except ValueError:\n            pass\n    finish(value)\n"
                );
                common::assert_matches_cpython_marshal(&source, "nested_finally_entry_markers.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_finally_entry_shared_and_empty_controls() {
    for body in [
        "        pass\n",
        "        if value:\n            call()\n",
        "        if value and ready():\n            return\n",
    ] {
        for handler in ["raise", "pass"] {
            let source = format!(
                "def f(value):\n    try:\n{body}    except Exception:\n        {handler}\n    finally:\n        try:\n            finish()\n        except ValueError:\n            pass\n    finish(value)\n"
            );
            common::assert_matches_cpython_marshal(&source, "nested_finally_entry_controls.py");
        }
    }
}

#[test]
fn matches_cpython_nested_copied_cleanup_try_pop_blocks() {
    for (filename, source) in [
        (
            "nested_cleanup_try_return.py",
            r"def probe(inner, outer, value):
    try:
        try:
            return value
        finally:
            try:
                inner.close()
            except Exception:
                pass
    finally:
        try:
            outer.close()
        except Exception:
            pass
",
        ),
        (
            "nested_named_cleanup_try_return.py",
            r"def probe(inner, outer, report):
    try:
        try:
            return 0
        finally:
            try:
                result = inner()
            except Exception as error:
                report(error)
    finally:
        try:
            outer()
        except Exception as error:
            report(error)
",
        ),
        (
            "nested_async_cleanup_try_return.py",
            r"async def probe(manager, inner, outer, value):
    async with manager():
        try:
            try:
                return value
            finally:
                try:
                    await inner.aclose()
                except Exception:
                    pass
        finally:
            try:
                outer.close()
            except Exception:
                pass
",
        ),
        (
            "single_cleanup_try_return_control.py",
            r"def probe(inner, value):
    try:
        return value
    finally:
        try:
            inner.close()
        except Exception:
            pass
",
        ),
        (
            "nested_cleanup_try_tail_control.py",
            r"def probe(inner, outer, observe, value):
    try:
        try:
            return value
        finally:
            try:
                inner.close()
            except Exception:
                pass
            observe()
    finally:
        try:
            outer.close()
        except Exception:
            pass
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
