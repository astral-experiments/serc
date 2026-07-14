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
