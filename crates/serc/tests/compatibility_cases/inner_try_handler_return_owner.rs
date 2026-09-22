use crate::common;

#[test]
fn matches_cpython_handler_return_after_inner_try() {
    for (scope, suspension) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
        for returned in ["value", "obj.attr"] {
            for conditional in [false, true] {
                let returned = if conditional {
                    format!("            if flag:\n                return {returned}\n")
                } else {
                    format!("            return {returned}\n")
                };
                let source = format!(
                    "{scope} f(value, flag, obj):\n{suspension}    try:\n        work()\n    except RuntimeError:\n        try:\n{returned}        except LookupError:\n            fallback()\n"
                );
                common::assert_matches_cpython_marshal(&source, "inner_try_handler_return.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_handler_returns_before_older_cleanup() {
    let handlers = "try:\n    work()\nexcept ValueError:\n    try:\n        work()\n    except ValueError:\n        try:\n            return value\n        except KeyError:\n            recover()\n";
    for (scope, suspension) in [("def", ""), ("async def", ""), ("def", "    yield value\n")] {
        for older_loop in [None, Some("while flag:"), Some("for value in values:")] {
            let body = if let Some(older_loop) = older_loop {
                format!(
                    "with manager:\n{}",
                    common::indent(&format!(
                        "try:\n{}finally:\n    finish()\n",
                        common::indent(&format!("{older_loop}\n{}", common::indent(handlers)))
                    ))
                )
            } else {
                handlers.to_owned()
            };
            let source = format!(
                "{scope} f(value, values, flag, manager):\n{suspension}{}",
                common::indent(&body)
            );
            common::assert_matches_cpython_marshal(&source, "nested_handler_inner_try_return.py");
        }
    }
}

#[test]
fn matches_cpython_handler_return_through_two_plain_finalizers() {
    for (scope, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield previous\n"),
    ] {
        for binding in ["", " as error"] {
            let source = format!(
                "{scope} f(config, previous):\n{suspension}    try:\n        try:\n            value = config.read()\n            try:\n                return convert(value)\n            except ValueError{binding}:\n                return value\n        finally:\n            config.close()\n    except UsageError as error:\n        report(error)\n        return status.usage\n    finally:\n        finish()\n"
            );
            common::assert_matches_cpython_marshal(&source, "handler_two_finalizer_owners.py");
        }
    }
}
