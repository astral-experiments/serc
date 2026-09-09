use crate::common;

#[test]
fn preserves_terminal_sync_with_conjunction_markers_in_loops() {
    let source = r"import dis


def loop_nested_return(items, manager, guard, first, second):
    for item in items:
        observe(item)
        if guard(item):
            if item:
                with manager:
                    if first(item) and second(item):
                        return item
    return None


def loop_continue(items, manager, first, second):
    for item in items:
        with manager:
            if first(item) and second(item):
                continue
        observe(item)


for function in (loop_nested_return, loop_continue):
    instructions = list(dis.get_instructions(function))
    print([
        (instructions[index - 1].opname, instructions[index + 1].opname)
        for index, instruction in enumerate(instructions)
        if instruction.opname == 'NOP'
    ])
";
    common::assert_matches_cpython_marshal(source, "with_loop_condition_markers.py");
    common::assert_executes(source, "[]\n[('NOT_TAKEN', 'LOAD_CONST')]\n");
}

#[test]
fn matches_cpython_with_cleanup_markers_after_multiple_condition_exits() {
    for prefix in ["", "async "] {
        for loop_header in ["for item in values", "while flag"] {
            for condition in ["value", "0 < value < 2", "first(value) and second(value)"] {
                for body_prefix in ["", "import math\n"] {
                    for exit in ["return value", "continue"] {
                        let body = format!("{body_prefix}if {condition}:\n    {exit}");
                        let source = format!(
                            "{prefix}def f(manager, values, flag, first, second):\n    {loop_header}:\n        try:\n            with manager as value:\n                {}\n        except Error:\n            pass\n        work()\n",
                            body.replace('\n', "\n                "),
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "with_cleanup_multiple_condition_exits.py",
                        );
                    }
                }
            }
        }
    }
}
