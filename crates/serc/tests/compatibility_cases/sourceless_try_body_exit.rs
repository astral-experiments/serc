use crate::common;

#[test]
fn matches_cpython_forwarded_try_body_condition_exit() {
    for (declaration, prefix) in [
        ("def", ""),
        ("async def", "    await tick()\n"),
        ("def", "    yield idx\n"),
    ] {
        for condition in ["bad", "bad and good", "bad or good", "not bad", "True"] {
            for terminal in ["continue", "break", "return idx", "raise Error"] {
                let source = format!(
                    r"{declaration} f(items, idx, bad, good):
{prefix}    for item in items:
        try:
            if idx == 0:
                idx = 1
            else:
                if {condition}:
                    if good:
                        idx += 1
                    else:
                        {terminal}
        except Exception:
            pass
        consume(idx)
"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "forwarded_try_body_condition_exit.py",
                );
            }
        }
    }
}
