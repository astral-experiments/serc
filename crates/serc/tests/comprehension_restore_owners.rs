use std::io::Write as _;
use std::process::{Command, Stdio};

use serc::{CompileOptions, PythonVersion, compile_bytes_with_options, compile_with_options};

fn assert_matches_cpython(source: &str) {
    for (version, variable, fallback) in [
        (PythonVersion::Py312, "SERC_TEST_PYTHON_312", "python3.12"),
        (PythonVersion::Py313, "SERC_TEST_PYTHON_313", "python3.13"),
        (PythonVersion::Py314, "SERC_TEST_PYTHON", "python3.14"),
        (PythonVersion::Py315, "SERC_TEST_PYTHON_315", "python3.15"),
    ] {
        let target = version.target();
        let python = std::env::var_os(variable).unwrap_or_else(|| fallback.into());
        let filename = "comprehension_restore_owners.py";
        let oracle = r"
import importlib.util, marshal, sys
assert sys.implementation.name == 'cpython'
assert sys.version.split()[0] == sys.argv[1]
assert importlib.util.MAGIC_NUMBER.hex() == sys.argv[2]
code = compile(sys.stdin.buffer.read(), sys.argv[3], 'exec', dont_inherit=True, optimize=0)
sys.stdout.buffer.write(marshal.dumps(code))
";
        let mut child = Command::new(python)
            .args(["-B", "-I", "-S", "-X", "utf8", "-c", oracle])
            .args([
                target.tag.trim_start_matches('v'),
                &target.magic_hex(),
                filename,
            ])
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
        let expected = child.wait_with_output().unwrap();
        assert!(
            expected.status.success(),
            "{version:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        for actual in [
            compile_with_options(source, filename, &options).unwrap(),
            compile_bytes_with_options(source.as_bytes(), filename, &options).unwrap(),
        ] {
            assert_eq!(actual.marshal(), expected.stdout, "{version:?}:\n{source}");
        }
    }
}

#[test]
fn comprehension_restores_preserve_their_original_owner_array() {
    for prefix in ["", "async "] {
        for comprehension in [
            "[item for item in values]",
            "{item for item in values}",
            "{item: item for item in values}",
        ] {
            for consumer in ["store", "tuple", "attribute", "call"] {
                let assignment = match consumer {
                    "store" => format!("result = {comprehension}"),
                    "tuple" => format!("result = ({comprehension}, {{}})"),
                    "attribute" => format!("box.result = {comprehension}"),
                    _ => format!("consume({comprehension})"),
                };
                for condition in ["flag", "flag.state.ready", "flag is not None"] {
                    assert_matches_cpython(&format!(
                        "{prefix}def f(values, flag, box):\n    {assignment}\n    try:\n        if {condition}:\n            work(flag)\n    except Error:\n        recover(flag)\n    return flag\n"
                    ));
                }
            }
        }
    }
}

fn indent(source: &str) -> String {
    let mut result = String::new();
    for line in source.lines() {
        result.push_str("    ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[test]
fn named_scopes_preserve_normal_restore_owner_arrays() {
    for class_scope in [false, true] {
        for comprehension in [
            "[item for item in values]",
            "{item for item in values}",
            "{item: item for item in values}",
        ] {
            for consumer in ["tuple", "map", "store"] {
                let assignment = match consumer {
                    "tuple" => format!("result = ({comprehension}, {{}})"),
                    "map" => format!("result = {{'items': {comprehension}, 'state': 'ready'}}"),
                    _ => format!("result = {comprehension}"),
                };
                for context in ["plain", "try", "named-handler"] {
                    let mut body = format!(
                        "{assignment}\ntry:\n    if flag:\n        work(flag)\nexcept Error:\n    recover(flag)\ntail(flag)\n"
                    );
                    body = match context {
                        "try" => format!(
                            "try:\n{}except OuterError:\n    recover(flag)\n",
                            indent(&body)
                        ),
                        "named-handler" => format!(
                            "try:\n    work(flag)\nexcept OuterError as error:\n{}",
                            indent(&body)
                        ),
                        _ => body,
                    };
                    if class_scope {
                        body = format!("class C:\n{}", indent(&body));
                    }
                    assert_matches_cpython(&body);
                }
            }
        }
    }
}

#[test]
fn original_try_setups_complete_four_slot_restore_arrays() {
    for scope in ["sync", "coroutine", "generator", "async-generator"] {
        for comprehension in [
            "[item for item in values]",
            "{item for item in values}",
            "{item: item for item in values}",
        ] {
            for consumer in ["map", "binary", "call", "method"] {
                let result = match consumer {
                    "map" => format!("{{'items': {comprehension}}}"),
                    "binary" => format!("seed | {comprehension}"),
                    "call" => format!("collect({comprehension})"),
                    _ => format!("box.collect({comprehension})"),
                };
                for outer_try in [false, true] {
                    let mut body = format!(
                        "result = {result}\ntry:\n    if flag:\n        work(flag)\nexcept Error:\n    recover(flag)\n"
                    );
                    if outer_try {
                        body = format!(
                            "try:\n{}except OuterError:\n    recover(values)\n",
                            indent(&body)
                        );
                    }
                    body.push_str(if scope.ends_with("generator") {
                        "yield flag\n"
                    } else {
                        "return flag\n"
                    });
                    let prefix = if matches!(scope, "coroutine" | "async-generator") {
                        "async "
                    } else {
                        ""
                    };
                    assert_matches_cpython(&format!(
                        "{prefix}def f(values, flag, box, seed):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}

#[test]
fn discarded_and_suspending_restores_keep_their_existing_boundaries() {
    for prefix in ["", "async "] {
        for terminal in ["return flag", "yield flag"] {
            assert_matches_cpython(&format!(
                "{prefix}def f(values, flag):\n    [(a, b) for a, b in values]\n    try:\n        if flag:\n            work(flag)\n    except Error:\n        recover(flag)\n    {terminal}\n"
            ));
        }
    }
    for assignment in ["", "result = "] {
        for terminal in ["return flag", "yield flag"] {
            assert_matches_cpython(&format!(
                "async def f(values, flag):\n    {assignment}[a async for a in values]\n    try:\n        if flag:\n            work(flag)\n    except Error:\n        recover(flag)\n    {terminal}\n"
            ));
        }
    }
}

#[test]
fn positive_depth_try_setups_preserve_the_original_restore_array() {
    for scope in ["sync", "coroutine", "generator", "async-generator"] {
        let asynchronous = matches!(scope, "coroutine" | "async-generator");
        for comprehension in [
            "[item for item in values]",
            "{item for item in values}",
            "{item: item for item in values}",
        ] {
            for result in [
                format!("{{'items': {comprehension}}}"),
                format!("seed | {comprehension}"),
            ] {
                for ambient in [
                    "for",
                    "two-for",
                    "for-with",
                    "two-with",
                    "handler-with",
                    "async-with",
                ] {
                    if ambient == "async-with" && !asynchronous {
                        continue;
                    }
                    let body = format!(
                        "result = {result}\ntry:\n    if flag:\n        work(flag)\nexcept Error:\n    recover(flag)\n"
                    );
                    let mut body = match ambient {
                        "for" => format!("for token in tokens:\n{}", indent(&body)),
                        "two-for" => format!(
                            "for token in tokens:\n    for token2 in tokens2:\n{}",
                            indent(&indent(&body))
                        ),
                        "for-with" => format!(
                            "for token in tokens:\n    with manager():\n{}",
                            indent(&indent(&body))
                        ),
                        "two-with" => format!(
                            "with manager():\n    with manager2():\n{}",
                            indent(&indent(&body))
                        ),
                        "handler-with" => format!(
                            "try:\n    prepare()\nexcept OuterError as error:\n    with manager():\n{}",
                            indent(&indent(&body))
                        ),
                        _ => format!("async with manager():\n{}", indent(&body)),
                    };
                    body.push_str(if scope.ends_with("generator") {
                        "yield flag\n"
                    } else {
                        "return flag\n"
                    });
                    let prefix = if asynchronous { "async " } else { "" };
                    assert_matches_cpython(&format!(
                        "{prefix}def f(values, flag, seed, tokens, tokens2):\n{}",
                        indent(&body)
                    ));
                }
            }
        }
    }
}
