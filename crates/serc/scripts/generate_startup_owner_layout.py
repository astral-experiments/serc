"""Generate offline owner layouts from the pinned CPython source and headers.

The capture directory must contain layout.json and header-provenance.json from
an already admitted capture. This script compiles and runs only a standalone C
layout helper; it never starts Python, loads libpython, or accesses a process.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

COMMIT = "5607950ef232dad16d75c0cf53101d9649d89115"


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--capture-dir",
        type=Path,
        required=True,
        help="Directory containing admitted header-provenance.json and layout.json",
    )
    parser.add_argument(
        "--cpython",
        type=Path,
        required=True,
        help="CPython checkout at the exact pinned commit",
    )
    parser.add_argument(
        "--headers",
        type=Path,
        required=True,
        help="Installed include/python3.14 directory from the admitted runtime",
    )
    args = parser.parse_args()
    if not __debug__:
        parser.error("Run without -O: provenance assertions must remain enabled")
    ROOT = args.capture_dir.resolve()
    SOURCE = args.cpython.resolve()
    HEADERS = args.headers.resolve()
    PROOF = json.loads((ROOT / "header-provenance.json").read_text())

    assert PROOF["source_commit"] == COMMIT
    assert (
        subprocess.check_output(
            ["git", "-C", str(SOURCE), "rev-parse", "HEAD"], text=True
        ).strip()
        == COMMIT
    )
    for name, entry in PROOF["headers"].items():
        assert sha((HEADERS / name).read_bytes()) == entry["installed_sha256"], name
        if name != "pyconfig.h":
            assert entry["installed_sha256"] == entry["exact_tag_file_sha256"], name

    copies = []
    private_proof = {}
    for source, original, replacement in [
        ("Objects/funcobject.c", "classmethod", "OwnerClassMethod"),
        ("Objects/funcobject.c", "staticmethod", "OwnerStaticMethod"),
        ("Objects/descrobject.c", "mappingproxyobject", "OwnerMappingProxy"),
        ("Objects/descrobject.c", "wrapperobject", "OwnerMethodWrapper"),
        ("Include/internal/pycore_descrobject.h", "propertyobject", "OwnerProperty"),
        ("Objects/namespaceobject.c", "_PyNamespaceObject", "OwnerNamespace"),
        ("Modules/posixmodule.c", "_posixstate", "OwnerPosixState"),
    ]:
        data = subprocess.check_output(
            ["git", "-C", str(SOURCE), "show", f"{COMMIT}:{source}"]
        )
        text = data.decode()
        pattern = (
            r"typedef struct \{(?:(?!typedef struct \{).)*?\}\s*"
            + re.escape(original)
            + r";"
        )
        matches = re.findall(pattern, text, re.S)
        assert len(matches) == 1, (source, original, len(matches))
        block = matches[0]
        assert "PyObject_HEAD" in block or original == "_posixstate"
        private_proof[original] = {
            "source": source,
            "source_sha256": sha(data),
            "declaration_sha256": sha(block.encode()),
            "first_line": text[: text.index(block)].count("\n") + 1,
            "copied_name": replacement,
        }
        copied = re.sub(r"\b" + re.escape(original) + r";$", replacement + ";", block)
        copies.append(
            f"/* Exact {source} declaration sha256:{sha(block.encode())}. */\n{copied}"
        )

    def committed(path):
        return subprocess.check_output(
            ["git", "-C", str(SOURCE), "show", f"{COMMIT}:{path}"]
        ).decode()

    # The exact finalizers establish that these interpreter fields own references;
    # a pointer-shaped field alone is not an ownership proof.
    ast_header = committed("Include/internal/pycore_ast_state.h")
    ast_source = committed("Python/Python-ast.c")
    ast_fields = re.findall(r"^    PyObject \*(\w+);$", ast_header, re.M)
    ast_finalizer = re.search(
        r"void _PyAST_Fini\(PyInterpreterState \*interp\)\n\{(.*?)\n\}",
        ast_source,
        re.S,
    ).group(1)
    assert set(ast_fields) == set(
        re.findall(r"Py_CLEAR\(state->(\w+)\)", ast_finalizer)
    )
    assert "_fields" in ast_fields and "_attributes" in ast_fields
    typevar_source = committed("Objects/typevarobject.c")
    typevar_finalizer = re.search(
        r"void _Py_clear_generic_types\(PyInterpreterState \*interp\)\n\{(.*?)\n\}",
        typevar_source,
        re.S,
    ).group(1)
    typevar_fields = re.findall(
        r"Py_CLEAR\(interp->cached_objects\.(\w+)\)", typevar_finalizer
    )
    assert set(typevar_fields) == {
        name + "_type"
        for name in re.findall(r"^    MAKE_TYPE\((\w+)\);", typevar_source, re.M)
    }
    assert len(typevar_fields) == 7
    strong_root_fields = [
        *(f"ast.{name}" for name in ast_fields),
        *(f"cached_objects.{name}" for name in typevar_fields),
    ]
    strong_root_proof = {
        "ast": {
            "header": "Include/internal/pycore_ast_state.h",
            "header_sha256": sha(ast_header.encode()),
            "source": "Python/Python-ast.c",
            "source_sha256": sha(ast_source.encode()),
            "ownership_function": "_PyAST_Fini",
            "fields": ast_fields,
        },
        "generic_types": {
            "source": "Objects/typevarobject.c",
            "source_sha256": sha(typevar_source.encode()),
            "ownership_functions": [
                "_Py_initialize_generic",
                "_Py_clear_generic_types",
            ],
            "fields": typevar_fields,
        },
    }
    posix_source = committed("Modules/posixmodule.c")
    assert (
        'state->struct_rusage = PyUnicode_InternFromString("struct_rusage")'
        in posix_source
    )
    assert "Py_VISIT(state->struct_rusage)" in posix_source
    assert "Py_CLEAR(state->struct_rusage)" in posix_source
    strong_root_proof["posix_struct_rusage"] = {
        "source": "Modules/posixmodule.c",
        "source_sha256": sha(posix_source.encode()),
        "initialization": 'state->struct_rusage = PyUnicode_InternFromString("struct_rusage")',
        "ownership_functions": ["_posix_traverse", "_posix_clear"],
        "build_condition": "defined(HAVE_WAIT3) || defined(HAVE_WAIT4)",
        "owner": "posix module md_state; _posixstate.struct_rusage",
        "scope": "This offset is not valid for other module state blocks.",
    }

    groups = [
        ("object", "PyObject", ["ob_type", "ob_refcnt"]),
        ("varobject", "PyVarObject", ["ob_size"]),
        (
            "interpreter",
            "PyInterpreterState",
            [
                "imports.modules",
                "imports.modules_by_index",
                "builtins",
                "builtins_copy",
                "sysdict",
                "threads.head",
                "threads.main",
                "threads.count",
                "codecs.search_path",
                "codecs.search_cache",
                "codecs.error_registry",
                *strong_root_fields,
            ],
        ),
        (
            "static_state",
            "managed_static_type_state",
            ["type", "isbuiltin", "readying", "ready", "tp_dict"],
        ),
        (
            "thread",
            "PyThreadState",
            ["next", "interp", "current_frame", "dict", "context", "exc_info"],
        ),
        ("exception_stack", "_PyErr_StackItem", ["exc_value", "previous_item"]),
        ("module", "PyModuleObject", ["md_dict", "md_name", "md_state", "md_def"]),
        ("moduledef", "PyModuleDef", ["m_name", "m_size"]),
        ("posixstate", "OwnerPosixState", ["struct_rusage"]),
        (
            "type",
            "PyTypeObject",
            [
                "tp_name",
                "tp_flags",
                "tp_dict",
                "tp_bases",
                "tp_mro",
                "tp_base",
                "tp_basicsize",
                "tp_itemsize",
                "tp_dictoffset",
                "tp_members",
            ],
        ),
        (
            "heaptype",
            "PyHeapTypeObject",
            ["ht_name", "ht_qualname", "ht_module", "ht_slots", "ht_cached_keys"],
        ),
        ("tuple", "PyTupleObject", ["ob_item"]),
        ("list", "PyListObject", ["ob_item", "allocated"]),
        ("set", "PySetObject", ["fill", "used", "mask", "table"]),
        ("set.entry", "setentry", ["key", "hash"]),
        (
            "dict.values",
            "PyDictValues",
            ["values", "capacity", "size", "embedded", "valid"],
        ),
        (
            "code",
            "PyCodeObject",
            [
                "co_consts",
                "co_names",
                "co_localsplusnames",
                "co_filename",
                "co_name",
                "co_qualname",
                "co_nlocalsplus",
                "co_stacksize",
                "co_framesize",
                "co_firstlineno",
            ],
        ),
        (
            "function",
            "PyFunctionObject",
            [
                "func_globals",
                "func_builtins",
                "func_name",
                "func_qualname",
                "func_code",
                "func_defaults",
                "func_kwdefaults",
                "func_closure",
                "func_doc",
                "func_dict",
                "func_module",
                "func_annotations",
                "func_annotate",
                "func_typeparams",
            ],
        ),
        ("cell", "PyCellObject", ["ob_ref"]),
        ("method", "PyMethodObject", ["im_func", "im_self"]),
        ("instance_method", "PyInstanceMethodObject", ["func"]),
        ("builtin", "PyCFunctionObject", ["m_self", "m_module"]),
        ("builtin_method", "PyCMethodObject", ["mm_class"]),
        ("descriptor", "PyDescrObject", ["d_type", "d_name", "d_qualname"]),
        (
            "property",
            "OwnerProperty",
            ["prop_get", "prop_set", "prop_del", "prop_doc", "prop_name"],
        ),
        ("classmethod", "OwnerClassMethod", ["cm_callable", "cm_dict"]),
        ("staticmethod", "OwnerStaticMethod", ["sm_callable", "sm_dict"]),
        ("mappingproxy", "OwnerMappingProxy", ["mapping"]),
        ("namespace", "OwnerNamespace", ["ns_dict"]),
        ("method_wrapper", "OwnerMethodWrapper", ["descr", "self"]),
        (
            "frame",
            "_PyInterpreterFrame",
            [
                "f_executable",
                "f_funcobj",
                "previous",
                "f_globals",
                "f_builtins",
                "f_locals",
                "frame_obj",
                "stackpointer",
                "owner",
                "localsplus",
            ],
        ),
        (
            "frameobject",
            "PyFrameObject",
            [
                "f_back",
                "f_frame",
                "f_trace",
                "f_extra_locals",
                "f_locals_cache",
                "f_overwritten_fast_locals",
            ],
        ),
        ("member_definition", "PyMemberDef", ["name", "type", "offset", "flags"]),
    ]
    lines = [
        """/* Generated offline; this program neither initializes nor links libpython.
     * Numeric offsets are not ownership claims. Weak references are omitted.
     * In particular frame globals/builtins are borrowed and frame.previous is a chain.
     */
    #define Py_BUILD_CORE 1
    #include <Python.h>
    #include <stddef.h>
    #include <stdio.h>
    #include "internal/pycore_interp_structs.h"
    #include "internal/pycore_moduleobject.h"
    #include "internal/pycore_interpframe_structs.h"
    #include "internal/pycore_frame.h"
    #include "internal/pycore_stackref.h"
    #include "internal/pycore_dict.h"
    #include "internal/pycore_descrobject.h"
    #if PY_VERSION_HEX != 0x030e05f0
    #error "This layout requires the pinned CPython 3.14.5 headers"
    #endif
    #if defined(Py_GIL_DISABLED) || defined(Py_STACKREF_DEBUG)
    #error "This layout requires the admitted GIL build and direct stackrefs"
    #endif
    _Static_assert(sizeof(void *) == 8, "The captured target is 64-bit");
    """,
        *copies,
        """
    _Static_assert(sizeof(OwnerProperty) == sizeof(propertyobject), "property layout");
    _Static_assert(offsetof(OwnerProperty, prop_name) == offsetof(propertyobject, prop_name), "property name");
    _Static_assert(sizeof(OwnerClassMethod) == sizeof(OwnerStaticMethod), "wrapper size");
    _Static_assert(offsetof(OwnerClassMethod, cm_callable) == offsetof(OwnerStaticMethod, sm_callable), "wrapper callable");
    _Static_assert(offsetof(OwnerClassMethod, cm_dict) == offsetof(OwnerStaticMethod, sm_dict), "wrapper dict");
    static int first = 1;
    static void emit(const char *key, long long value) {
        printf("%s  \\\"%s\\\": %lld", first ? "" : ",\\n", key, value);
        first = 0;
    }
    #define O(KEY, TYPE, MEMBER) emit(KEY, (long long)offsetof(TYPE, MEMBER))
    #define S(KEY, TYPE) emit(KEY, (long long)sizeof(TYPE))
    #define N(KEY, VALUE) emit(KEY, (long long)(VALUE))
    int main(void) {
        puts("{");
    """,
    ]
    for prefix, typ, fields in groups:
        size_key = "struct_size" if "size" in fields else "size"
        lines.append(f'    S("{prefix}.{size_key}", {typ});')
        for field in fields:
            lines.append(f'    O("{prefix}.{field}", {typ}, {field});')
    for key, typ, field in [
        (
            "interpreter.builtins.count",
            "PyInterpreterState",
            "types.builtins.num_initialized",
        ),
        (
            "interpreter.builtins.initialized",
            "PyInterpreterState",
            "types.builtins.initialized",
        ),
        (
            "interpreter.extensions.count",
            "PyInterpreterState",
            "types.for_extensions.num_initialized",
        ),
        (
            "interpreter.extensions.next_index",
            "PyInterpreterState",
            "types.for_extensions.next_index",
        ),
        (
            "interpreter.extensions.initialized",
            "PyInterpreterState",
            "types.for_extensions.initialized",
        ),
        ("wrapper.callable", "OwnerClassMethod", "cm_callable"),
        ("wrapper.dict", "OwnerClassMethod", "cm_dict"),
    ]:
        lines.append(f'    O("{key}", {typ}, {field});')
    for key, expr in {
        "pointer.size": "sizeof(void *)",
        "stackref.size": "sizeof(_PyStackRef)",
        "stackref.tag_mask": "Py_TAG_BITS",
        "stackref.refcnt_tag": "Py_TAG_REFCNT",
        "stackref.int_tag": "Py_TAG_BITS",
        "stackref.null_bits": "PyStackRef_NULL_BITS",
        "flags.type_subclass": "Py_TPFLAGS_TYPE_SUBCLASS",
        "flags.tuple_subclass": "Py_TPFLAGS_TUPLE_SUBCLASS",
        "flags.list_subclass": "Py_TPFLAGS_LIST_SUBCLASS",
        "flags.heaptype": "Py_TPFLAGS_HEAPTYPE",
        "flags.static_builtin": "_Py_TPFLAGS_STATIC_BUILTIN",
        "flags.immutabletype": "Py_TPFLAGS_IMMUTABLETYPE",
        "flags.ready": "Py_TPFLAGS_READY",
        "flags.managed_dict": "Py_TPFLAGS_MANAGED_DICT",
        "flags.inline_values": "Py_TPFLAGS_INLINE_VALUES",
        "managed_dict.offset": "MANAGED_DICT_OFFSET",
        "interpreter.builtins.capacity": "_Py_MAX_MANAGED_STATIC_BUILTIN_TYPES",
        "interpreter.extensions.capacity": "_Py_MAX_MANAGED_STATIC_EXT_TYPES",
        "frame.owner.thread": "FRAME_OWNED_BY_THREAD",
        "frame.owner.generator": "FRAME_OWNED_BY_GENERATOR",
        "frame.owner.frame_object": "FRAME_OWNED_BY_FRAME_OBJECT",
        "frame.owner.interpreter": "FRAME_OWNED_BY_INTERPRETER",
        "frame.owner.c_stack": "FRAME_OWNED_BY_CSTACK",
        "member.type.object": "_Py_T_OBJECT",
        "member.type.object_ex": "Py_T_OBJECT_EX",
    }.items():
        lines.append(f'    N("{key}", {expr});')
    lines.append('    puts("\\n}");\n    return 0;\n}\n')
    helper = ROOT / "owner-layout.c"
    helper.write_text("\n".join(lines))
    command = [
        "cc",
        "-std=c11",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-isystem",
        str(HEADERS),
        str(helper),
        "-o",
        str(ROOT / "owner-layout"),
    ]
    subprocess.run(command, check=True)
    output = subprocess.check_output([str(ROOT / "owner-layout")])

    def unique_object(pairs):
        assert len(pairs) == len(dict(pairs)), "duplicate layout key"
        return dict(pairs)

    layout = json.loads(output, object_pairs_hook=unique_object)
    dependencies = subprocess.check_output(
        ["readelf", "-d", str(ROOT / "owner-layout")], text=True
    )
    undefined_symbols = subprocess.check_output(
        ["nm", "-u", str(ROOT / "owner-layout")], text=True
    )
    assert "libpython" not in dependencies
    assert not re.search(r"\b_?Py[A-Za-z_]", undefined_symbols)
    base = json.loads((ROOT / "layout.json").read_text())
    for key in base.keys() & layout.keys():
        assert base[key] == layout[key], (key, base[key], layout[key])
    (ROOT / "owner-layout.json").write_text(
        json.dumps(layout, indent=2, sort_keys=True) + "\n"
    )
    receipt = {
        "source_commit": COMMIT,
        "header_provenance_sha256": sha((ROOT / "header-provenance.json").read_bytes()),
        "private_declarations": private_proof,
        "strong_root_proof": strong_root_proof,
        "additional_strong_root_fields": [
            "interpreter." + name for name in strong_root_fields
        ],
        "generator_sha256": sha(Path(__file__).read_bytes()),
        "helper_sha256": sha(helper.read_bytes()),
        "binary_sha256": sha((ROOT / "owner-layout").read_bytes()),
        "layout_sha256": sha((ROOT / "owner-layout.json").read_bytes()),
        "compile_command": [
            "cc",
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-isystem",
            "<headers>",
            "<capture-dir>/owner-layout.c",
            "-o",
            "<capture-dir>/owner-layout",
        ],
        "compiler_version": subprocess.check_output(
            ["cc", "--version"], text=True
        ).splitlines()[0],
        "dynamic_dependencies": [
            line.strip() for line in dependencies.splitlines() if "(NEEDED)" in line
        ],
        "undefined_symbols": undefined_symbols.splitlines(),
        "libpython_linked": False,
        "target_process_access": False,
        "header_target": "CPython 3.14.5 default-GIL x86_64",
    }
    (ROOT / "owner-layout-provenance.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n"
    )
    print(
        json.dumps(
            {"layout_entries": len(layout), "layout_sha256": receipt["layout_sha256"]}
        )
    )


if __name__ == "__main__":
    main()
