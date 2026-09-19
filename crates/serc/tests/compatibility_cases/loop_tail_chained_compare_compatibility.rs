use crate::common;

#[test]
fn matches_cpython_marshal_for_loop_tail_chained_compare_returns() {
    let source = r"def direct(items, low, high):
    for item in items:
        if low <= item < high:
            return item
    return None

def after_assignment(items, low, high):
    for item in items:
        value = item.value
        if low <= value < high:
            return value
    return None

def after_continue(items, low, high):
    for item in items:
        if item is None:
            continue
        if low <= item < high:
            return item
    return None

def nested_for(items, low, high):
    while items:
        for item in items:
            if low <= item < high:
                return True
        items = items[1:]
    return False

def three_comparators(items, low, mid, high):
    for item in items:
        if low <= item < mid < high:
            return item
    return None

def nested_if(items, low, high, ready):
    for item in items:
        if ready:
            if low <= item < high:
                return item
    return None

def raises(items, low, high):
    for item in items:
        if low <= item < high:
            raise ValueError(item)
    return None

def nonterminal(items, low, high, out):
    for item in items:
        if low <= item < high:
            out.append(item)
    return None

def multi_statement_terminal(items, low, high, out):
    for item in items:
        if low <= item < high:
            out.append(item)
            return item
    return None
";
    common::assert_matches_cpython_marshal(source, "loop_tail_chained_compare_returns.py");
}

#[test]
fn matches_cpython_marshal_for_final_elif_chained_compare() {
    common::assert_matches_cpython_marshal(
        r"def probe(items, out):
    for value in items:
        if 10 <= value < 20:
            out[1] += 1
    return out
",
        "loop_bare_chained_compare_control.py",
    );
    common::assert_matches_cpython_marshal(
        r"def probe(items, out):
    for value in items:
        if 0 <= value < 10:
            out[0] += 1
        elif 10 <= value < 20:
            out[1] += 1
    return out
",
        "loop_final_elif_chained_compare.py",
    );
    common::assert_matches_cpython_marshal(
        r"for value in items:
    if 0 <= value < 10:
        out[0] += 1
    elif 10 <= value < 20:
        out[1] += 1
finish(out)
",
        "module_final_elif_chained_compare.py",
    );
}

#[test]
fn matches_cpython_marshal_for_final_elif_or_chains() {
    for (filename, source) in [
        (
            "final_elif_or_two_chains_loop_target.py",
            r#"def count(items, counts):
    for value in items:
        if value is None:
            counts["missing"] += 1
        elif 0 <= value < 10 or 20 <= value < 30:
            counts["range"] += 1
    return counts
"#,
        ),
        (
            "final_elif_or_attribute_chain_loop_target.py",
            r"def collect(items, bounds, output):
    count = 0
    for value in items:
        if value == bounds.stop:
            count += 1
        elif count == 0:
            if value == bounds.special or bounds.low <= value <= bounds.high:
                output.append(value)
            elif value < bounds.limit:
                output.append(value)
        elif value == bounds.special or bounds.low <= value <= bounds.high:
            output.append(value)
    return count, output
",
        ),
        (
            "final_elif_or_negated_chain_loop_target.py",
            r"def validate(items, upper, warnings):
    for field, value in items:
        if value is None:
            if field not in warnings:
                warnings.append(field)
        elif not finite(value) or not 0.0 <= value <= upper:
            warnings.append(field)
    return warnings
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_marshal_for_final_elif_and_chains() {
    for (filename, source) in [
        (
            "final_elif_and_chain_loop_control.py",
            r#"def count(items, counts):
    for value in items:
        if value is None:
            counts["missing"] += 1
        elif value != 5 and 0 <= value < 10:
            counts["range"] += 1
    return counts
"#,
        ),
        (
            "final_elif_and_calls_chain_loop_target.py",
            r"def restore(items):
    for parent, key, value in items:
        if mapping(parent):
            parent[key] = value
        elif (
            sequence(parent)
            and integer(key)
            and 0 <= key < len(parent)
        ):
            parent[key] = value
",
        ),
        (
            "final_elif_and_equal_chain_loop_target.py",
            r"def collect(items, left, middle, right, output):
    for value in items:
        if value == '(':
            left += 1
        elif value == ')':
            left -= 1
        elif value == ',' and left == middle == right == 0:
            output.append(value)
    return output
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_final_elif_chain_original_cleanup_donors() {
    for (filename, source) in [
        (
            "sync_final_elif_negated_chain.py",
            r"def run(items, upper, output):
    for value in items:
        if value is None:
            output.append(value)
        elif not 0 <= value <= upper:
            output.append(value)
    return output
",
        ),
        (
            "async_final_elif_negated_chain.py",
            r"async def run(fetch, upper, output):
    items = await fetch()
    if items:
        for value in items:
            if value is None:
                output.append(value)
            elif not 0 <= value <= upper:
                output.append(value)
    return output
",
        ),
        (
            "async_try_final_elif_direct_chain.py",
            r"async def run(fetch, previous, current, output):
    try:
        items = await fetch()
        for limit in items:
            if previous < limit <= current:
                output.append(limit)
            elif current < limit <= previous:
                output.append(limit)
    except Exception:
        output.clear()
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_nonfinal_elif_chained_compare_exit() {
    for (filename, source) in [
        (
            "nonfinal_nested_elif_chained_compare_target.py",
            r"def compare_levels(rows):
    changed = False
    for current, middle, target in rows:
        if target < current:
            changed = True
        elif target > current:
            if middle is not None:
                if middle < current:
                    changed = True
                elif current < middle <= target:
                    changed = True
        else:
            if middle is not None and middle < current:
                changed = True
    return changed
",
        ),
        (
            "nonfinal_nested_elif_simple_compare_control.py",
            r"def compare_levels(rows):
    changed = False
    for current, middle, target in rows:
        if target < current:
            changed = True
        elif target > current:
            if middle is not None:
                if middle < current:
                    changed = True
                elif middle <= target:
                    changed = True
        else:
            if middle is not None and middle < current:
                changed = True
    return changed
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}

#[test]
fn matches_cpython_loop_tail_comparison_pass_original_join() {
    for (filename, source) in [
        (
            "loop_tail_chained_comparison_pass_join_target.py",
            r"def collect(items, limit, buckets):
    for index, value in enumerate(items):
        if 0 <= value < limit:
            buckets[value].append(index)
        else:
            pass
    finish(buckets)
",
        ),
        (
            "loop_tail_single_comparison_pass_join_control.py",
            r"def collect(items, limit, buckets):
    for index, value in enumerate(items):
        if value < limit:
            buckets[value].append(index)
        else:
            pass
    finish(buckets)
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
