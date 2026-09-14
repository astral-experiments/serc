use crate::common;

#[test]
fn copies_small_else_returns_after_chained_comparison_cleanup() {
    let source = r"def case_0(item, value, bound, values, flag):
    if (
        0 <
        item < value
    ):
        return item
    else:
        return value

def case_10(item, value, bound, values, flag):
    if 0 < item < value < bound:
        return item
    else:
        return value

def case_20(item, value, bound, values, flag):
    if 0 < item is None:
        return item
    else:
        return value

def case_30(item, value, bound, values, flag):
    if (
        0 <
        item in [0, 1]
    ):
        return item
    else:
        return value

def case_40(item, value, bound, values, flag):
    if 0 < item in (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,):
        return item
    else:
        return value

def case_50(item, value, bound, values, flag):
    if 0 < item in (*values,):
        return item
    else:
        return value
";
    common::assert_matches_cpython_marshal(
        source,
        "copies_small_else_returns_after_chained_comparison_cleanup.py",
    );
}

#[test]
fn preserves_copied_else_return_owners_in_protected_suspended_functions() {
    let source = r"def case_60(item, value, bound, values, flag):
    try:
        if (
            0 <
            item < value
        ):
            return item
        else:
            return value
    except Error:
        recover()

async def case_300(item, value, bound, values, flag):
    await ready()
    try:
        if (
            0 <
            item < value
        ):
            return item
        else:
            return value
    except Error:
        recover()

def case_540(item, value, bound, values, flag):
    yield value
    try:
        if (
            0 <
            item < value
        ):
            return item
        else:
            return value
    except Error:
        recover()

async def case_780(item, value, bound, values, flag):
    await ready()
    yield value
    try:
        if (
            0 <
            item < value
        ):
            yield item
            return
        else:
            yield value
            return
    except Error:
        recover()
";
    common::assert_matches_cpython_marshal(
        source,
        "preserves_copied_else_return_owners_in_protected_suspended_functions.py",
    );
}

#[test]
fn copies_bare_else_returns_in_async_generators() {
    let source = r"async def function(item, value, values):
    await ready()
    yield value
    if 0 < item in [0, 1]:
        return
    else:
        return
";
    common::assert_matches_cpython_marshal(source, "async_generator_else_return.py");
}

#[test]
fn keeps_large_chained_return_blocks_after_comparison_cleanup() {
    for returned in [
        "holder.values[index]",
        "values[index].replace(' ', '_')",
        "work(index)",
    ] {
        let source = format!(
            "def f(holder, index, bound, values):\n    if 0 <= index < bound:\n        return {returned}\n    else:\n        raise IndexError('x')\n"
        );
        common::assert_matches_cpython_marshal(&source, "large_chained_else_return.py");
    }
}

#[test]
fn copies_four_instruction_chained_return_blocks() {
    let source = r"def f(index, bound, values):
    if 0 <= index < bound:
        return values[index]
    else:
        raise IndexError('x')
";
    common::assert_matches_cpython_marshal(source, "four_instruction_chained_else_return.py");
}

#[test]
fn copies_constant_returns_after_generic_chained_cleanup() {
    for (filename, source) in [
        (
            "chained_cleanup_constant_footer.py",
            r"def f(raw):
    try:
        value = int(raw)
        if 0 <= value <= 4:
            return value
        return None
    except Exception:
        return None
",
        ),
        (
            "boolean_chained_cleanup_constant_footer.py",
            r"def f(ready, value, limit):
    try:
        if ready and 0 < value < limit:
            return True
        return False
    except Exception:
        return False
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
