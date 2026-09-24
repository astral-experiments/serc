use crate::common;

#[test]
fn matches_cpython_non_final_branch_while_break_locations() {
    for outer in ["for group in groups:", "while ready():"] {
        for condition in ["index < limit", "check(index)", "flag"] {
            for update in ["index += 1", "index = update(index)"] {
                let source = format!(
                    "def f(groups, flag, limit):\n    {outer}\n        if flag:\n            index = 0\n            while {condition}:\n                consume(index)\n                if done(index):\n                    break\n                {update}\n        else:\n            consume(flag)\n    return\n"
                );
                common::assert_matches_cpython_marshal(&source, "branch_while_break_locations.py");
            }
        }
    }
}

#[test]
fn matches_cpython_nested_tail_while_breaks() {
    for prefix in [
        "",
        "        consume(group)\n",
        "        if skip(group):\n            continue\n",
    ] {
        for condition in ["index < len(group)", "check(index)", "flag"] {
            for update in ["index += 1", "index = update(index)"] {
                for after in ["", "        consume(index)\n"] {
                    let source = format!(
                        "def f(groups, flag):\n    result = []\n    for group in groups:\n{prefix}        index = 0\n        while {condition}:\n            consume(index)\n            if done(index):\n                break\n            {update}\n{after}    return result\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "nested_tail_while_breaks.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_traced_break_into_an_outer_iterator_backedge() {
    for (definition, footer, wrapper, close) in [
        ("def", "    yield items\n", "", ""),
        ("async def", "    await finish()\n", "", ""),
        (
            "def",
            "",
            "    try:\n",
            "    except ValueError:\n        recover()\n",
        ),
        ("def", "", "    with manager():\n", ""),
        ("def", "", "", ""),
    ] {
        for tail in ["", "else:\n    finish()\n"] {
            let body = format!(
                "for value in items:\n    while ready():\n        break\n    else:\n        clean()\n{tail}",
            );
            let source = format!(
                "{definition} f(items):\n{wrapper}{}{close}{footer}",
                if wrapper.is_empty() {
                    common::indent(&body)
                } else {
                    common::indent(&common::indent(&body))
                },
            );
            common::assert_matches_cpython_marshal(&source, "traced_nested_while_break.py");
        }
    }

    common::assert_executes(
        r"import sys

def f(items):
    for value in items:
        while more:
            break
        else:
            clean()
    yield items

more = True
lines = []
def trace(frame, event, arg):
    if frame.f_code.co_name == 'f' and event == 'line':
        lines.append(frame.f_lineno)
    return trace

def clean():
    raise AssertionError('unexecuted loop else')

sys.settrace(trace)
try:
    list(f([1, 2]))
finally:
    sys.settrace(None)
print(lines)",
        "[4, 5, 6, 4, 5, 6, 4, 9]\n",
    );
}
