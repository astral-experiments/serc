use crate::common;

#[test]
fn matches_cpython_generator_protected_loop_exits() {
    for (function, iterator) in [("def", "for"), ("async def", "async for")] {
        for handler in ["except:", "except Error:", "except Error as error:"] {
            for action in ["continue", "pass"] {
                for condition in ["flag", "flag and check()"] {
                    let source = format!(
                        "{function} f(items, flag):\n    {iterator} item in items:\n        try:\n            if {condition}:\n                yield item\n        {handler}\n            action()\n            {action}\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "generator_protected_loop_exits.py",
                    );
                }
            }
        }
    }
}

#[test]
fn matches_cpython_generator_protected_exits_after_nested_iteration() {
    for (function, iterator) in [("def", "for"), ("async def", "async for")] {
        for following in ["", "    finish()\n"] {
            let source = format!(
                "{function} f(items, flag):\n    {iterator} item in items:\n        try:\n            for value in item:\n                yield value\n            if flag:\n                yield item\n        except:\n            action()\n            continue\n{following}"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "generator_protected_exits_after_iteration.py",
            );
        }
    }
}
