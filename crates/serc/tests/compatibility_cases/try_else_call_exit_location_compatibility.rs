use crate::common;

#[test]
fn matches_cpython_marshal_for_try_else_call_exit_locations() {
    let source = r"def direct(flag):
    try:
        action()
    except Error:
        pass
    else:
        if check(flag):
            return result
    consume()


def raised(flag):
    try:
        action()
    except Error:
        pass
    else:
        if check(flag):
            raise Problem
    consume()


def final_elif(flag):
    try:
        action()
    except Error:
        pass
    else:
        if first_check(flag):
            return first_result
        elif second_check(flag):
            return second_result
    consume()


def multiline_method(state, flag):
    try:
        action()
    except FirstError:
        pass
    except SecondError:
        pass
    else:
        if state.ready(
            flag,
        ):
            return result
    consume()
";
    common::assert_matches_cpython_marshal(source, "try_else_call_exit_locations.py");
}

#[test]
fn matches_cpython_linear_try_else_break_locations() {
    for (filename, source) in [
        (
            "generator_try_else_linear_break_location.py",
            r"def run(client, attempts, release):
    try:
        for attempt in attempts:
            try:
                ready = client.ping()
            except Exception:
                report(attempt)
            else:
                if ready:
                    report(ready)
                    break
            pause(attempt)
        yield client
    finally:
        release()
",
        ),
        (
            "nested_loops_try_else_linear_break_location.py",
            r"def run(groups, roots, copy, exists):
    restored = []
    for group in groups:
        for root in roots:
            try:
                copy(root, group)
            except Exception as error:
                message = str(error)
            else:
                if exists(group):
                    ready = True
                    restored.append(group)
                    report(root, group)
                    break
            retry(root, group)
    return restored
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
