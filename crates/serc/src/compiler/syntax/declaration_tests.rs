use crate::{CompileError, CompileOptions, PythonVersion, compile, compile_with_options};

const TARGETS: [PythonVersion; 4] = [
    PythonVersion::Py312,
    PythonVersion::Py313,
    PythonVersion::Py314,
    PythonVersion::Py315,
];

fn error_at(source: &str, version: PythonVersion, message: &str, diagnostic: &str) {
    let options = CompileOptions {
        python_version: version,
        ..CompileOptions::default()
    };
    let error = compile_with_options(source, "declarations.py", &options).unwrap_err();
    let CompileError::InvalidSyntax {
        message: actual,
        range,
    } = error
    else {
        panic!("{version:?}: {source}: {error}");
    };
    assert_eq!(actual, message, "{version:?}: {source}");
    let start = source.find(diagnostic).unwrap();
    assert_eq!(
        (usize::from(range.start()), usize::from(range.end())),
        (start, start + diagnostic.len()),
        "{version:?}: {source}",
    );
}

fn all_versions_error(source: &str, message: &str, diagnostic: &str) {
    for version in TARGETS {
        error_at(source, version, message, diagnostic);
    }
}

#[test]
fn declarations_precede_pattern_lowering_errors() {
    for pattern in [
        "{1: _, 1: _}",
        "Point(x=1, x=2)",
        "[x, x]",
        "[(1 as x) | (2 as y)]",
        "[*x, *y]",
        "[x | 1]",
    ] {
        let matching = format!("match subject:\n case {pattern}: pass\n");
        let declaration = "def f():\n nonlocal missing\n";
        for source in [
            format!("{matching}{declaration}"),
            format!("{declaration}{matching}"),
        ] {
            all_versions_error(
                &source,
                "no binding for nonlocal 'missing' found",
                "nonlocal missing",
            );
        }
    }

    for source in [
        "match subject:\n case {1: _, 1: _}: pass\ndef f():\n value = 1\n global value\n",
        "def f():\n value = 1\n global value\nmatch subject:\n case {1: _, 1: _}: pass\n",
    ] {
        all_versions_error(
            source,
            "name 'value' is assigned to before global declaration",
            "global value",
        );
    }

    let bound = "def outer():\n value = 1\n def inner():\n  nonlocal value\n  match subject:\n   case {1: _, 1: _}: pass\n";
    for version in TARGETS {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        assert!(matches!(
            compile_with_options(bound, "declarations.py", &options),
            Err(CompileError::InvalidPattern { .. })
        ));
    }
}

#[test]
fn declaration_order_rejects_uses_bindings_parameters_and_annotations() {
    for (source, message, diagnostic) in [
        (
            "value\nglobal value, spare",
            "name 'value' is used prior to global declaration",
            "global value, spare",
        ),
        (
            "def f():\n value = 1\n print(value)\n global value",
            "name 'value' is used prior to global declaration",
            "global value",
        ),
        (
            "def f():\n del value\n global value, spare",
            "name 'value' is assigned to before global declaration",
            "global value, spare",
        ),
        (
            "def f(value):\n nonlocal value",
            "name 'value' is parameter and nonlocal",
            "nonlocal value",
        ),
        (
            "def f():\n def value(): pass\n global spare, value",
            "name 'value' is assigned to before global declaration",
            "global spare, value",
        ),
        (
            "def f():\n type value = int\n global value",
            "name 'value' is assigned to before global declaration",
            "global value",
        ),
        (
            "def f():\n match 1:\n  case value: pass\n global value",
            "name 'value' is assigned to before global declaration",
            "global value",
        ),
        (
            "def f():\n try: pass\n except Exception as value: pass\n global value",
            "name 'value' is assigned to before global declaration",
            "global value",
        ),
        (
            "def f():\n with manager as value: pass\n global value",
            "name 'value' is assigned to before global declaration",
            "global value",
        ),
        (
            "def f():\n global value\n value: int",
            "annotated name 'value' can't be global",
            "value: int",
        ),
        (
            "value: int\nglobal value",
            "annotated name 'value' can't be global",
            "global value",
        ),
        (
            "def outer():\n value = 1\n def f():\n  value: int\n  nonlocal value",
            "annotated name 'value' can't be nonlocal",
            "nonlocal value",
        ),
        (
            "def outer():\n value = 1\n def f():\n  nonlocal value\n  value: int",
            "annotated name 'value' can't be nonlocal",
            "value: int",
        ),
        (
            "value = 1\nnonlocal value",
            "name 'value' is assigned to before nonlocal declaration",
            "nonlocal value",
        ),
        (
            "def f():\n super\n global __class__",
            "name '__class__' is used prior to global declaration",
            "global __class__",
        ),
        (
            "class C:\n def f():\n  super()\n  nonlocal __class__",
            "name '__class__' is used prior to nonlocal declaration",
            "nonlocal __class__",
        ),
    ] {
        all_versions_error(source, message, diagnostic);
    }
}

#[test]
fn declarations_retain_activity_after_prior_declarations_and_first_conflict_ranges() {
    for (source, message, diagnostic) in [
        (
            "def f():\n global value\n value = 1\n global value, spare",
            "name 'value' is assigned to before global declaration",
            "global value, spare",
        ),
        (
            "def outer():\n value = 1\n def f():\n  nonlocal value\n  value\n  nonlocal value, spare",
            "name 'value' is used prior to nonlocal declaration",
            "nonlocal value, spare",
        ),
        (
            "def f():\n global value\n nonlocal value",
            "name 'value' is nonlocal and global",
            "global value",
        ),
        (
            "def f():\n nonlocal value\n global value",
            "name 'value' is nonlocal and global",
            "nonlocal value",
        ),
        (
            "def f():\n import os as value\n value\n global value",
            "name 'value' is used prior to global declaration",
            "global value",
        ),
    ] {
        all_versions_error(source, message, diagnostic);
    }
}

#[test]
fn declaration_names_follow_private_mangling_but_use_the_declared_spelling() {
    for (source, message, diagnostic) in [
        (
            "class C:\n def f(__value):\n  global _C__value",
            "name '_C__value' is parameter and global",
            "global _C__value",
        ),
        (
            "class C:\n def f(_C__value):\n  global __value",
            "name '__value' is parameter and global",
            "global __value",
        ),
        (
            "class C:\n def f(__value):\n  nonlocal _C__value",
            "name '_C__value' is parameter and nonlocal",
            "nonlocal _C__value",
        ),
        (
            "class C:\n def f():\n  __value\n  global _C__value",
            "name '_C__value' is used prior to global declaration",
            "global _C__value",
        ),
        (
            "class C:\n def f():\n  _C__value = 1\n  global __value",
            "name '__value' is assigned to before global declaration",
            "global __value",
        ),
        (
            "class C:\n def f(\u{212A}):\n  global K",
            "name 'K' is parameter and global",
            "global K",
        ),
    ] {
        all_versions_error(source, message, diagnostic);
    }
}

#[test]
fn nonlocal_resolution_distinguishes_enclosing_bindings_class_cells_and_type_parameters() {
    for (source, message, diagnostic) in [
        (
            "def outer():\n def f():\n  nonlocal missing, other",
            "no binding for nonlocal 'missing' found",
            "nonlocal missing, other",
        ),
        (
            "class C:\n value = 1\n def f():\n  nonlocal value",
            "no binding for nonlocal 'value' found",
            "nonlocal value",
        ),
        (
            "def outer():\n class C:\n  nonlocal __class__",
            "no binding for nonlocal '__class__' found",
            "nonlocal __class__",
        ),
        (
            "class C:\n def f():\n  global __class__\n  def g():\n   nonlocal __class__",
            "no binding for nonlocal '__class__' found",
            "nonlocal __class__",
        ),
        (
            "def outer():\n global value\n def f():\n  nonlocal value",
            "no binding for nonlocal 'value' found",
            "nonlocal value",
        ),
        (
            "def outer():\n [value for value in ()]\n def f():\n  nonlocal value",
            "no binding for nonlocal 'value' found",
            "nonlocal value",
        ),
        (
            "def f[Value]():\n nonlocal Value",
            "nonlocal binding not allowed for type parameter 'Value'",
            "nonlocal Value",
        ),
        (
            "def outer[Value]():\n def f():\n  nonlocal Value",
            "nonlocal binding not allowed for type parameter 'Value'",
            "nonlocal Value",
        ),
        (
            "class C[Value]:\n def f():\n  nonlocal Value",
            "nonlocal binding not allowed for type parameter 'Value'",
            "nonlocal Value",
        ),
    ] {
        all_versions_error(source, message, diagnostic);
    }
}

#[test]
fn comprehensions_keep_body_reads_local_and_walrus_bindings_in_the_enclosing_scope() {
    for (source, message, diagnostic) in [
        (
            "def f():\n result = [item for item in value]\n global value",
            "name 'value' is used prior to global declaration",
            "global value",
        ),
        (
            "def f():\n result = [(value := item) for item in ()]\n global value",
            "name 'value' is assigned to before global declaration",
            "global value",
        ),
        (
            "def f():\n global value\n result = ((value := item) for item in ())\n global value, spare",
            "name 'value' is assigned to before global declaration",
            "global value, spare",
        ),
    ] {
        all_versions_error(source, message, diagnostic);
    }
}

#[test]
fn accepted_declarations_preserve_imports_forward_bindings_and_independent_scopes() {
    for source in [
        "global value\nvalue: int",
        "def f():\n import os as value\n global value",
        "def f():\n import value.submodule\n global value",
        "def outer():\n value = 1\n def f():\n  import os as value\n  nonlocal value",
        "def outer():\n import os as value\n def f():\n  nonlocal value",
        "def f():\n global value\n global value",
        "def outer():\n def f():\n  nonlocal value\n value = 1",
        "def outer():\n value: int\n def f():\n  nonlocal value",
        "def outer():\n ((value := 1) for item in ())\n def f():\n  nonlocal value",
        "class C:\n def f():\n  nonlocal __class__",
        "class C:\n global __class__\n def f():\n  nonlocal __class__",
        "class A:\n class B:\n  nonlocal __class__",
        "class C:\n def f():\n  __value = 1\n  def g():\n   nonlocal _C__value",
        "def f():\n result = [value for value in ()]\n global value",
        "def f():\n result = [value for item in ()]\n global value",
        "def f():\n result = (value for item in ())\n global value",
        "def f():\n def g[T: value](): pass\n global value",
        "def f():\n class C[T](value): pass\n global value",
        "from __future__ import annotations\ndef f():\n value: external\n global external",
    ] {
        compile(source, "declarations.py").unwrap_or_else(|error| panic!("{source}: {error}"));
        for version in TARGETS {
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            compile_with_options(source, "declarations.py", &options)
                .unwrap_or_else(|error| panic!("{version:?}: {source}: {error}"));
        }
    }
}

#[test]
fn annotations_only_supply_ordinary_scope_reads_before_python_314() {
    for source in [
        "value: external\nglobal external",
        "def f():\n value: external\n global external",
        "def f():\n def g(value: external): pass\n global external",
    ] {
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            error_at(
                source,
                version,
                "name 'external' is used prior to global declaration",
                "global external",
            );
        }
        for version in [PythonVersion::Py314, PythonVersion::Py315] {
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            compile_with_options(source, "declarations.py", &options)
                .unwrap_or_else(|error| panic!("{version:?}: {source}: {error}"));
        }
    }
}
