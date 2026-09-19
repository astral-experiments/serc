use std::io::Write as _;
use std::process::Stdio;

use crate::common;

const ASSIGNMENT_CLEANUP: &str = r"
import dis, marshal, sys
native = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
actual = marshal.loads(sys.stdin.buffer.read())

def cleanup(root):
    code = next(value for value in root.co_consts if getattr(value, 'co_name', None) == 'f')
    instructions = list(dis.get_instructions(code))
    by_offset = {instruction.offset: instruction for instruction in instructions}
    positions = list(code.co_positions())
    starts = [i + 1 for i, instruction in enumerate(instructions[:-1])
              if instruction.opname == 'STORE_FAST' and instruction.argval == 'result'
              and instructions[i + 1].opname == 'POP_EXCEPT']
    assert len(starts) == 1, starts
    start = starts[0]
    end = next(i for i in range(start, len(instructions))
               if instructions[i].opname.startswith('JUMP_BACKWARD'))
    jump = instructions[end]
    stop = instructions[end + 1].offset
    slots = [(dis.opname[code.co_code[offset]], positions[offset // 2])
             for offset in range(instructions[start].offset, stop, 2)]
    target = by_offset[jump.argval]
    target_identity = (target.opname, target.argrepr, tuple(target.positions))
    owners = []
    for offset in range(instructions[start].offset, stop, 2):
        entry = next((entry for entry in dis.Bytecode(code).exception_entries
                      if entry.start <= offset < entry.end), None)
        if entry is None:
            owners.append(None)
        else:
            owner = by_offset[entry.target]
            owners.append((owner.opname, tuple(owner.positions), entry.depth, entry.lasti))
    return slots, target_identity, owners

expected = cleanup(native)
observed = cleanup(actual)
assert observed == expected, (observed, expected)
";

fn indent(source: &str) -> String {
    format!("    {}", source.replace('\n', "\n    "))
}

fn protect(source: &str, scope: &str) -> String {
    if scope == "with" {
        format!("with manager:\n{}", indent(source))
    } else {
        format!("try:\n{}\nfinally:\n    {scope}", indent(source))
    }
}

fn source(scope: &str, loop_header: &str, binding: &str, inside: bool, test: &str) -> String {
    let handler = format!(
        "try:\n    work()\nexcept Error{binding}:\n    prepare()\n    if {test}:\n        result = value"
    );
    let handler = if inside {
        protect(&handler, scope)
    } else {
        handler
    };
    let body = format!("if mode:\n{}\nelif other:\n    again()", indent(&handler));
    let body = format!("{loop_header}:\n{}", indent(&body));
    let body = if inside { body } else { protect(&body, scope) };
    format!(
        "def f(items, manager, flag, running, mode, other, value, result):\n{}\n",
        indent(&body)
    )
}

#[test]
fn matches_native_handler_assignment_cleanup_before_loop_backedge() {
    for scope in ["with", "cleanup()", "return result", "raise Error"] {
        for loop_header in ["for item in items", "while running"] {
            for binding in ["", " as error"] {
                let source = source(scope, loop_header, binding, false, "flag");
                let filename = "handler_cleanup_before_outer_scope.py";
                let marshal = serc::compile(&source, filename).unwrap().marshal();
                // The false edge still has a separate, preexisting exit-copy mismatch. Compare
                // the retained assignment fallthrough, including every cleanup/cache position,
                // its loop target, and its exception owner against the pinned native occurrence.
                let mut child = common::python_3145_command()
                    .args(["-c", ASSIGNMENT_CLEANUP, &source, filename])
                    .stdin(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&marshal).unwrap();
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "{scope} / {loop_header} / {binding}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

#[test]
fn matches_cpython_handler_cleanup_scope_boundaries() {
    for scope in ["with", "cleanup()", "raise Error"] {
        for loop_header in ["for item in items", "while running"] {
            for binding in ["", " as error"] {
                for (inside, test) in [(true, "flag"), (true, "True"), (false, "True")] {
                    common::assert_matches_cpython_marshal(
                        &source(scope, loop_header, binding, inside, test),
                        "handler_cleanup_scope_boundaries.py",
                    );
                }
            }
        }
    }
}

const EXCEPTIONAL_FINALIZER_SUFFIX: &str = r"
import dis, marshal, sys
native = compile(sys.argv[1], sys.argv[2], 'exec', dont_inherit=True, optimize=0)
actual = marshal.loads(sys.stdin.buffer.read())

def suffix(root):
    code = next(value for value in root.co_consts if getattr(value, 'co_name', None) == 'f')
    instructions = list(dis.get_instructions(code, show_caches=True))
    call = max(i for i, instruction in enumerate(instructions)
               if instruction.opname == 'LOAD_GLOBAL' and instruction.argval == 'step')
    start = max(i for i in range(call) if instructions[i].opname == 'PUSH_EXC_INFO')
    offsets = {instruction.offset: i for i, instruction in enumerate(instructions)}
    entries = list(dis.Bytecode(code).exception_entries)
    result = []
    for instruction in instructions[start:]:
        argument = (offsets[instruction.argval] - start
                    if instruction.opcode in dis.hasjump else instruction.arg)
        owner = next((entry for entry in entries
                      if entry.start <= instruction.offset < entry.end), None)
        owner = None if owner is None else (offsets[owner.target] - start, owner.depth, owner.lasti)
        result.append((instruction.opname, argument, tuple(instruction.positions), owner))
    return result

assert suffix(actual) == suffix(native), (suffix(actual), suffix(native))
";

#[test]
fn matches_native_exceptional_finalizer_cleanup_inside_existing_loop() {
    for scope in ["with", "finish()"] {
        for loop_header in ["for item in items", "while running"] {
            for binding in ["", " as error"] {
                for action in ["work()", "break", "return result"] {
                    let handler = format!(
                        "try:\n    step()\nexcept Error{binding}:\n    prepare()\n    if flag:\n        result = value"
                    );
                    let finalizer = format!("try:\n    {action}\nfinally:\n{}", indent(&handler));
                    let body = format!("{loop_header}:\n{}", indent(&finalizer));
                    let source = format!(
                        "def f(items, manager, flag, running, value, result):\n{}\n",
                        indent(&protect(&body, scope))
                    );
                    let filename = "exceptional_finalizer_existing_loop.py";
                    let marshal = serc::compile(&source, filename).unwrap().marshal();
                    // The normal copied finalizer has an older false-edge copy mismatch.
                    // Compare every native instruction, location and owner from the exceptional
                    // finalizer entry, including each conditional handler's distinct exit copy.
                    let mut child = common::python_3145_command()
                        .args(["-c", EXCEPTIONAL_FINALIZER_SUFFIX, &source, filename])
                        .stdin(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .unwrap();
                    child.stdin.take().unwrap().write_all(&marshal).unwrap();
                    let output = child.wait_with_output().unwrap();
                    assert!(
                        output.status.success(),
                        "{scope} / {loop_header} / {binding} / {action}: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    if loop_header == "while running" && action == "return result" {
                        common::assert_matches_cpython_marshal(&source, filename);
                    }
                }
            }
        }
    }
}
