use std::io::Write as _;
use std::process::Stdio;

use serc::{
    InitialInternState, compile, compile_bytes_with_intern_state, compile_with_intern_state,
};

use crate::common;

// Both contexts execute this same bootstrap. Canonical strings arrive through argv,
// so compiling the bootstrap cannot itself intern the tested entries.
const BOOTSTRAP: &str = "import marshal, sys; retained = tuple(map(sys.intern, sys.argv[2:])); code = compile(sys.stdin.buffer.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0); sys.stdout.buffer.write(marshal.dumps(code))";

fn cpython_in_context(source: &str, filename: &str, entries: &[&str]) -> Vec<u8> {
    let mut child = common::python_3145_command()
        .args(["-c", BOOTSTRAP, filename])
        .args(entries)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn matches_cpython_with_explicit_immutable_initial_intern_state() {
    let marker = "bootstrap_token_827";
    let state = InitialInternState::new([marker]);
    let shapes = [
        "f = lambda x: x in {'MARKER', 'secondary_827'}\nx in {'MARKER', 'secondary_827'}\n",
        "def first(x):\n    return x in {'MARKER', 'secondary_827'}\ndef second(x):\n    'different tuple'\n    return x in {'MARKER', 'secondary_827'}\n",
        "prefix = 'MARKER'\nf = lambda x: x in {'MARKER', 'secondary_827'}\nx in {'MARKER', 'secondary_827'}\n",
        "if False:\n    def discarded(x):\n        return x in {'MARKER', 'secondary_827'}\nf = lambda x: x in {'MARKER', 'secondary_827'}\nx in {'MARKER', 'secondary_827'}\n",
        "prefix = ('MARKER',)\ndef child():\n    call(MARKER=1)\nf = lambda x: x in {'MARKER', 'secondary_827'}\nx in {'MARKER', 'secondary_827'}\n",
        "def child(MARKER):\n    return MARKER\nf = lambda x: x in {'MARKER', 'secondary_827'}\nx in {'MARKER', 'secondary_827'}\n",
        "f = lambda x: x in {('MARKER',), ('secondary_827',)}\nx in {('MARKER',), ('secondary_827',)}\n",
        "f = lambda x: x in {'MARKER', 'secondary_827'}\ndef child():\n    return {'MARKER': 1, 'secondary_827': 2}\nx in {'MARKER', 'secondary_827'}\n",
        "import pathlib\nf = lambda x: x in {'MARKER', 'secondary_827'}\nx in {'MARKER', 'secondary_827'}\n",
        "x in {'MARKER', 'secondary_827'}\n",
        "prefix = 'MARKER'\n",
    ];
    let mut context_sensitive = 0;
    for (index, shape) in shapes.iter().enumerate() {
        let source = shape.replace("MARKER", marker);
        let filename = format!("initial_intern_state_{index}.py");
        let isolated = compile(&source, &filename).unwrap();
        let contextual = compile_with_intern_state(&source, &filename, &state).unwrap();
        let isolated_bytes = isolated.marshal();
        let contextual_bytes = contextual.marshal();
        assert_eq!(isolated.bytecode(), contextual.bytecode());
        assert_eq!(
            isolated_bytes,
            cpython_in_context(&source, &filename, &[]),
            "isolated shape {index}"
        );
        assert_eq!(
            contextual_bytes,
            cpython_in_context(&source, &filename, &[marker]),
            "context shape {index}"
        );
        context_sensitive += usize::from(isolated_bytes != contextual_bytes);
        assert_eq!(contextual_bytes, contextual.clone().marshal());
        assert_eq!(
            &contextual.to_timestamp_pyc(3, u32::try_from(source.len()).unwrap())[16..],
            contextual_bytes
        );
        assert_eq!(
            compile_bytes_with_intern_state(source.as_bytes(), &filename, &state)
                .unwrap()
                .marshal(),
            contextual_bytes
        );
        assert_eq!(
            compile_with_intern_state(&source, &filename, &InitialInternState::default())
                .unwrap()
                .marshal(),
            isolated_bytes
        );
    }
    assert!(
        context_sensitive >= 5,
        "wrong profiles must retain observable mismatches"
    );
}

#[test]
fn initial_state_does_not_force_noninternable_or_cached_constants() {
    for marker in ["bootstrap-token-827", "bootstrap_é_827", "z", ""] {
        let source = format!(
            "f = lambda x: x in {{{marker:?}, 'secondary_827'}}\nx in {{{marker:?}, 'secondary_827'}}\n"
        );
        let filename = "initial_intern_state_noninternable.py";
        let contextual = {
            let state = InitialInternState::new([marker]);
            compile_with_intern_state(&source, filename, &state).unwrap()
        };
        let isolated = compile(&source, filename).unwrap().marshal();
        assert_eq!(isolated, contextual.marshal());
        assert_eq!(
            contextual.marshal(),
            cpython_in_context(&source, filename, &[marker])
        );
    }
}
