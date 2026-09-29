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
