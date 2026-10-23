use crate::common;

#[test]
fn matches_cpython_resume_depth_in_terminal_named_handlers() {
    for definition in ["def", "async def"] {
        for (first, second) in [
            ("yield 1", "yield 2"),
            ("yield 1", "return"),
            ("return", "yield 2"),
        ] {
            let source = format!(
                "{definition} f(ready):\n    try:\n        work()\n    except Error as error:\n        if ready:\n            {first}\n        else:\n            {second}\n    except Exception:\n        yield 3\n"
            );
            common::assert_matches_cpython_marshal(&source, "named_handler_resume_depth.py");
        }
    }
}

#[test]
fn matches_cpython_resume_depth_in_copied_finalizers() {
    for (definition, iterator) in [("def", "for"), ("async def", "async for")] {
        for exit in ["break", "continue"] {
            for finalbody in [
                "yield item\n",
                "try:\n    yield item\nfinally:\n    yield 2\n",
            ] {
                let finalbody = finalbody.replace('\n', "\n            ");
                let source = format!(
                    "{definition} f(items, ready):\n    {iterator} item in items:\n        try:\n            if ready:\n                {exit}\n            work()\n        finally:\n            {finalbody}\n"
                );
                common::assert_matches_cpython_marshal(&source, "copied_finalizer_resume_depth.py");
            }
        }
    }
}

#[test]
fn preserves_resume_depth_under_outer_managers_and_inner_handlers() {
    for definition in ["def", "async def"] {
        for source in [
            "with manager:\n    for item in items:\n        try:\n            if ready:\n                continue\n            work()\n        finally:\n            yield item\n",
            "for item in items:\n    try:\n        if ready:\n            continue\n    finally:\n        try:\n            yield item\n        except Error:\n            pass\n",
        ] {
            let body = source.replace('\n', "\n    ");
            let source = format!("{definition} f(items, manager, ready):\n    {body}\n");
            common::assert_matches_cpython_marshal(&source, "protected_finalizer_resume_depth.py");
        }
    }
}

#[test]
fn preserves_resume_depth_in_exception_types_and_group_handlers() {
    for definition in ["def", "async def"] {
        for handler in [
            "except (yield Error): pass",
            "except* (yield Error): pass",
            "except* Error: yield value",
            "except* Error as caught: yield caught",
        ] {
            let source = format!(
                "{definition} f():\n    yield before\n    try: work()\n    {handler}\n    yield after\n"
            );
            common::assert_matches_cpython_marshal(&source, "handler_yield.py");
        }
    }
}
