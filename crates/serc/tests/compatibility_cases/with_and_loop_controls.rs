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
    common::assert_executes(
        source,
        "[('RETURN_VALUE', 'LOAD_CONST')]\n[('NOT_TAKEN', 'LOAD_CONST'), ('JUMP_BACKWARD', 'LOAD_CONST')]\n",
    );
}
