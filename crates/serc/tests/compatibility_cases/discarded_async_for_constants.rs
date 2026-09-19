use crate::common;

#[test]
fn discarded_async_for_admits_none_between_iterable_and_target() {
    for iterable in [
        "items",
        "factory('iter')",
        "('iter',)",
        "await factory('iter')",
    ] {
        for target in ["value", "sink['target']"] {
            for context in ["if_false", "while_false", "after_return"] {
                for keyword in ["async for", "for"] {
                    let discarded = format!(
                        "        {keyword} {target} in {iterable}:\n            consume('body')\n        else:\n            consume('else')\n        consume('later')\n"
                    );
                    let discarded = match context {
                        "if_false" => format!("    if False:\n{discarded}"),
                        "while_false" => format!("    while False:\n{discarded}"),
                        "after_return" => {
                            format!("    if flag:\n        return value\n{discarded}")
                        }
                        _ => unreachable!(),
                    };
                    let source = format!(
                        "async def f(items, value, flag, sink, factory, consume):\n    consume('before')\n{discarded}    async for value in items:\n        consume(value)\n    consume('iter', 'target', 'body', 'else', 'later')\n"
                    );
                    common::assert_matches_cpython_marshal(
                        &source,
                        "discarded_async_for_constants.py",
                    );
                }
            }
        }
    }
}
