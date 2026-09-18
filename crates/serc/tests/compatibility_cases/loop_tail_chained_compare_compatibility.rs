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
