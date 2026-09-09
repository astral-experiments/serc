use crate::common;

#[test]
fn matches_cpython_borrows_after_surviving_while_breaks() {
    for scope in 0..4 {
        let is_async = scope == 1 || scope == 3;
        let prefix = if is_async { "async " } else { "" };
        let yielded = if scope >= 2 { "    yield result\n" } else { "" };
        let tail = if scope >= 2 {
            "consume(value, result)\n    yield result"
        } else {
            "return consume(value, result)"
        };
        for body in [
            "value = next_value()\nif flag:\n    break\nif other:\n    break",
            "value = next_value()\nif flag:\n    break\nif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nconsume(value)\nif done:\n    break",
            "value = next_value()\nif flag:\n    break\nif other:\n    continue\nbreak",
            "value = next_value()\nif flag:\n    if other:\n        break\n    continue\nconsume(value)\nif done:\n    break",
            "value = next_value()\nif flag:\n    break\nelif other:\n    continue\nif done:\n    break",
            "value = next_value()\nif other:\n    continue\nif flag:\n    break\nconsume(value)",
        ] {
            let source = format!(
                "{prefix}def function(flag, other, done, result, next_value):\n{yielded}    while True:\n        {}\n    {tail}\n",
                body.replace('\n', "\n        "),
            );
            common::assert_matches_cpython_marshal(&source, "surviving_while_break.py");
        }
    }
}
