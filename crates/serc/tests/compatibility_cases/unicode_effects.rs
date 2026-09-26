use std::io::Write as _;
use std::process::Stdio;

use serc::{
    CompilationEffect, CompilationEvent, CompileError, CompileOptions, PreparedCompilation,
};

use crate::common;

// Native module initialization retains these canonical keys. This is a positive subset;
// neither this list nor the native comparisons establish absence for any other string.
const UNICODE_RETAINED_KEYS: [&str; 17] = [
    "decimal",
    "digit",
    "numeric",
    "category",
    "bidirectional",
    "combining",
    "mirrored",
    "east_asian_width",
    "decomposition",
    "name",
    "lookup",
    "is_normalized",
    "normalize",
    "unidata_version",
    "UCD",
    "ucd_3_2_0",
    "_ucnhash_CAPI",
];

const FILENAME: &str = "unicode_effects.py";

// Keep the normalizer, name capsule, and import machinery unchanged. Each source enters
// a fresh interpreter through stdin, so compiling the bootstrap cannot intern its names.
const BOOTSTRAP: &str = r"
import marshal, sys
assert 'unicodedata' not in sys.modules
code = compile(sys.stdin.buffer.read(), sys.argv[1], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
sys.stderr.write('1' if 'unicodedata' in sys.modules else '0')
";

fn native_marshal(source: &str, imports_unicode: bool) -> Vec<u8> {
    let mut child = common::python_3145_command()
        .arg("-B")
        .args(["-c", BOOTSTRAP, FILENAME])
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
        "{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        output.stderr.as_slice(),
        if imports_unicode { b"1" } else { b"0" },
        "unexpected native Unicode import: {source}"
    );
    output.stdout
}

fn compare_native(source: &str, unicode: Option<(bool, bool)>) {
    compare_native_with_unchanged(source, unicode, &[]);
}

fn compare_native_with_unchanged(source: &str, unicode: Option<(bool, bool)>, unchanged: &[&str]) {
    let prepared =
        PreparedCompilation::bytes(source.as_bytes(), FILENAME, &CompileOptions::default())
            .unwrap();
    assert!(prepared.warnings().is_empty(), "{source}");
    match (unicode, prepared.events()) {
        (Some((identifiers, named_escapes)), [CompilationEvent::UnicodeParsing(event)]) => {
            assert_eq!(event.normalizes_identifiers(), identifiers, "{source}");
            assert_eq!(event.decodes_named_escapes(), named_escapes, "{source}");
        }
        (None, []) => {}
        _ => panic!(
            "unexpected compilation events: {:?}\n{source}",
            prepared.events()
        ),
    }
    let mut effects = prepared.effects();
    if unicode.is_some() {
        effects
            .record_event(
                0,
                CompilationEffect::PartialRetainedStringsWithUnchanged {
                    retained_strings: UNICODE_RETAINED_KEYS
                        .iter()
                        .map(|key| (*key).to_string())
                        .collect(),
                    unchanged_strings: unchanged.iter().map(|value| (*value).to_string()).collect(),
                },
            )
            .unwrap();
    }
    assert_eq!(
        prepared.finish(&effects).unwrap().marshal(),
        native_marshal(source, unicode.is_some()),
        "{source}"
    );
}

#[test]
fn matches_cpython_marshal_for_noninterned_constant_metadata_collisions() {
    let source = r#"from package.config import exported, café
from other import *

if error.name == "package.config":
    recover()

class Outer:
    class Inner:
        pass
"#;
    // The pristine native import/normalizer cannot create this dotted spelling.
    // Preserve its incoming absence, while parser-owned import prefixes stay canonical.
    compare_native_with_unchanged(source, Some((true, false)), &["Outer.Inner"]);
}

fn consumers() -> &'static str {
    "def first(value):\n    return value in {'normalize', 'is_normalized'}\n\
     def second(value):\n    'Distinct constants tuple.'\n    return value in {'normalize', 'is_normalized'}\n"
}

#[test]
fn unicode_identifier_import_preserves_mixed_frozenset_owners() {
    let source = "слово = 1\n\
        def first(value):\n    return value in {'normalize', 'novel_746_marker'}, 1\n\
        def second(value):\n    'second docstring'\n    return value in {'normalize', 'novel_746_marker'}, 2\n";
    compare_native(source, Some((true, false)));
}

#[test]
fn unicode_parser_imports_preserve_native_frozenset_identity() {
    for (trigger, identifiers, named_escapes) in [
        ("я = 0\n", true, false),
        ("\u{212a} = 0\n", true, false),
        ("label = '\\N{LATIN CAPITAL LETTER A}'\n", false, true),
        ("\u{212a} = '\\N{LATIN CAPITAL LETTER A}'\n", true, true),
    ] {
        // Even a trigger after both function definitions runs before either code object
        // is assembled. It must not be delayed until the trigger's source position.
        for source in [
            format!("{trigger}{}", consumers()),
            format!("{}{trigger}", consumers()),
        ] {
            compare_native(&source, Some((identifiers, named_escapes)));
        }
    }
}

#[test]
fn unicode_parser_imports_preserve_parser_owned_identifier_origins() {
    for owner in [
        "normalize = 0\n",
        "def seed(normalize):\n    return normalize\n",
        "def normalize():\n    pass\n",
        "class normalize:\n    pass\n",
    ] {
        for (trigger, flags) in [
            ("\u{212a} = 0\n", (true, false)),
            ("label = '\\N{LATIN CAPITAL LETTER A}'\n", (false, true)),
        ] {
            for source in [
                format!("prior = 'normalize'\n{owner}{trigger}{}", consumers()),
                format!("prior = 'normalize'\n{trigger}{owner}{}", consumers()),
            ] {
                compare_native(&source, Some(flags));
            }
        }
    }
}

#[test]
fn unicode_text_and_inactive_named_escapes_do_not_import() {
    for control in [
        "label = 'я\u{212a}'\n",
        "# я \u{212a} \\N{LATIN CAPITAL LETTER A}\n",
        "label = r'\\N{LATIN CAPITAL LETTER A}'\n",
        "label = '\\\\N{LATIN CAPITAL LETTER A}'\n",
        "label = br'\\N{LATIN CAPITAL LETTER A}'\n",
        "label = b'\\\\N{LATIN CAPITAL LETTER A}'\n",
    ] {
        compare_native(&format!("{control}{}", consumers()), None);
    }
}

#[test]
fn partial_unicode_effects_leave_unproved_membership_unresolved() {
    let source = format!("\u{212a} = 0\n{}", consumers());
    let prepared =
        PreparedCompilation::bytes(source.as_bytes(), FILENAME, &CompileOptions::default())
            .unwrap();
    let mut effects = prepared.effects();
    effects
        .record_event(0, CompilationEffect::PartialRetainedStrings(Vec::new()))
        .unwrap();
    assert!(matches!(
        prepared.finish(&effects),
        Err(CompileError::UnresolvedInternState { value })
            if value == "normalize" || value == "is_normalized"
    ));
}
