"""Audit strong owners using only the previously captured isolated-child memory."""

from __future__ import annotations

import argparse
import bisect
import collections
import hashlib
import json
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--capture-dir", type=Path, required=True)
parser.add_argument(
    "--native-capture", type=Path, required=True, help="Saved direct-C capture"
)
args = parser.parse_args()
if not __debug__:
    parser.error("ownership validation requires assertions; do not use -O")
ROOT = args.capture_dir.resolve()
NATIVE = args.native_capture.resolve()
L = json.loads((ROOT / "layout.json").read_text())
S = json.loads((ROOT / "owner-layout.json").read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(pin):
    path = Path(pin["path"])
    assert path.stat().st_size == pin["bytes"] and digest(path) == pin["sha256"], path
    return path


SOURCE = json.loads((ROOT / "NATIVE-SOURCE.json").read_text())
assert verify(SOURCE["layout"]) == ROOT / "layout.json"
verify(SOURCE["header_provenance"])
verify(SOURCE["producer"])
assert verify(SOURCE["memory_capture"]) == NATIVE / "MEMORY-COMPLETE.json"
MEMORY = json.loads(verify(SOURCE["memory_capture"]).read_text())
assert MEMORY["actual_exit"] == 0
CAPTURE = json.loads(verify(MEMORY["capture"]).read_text())
REQUEST = json.loads(verify(CAPTURE["request"]).read_text())
MEMORY_REQUEST = json.loads(verify(MEMORY["memory_request"]).read_text())
assert MEMORY_REQUEST["parent_request"] == CAPTURE["request"]
assert CAPTURE["actual_exit"] == 0 and CAPTURE["status"] == "TWO_READ_NATIVE_STARTUP_CAPTURE"
assert CAPTURE["observed_equals_unobserved"] and CAPTURE["two_complete_reads_equal"]
assert CAPTURE["executable_bytes_unchanged"]
assert CAPTURE["thread_count"] == 1
assert verify(CAPTURE["observed"]).read_bytes() == verify(CAPTURE["unobserved"]).read_bytes()
assert CAPTURE["layout"] == SOURCE["original_layout"]
verify(CAPTURE["layout"])
verify(REQUEST["producer"])
verify(REQUEST["layout_source"])
verify(MEMORY_REQUEST["producer"])
assert MEMORY_REQUEST["capture_producer"] == REQUEST["producer"]
assert MEMORY_REQUEST["maps"]["sha256"] == CAPTURE["maps"]["sha256"]
maps = verify(MEMORY_REQUEST["maps"]).read_text().splitlines()
assert maps == verify(CAPTURE["maps"]).read_text().splitlines()
canonical = json.loads(verify(CAPTURE["canonical"]).read_text())
address_rows = json.loads(verify(CAPTURE["addresses"]).read_text())
assert len(canonical) == len(address_rows) == CAPTURE["entries"]
assert sorted(row["value"] for row in address_rows) == canonical
runtime = REQUEST["runtime"]
assert runtime["target"]["source_commit"] == "5607950ef232dad16d75c0cf53101d9649d89115"
verify(runtime["source"])
verify(runtime["builder"])
for row in runtime["headers"]["files"]:
    verify(row["artifact"])
images = [verify(runtime["binary"]), verify(runtime["libpython"])]
segments_file = verify(MEMORY_REQUEST["segments"])
segments = json.loads(segments_file.read_text())
starts = [s["start"] for s in segments]
assert starts == sorted(starts)
for s in segments:
    p = segments_file.parent / s["file"]
    assert digest(p) == s["sha256"]
    s["bytes"] = p.read_bytes()
    assert len(s["bytes"]) == s["end"] - s["start"]
assert sum(len(s["bytes"]) for s in segments) == MEMORY_REQUEST["retained_bytes"]

symbols = {}
for image in images:
    bases = []
    for line in maps:
        fields = line.split(maxsplit=5)
        if len(fields) == 6 and fields[5] == str(image):
            bases.append(int(fields[0].split("-")[0], 16) - int(fields[2], 16))
    assert bases, image
    base = min(bases)
    for arguments in (["nm", "-D", str(image)], ["nm", str(image)]):
        for line in subprocess.check_output(arguments, text=True, timeout=30).splitlines():
            fields = line.split()
            if len(fields) == 3:
                symbols.setdefault(fields[2], base + int(fields[0], 16))
assert symbols["PyUnicode_Type"] == MEMORY_REQUEST["unicode_type_address"]
assert symbols["_PyRuntime"] == MEMORY_REQUEST["runtime_address"]
type_names = {
    address: name for name, address in symbols.items() if name.endswith("_Type")
}
interns = {
    row["address"]: {**row, "codepoints": list(map(ord, row["value"]))}
    for row in address_rows
}
assert len(interns) == CAPTURE["entries"]


def read(address, length):
    index = bisect.bisect_right(starts, address) - 1
    assert index >= 0, hex(address)
    segment = segments[index]
    assert segment["start"] <= address and address + length <= segment["end"], (
        hex(address),
        length,
    )
    offset = address - segment["start"]
    return segment["bytes"][offset : offset + length]


def number(address, width=8, *, signed=False):
    return int.from_bytes(read(address, width), "little", signed=signed)


def field(address, name, width=8):
    return number(address + (S[name] if name in S else L[name]), width)


def object_type(address):
    return field(address, "object.ob_type")


def c_string(address):
    value = bytearray()
    for offset in range(512):
        byte = number(address + offset, 1)
        if not byte:
            return value.decode("ascii")
        value.append(byte)
    raise AssertionError("unterminated C name")


def dictionary_pairs(address):
    assert object_type(address) == symbols["PyDict_Type"]
    used = field(address, "dict.ma_used")
    keys = field(address, "dict.ma_keys")
    values = field(address, "dict.ma_values")
    kind = field(keys, "dict.keys.dk_kind", 1)
    assert kind in (L["dict.kind.general"], L["dict.kind.unicode"], 2)
    count = field(keys, "dict.keys.dk_nentries")
    assert used <= count < 1_000_000
    start = (
        keys
        + L["dict.keys.dk_indices"]
        + (1 << field(keys, "dict.keys.dk_log2_index_bytes", 1))
    )
    entry_kind = (
        "dict.general_entry."
        if kind == L["dict.kind.general"]
        else "dict.unicode_entry."
    )
    pairs = []
    for index in range(count):
        entry = start + index * L[entry_kind + "size"]
        key = field(entry, entry_kind + "key")
        value = (
            number(values + S["dict.values.values"] + index * 8)
            if values
            else field(entry, entry_kind + "value")
        )
        if value:
            assert key
            pairs.append((key, value))
    assert len(pairs) == used
    return pairs


def spelling(address):
    record = interns.get(address)
    if record is None:
        if object_type(address) != symbols["PyUnicode_Type"]:
            return hex(address)
        length = field(address, "unicode.length")
        state = field(address, "unicode.state", 4)
        kind = (state & L["unicode.state.kind_mask"]) >> 2
        assert length < 1_000_000 and kind in (1, 2, 4)
        if state & L["unicode.state.ascii_mask"]:
            data = address + L["unicode.ascii.size"]
        elif state & L["unicode.state.compact_mask"]:
            data = address + L["unicode.compact.size"]
        else:
            data = field(address, "unicode.data.any")
        return "".join(
            chr(number(data + index * kind, kind)) for index in range(length)
        )
    return "".join(map(chr, record["codepoints"]))


interp = MEMORY_REQUEST["interpreter_address"]
roots = []
for name in (
    "interpreter.imports.modules",
    "interpreter.builtins",
    "interpreter.sysdict",
):
    roots.append((field(interp, name), name))
for key, module in dictionary_pairs(field(interp, "interpreter.imports.modules")):
    if spelling(key) != "posix":
        continue
    assert object_type(module) == symbols["PyModule_Type"]
    definition = field(module, "module.md_def")
    assert c_string(field(definition, "moduledef.m_name")) == "posix"
    assert field(definition, "moduledef.m_size") == S["posixstate.size"]
    state = field(module, "module.md_state")
    assert state
    roots.append(
        (
            field(state, "posixstate.struct_rusage"),
            "interpreter.imports.modules[posix].C_state.struct_rusage",
        )
    )
owner_provenance = json.loads((ROOT / "owner-layout-provenance.json").read_text())
assert owner_provenance["source_commit"] == "5607950ef232dad16d75c0cf53101d9649d89115"
assert owner_provenance["layout_sha256"] == digest(ROOT / "owner-layout.json")
assert owner_provenance["generator_sha256"] == digest(
    verify(SOURCE["owner_layout_generator"])
)
for name in owner_provenance["additional_strong_root_fields"]:
    value = field(interp, name)
    if value:
        roots.append((value, name))
static_dicts = {}
count = field(interp, "interpreter.builtins.count")
assert 0 < count < 1000
for index in range(count):
    state = (
        interp + S["interpreter.builtins.initialized"] + index * S["static_state.size"]
    )
    typ = field(state, "static_state.type")
    dictionary = field(state, "static_state.tp_dict")
    assert typ and dictionary
    flags = field(typ, "type.tp_flags")
    assert flags & S["flags.immutabletype"] and flags & S["flags.ready"]
    static_dicts[typ] = dictionary
    roots.append((typ, f"immutable_static_type[{index}]"))
    roots.append((dictionary, f"immutable_static_type[{index}].dictionary"))
extension_count = field(interp, "interpreter.extensions.count")
populated = 0
for index in range(S["interpreter.extensions.capacity"]):
    state = (
        interp
        + S["interpreter.extensions.initialized"]
        + index * S["static_state.size"]
    )
    typ = field(state, "static_state.type")
    if not typ:
        continue
    populated += 1
    dictionary = field(state, "static_state.tp_dict")
    assert dictionary
    flags = field(typ, "type.tp_flags")
    assert flags & S["flags.immutabletype"] and flags & S["flags.ready"]
    static_dicts[typ] = dictionary
    roots.append((typ, f"immutable_extension_type[{index}]"))
    roots.append((dictionary, f"immutable_extension_type[{index}].dictionary"))
assert populated == extension_count

thread = field(interp, "interpreter.threads.head")
frame = field(thread, "thread.current_frame")
frame_count = 0
while frame:
    frame_count += 1
    assert frame_count < 100
    frame_fields = ["frame.f_executable"]
    if field(frame, "frame.owner", 1) != S["frame.owner.c_stack"]:
        frame_fields.append("frame.f_funcobj")
    for field_name in frame_fields:
        value = field(frame, field_name)
        if value and value & S["stackref.tag_mask"] == 0:
            roots.append((value, f"suspended_caller_frame[{frame_count}].{field_name}"))
    frame = field(frame, "frame.previous")

queue = collections.deque(roots)
seen = set()
owners = {}
unhandled = collections.Counter()
edge_kinds = collections.Counter()


def enqueue(address, path, edge):
    if address:
        queue.append((address, path + "." + edge))
        edge_kinds[edge.split("[")[0]] += 1


def pointer_fields(address, path, names):
    for name in names:
        enqueue(field(address, name), path, name)


while queue:
    address, path = queue.popleft()
    if address in seen:
        continue
    seen.add(address)
    assert len(seen) < 1_000_000
    typ = object_type(address)
    record = interns.get(address)
    if record:
        assert typ == symbols["PyUnicode_Type"]
        owners[address] = path
    if typ == symbols["PyUnicode_Type"]:
        continue
    if typ == symbols["PyDict_Type"]:
        for key, value in dictionary_pairs(address):
            label = spelling(key)
            enqueue(key, path, f"key[{label!r}]")
            enqueue(value, path, f"value[{label!r}]")
    elif typ == symbols["PyModule_Type"]:
        pointer_fields(address, path, ["module.md_dict"])
    elif typ == symbols["_PyNamespace_Type"]:
        pointer_fields(address, path, ["namespace.ns_dict"])
    elif typ == symbols["PyCode_Type"]:
        pointer_fields(
            address,
            path,
            [
                "code.co_consts",
                "code.co_names",
                "code.co_localsplusnames",
                "code.co_filename",
                "code.co_name",
                "code.co_qualname",
            ],
        )
    elif typ == symbols["PyFunction_Type"]:
        pointer_fields(
            address,
            path,
            [
                name
                for name in S
                if name.startswith("function.func_")
                and name != "function.func_weakreflist"
            ],
        )
    elif typ == symbols["PyCell_Type"]:
        pointer_fields(address, path, ["cell.ob_ref"])
    elif typ == symbols["PyMethod_Type"]:
        pointer_fields(address, path, ["method.im_func", "method.im_self"])
    elif typ in {symbols.get("PyCFunction_Type"), symbols.get("PyCMethod_Type")}:
        pointer_fields(address, path, ["builtin.m_self", "builtin.m_module"])
    elif typ in {
        symbols.get(name)
        for name in (
            "PyMethodDescr_Type",
            "PyClassMethodDescr_Type",
            "PyMemberDescr_Type",
            "PyGetSetDescr_Type",
            "PyWrapperDescr_Type",
        )
    }:
        pointer_fields(
            address,
            path,
            ["descriptor.d_type", "descriptor.d_name", "descriptor.d_qualname"],
        )
    elif typ in {symbols["PyClassMethod_Type"], symbols["PyStaticMethod_Type"]}:
        pointer_fields(address, path, ["wrapper.callable", "wrapper.dict"])
    elif typ == symbols["PyProperty_Type"]:
        pointer_fields(
            address, path, [name for name in S if name.startswith("property.prop_")]
        )
    elif typ in {symbols["PySet_Type"], symbols["PyFrozenSet_Type"]}:
        mask = field(address, "set.mask")
        table = field(address, "set.table")
        assert mask < 1_000_000
        for index in range(mask + 1):
            entry = table + index * S["set.entry.size"]
            key = field(entry, "set.entry.key")
            hashed = number(entry + S["set.entry.hash"], signed=True)
            if key and hashed != -1:
                enqueue(key, path, f"set_item[{index}]")
    else:
        flags = field(typ, "type.tp_flags")
        if flags & S["flags.type_subclass"]:
            enqueue(
                static_dicts.get(address) or field(address, "type.tp_dict"),
                path,
                "type.dictionary",
            )
            pointer_fields(address, path, ["type.tp_bases", "type.tp_mro"])
            if field(address, "type.tp_flags") & S["flags.heaptype"]:
                pointer_fields(
                    address,
                    path,
                    ["heaptype.ht_name", "heaptype.ht_qualname", "heaptype.ht_module"],
                )
        elif flags & S["flags.tuple_subclass"]:
            length = field(address, "varobject.ob_size")
            assert length < 1_000_000
            for index in range(length):
                enqueue(
                    number(address + S["tuple.ob_item"] + index * 8),
                    path,
                    f"tuple_item[{index}]",
                )
        elif flags & S["flags.list_subclass"]:
            length = field(address, "varobject.ob_size")
            items = field(address, "list.ob_item")
            assert length < 1_000_000
            for index in range(length):
                enqueue(number(items + index * 8), path, f"list_item[{index}]")
        else:
            unhandled[type_names.get(typ, hex(typ))] += 1

mortal = {
    address: record for address, record in interns.items() if record["state"] == 1
}
proof = [
    {"name": spelling(address), "address": address, "owner": owners.get(address)}
    for address in mortal
]
proof.sort(key=lambda row: row["name"])
report = {
    "canonical_sha256": CAPTURE["canonical"]["sha256"],
    "native_capture_sha256": digest(NATIVE / "COMPLETE.json"),
    "memory_capture_sha256": digest(NATIVE / "MEMORY-COMPLETE.json"),
    "native_source_sha256": digest(ROOT / "NATIVE-SOURCE.json"),
    "audit_script_sha256": digest(Path(__file__)),
    "owner_layout_sha256": digest(ROOT / "owner-layout.json"),
    "owner_layout_provenance_sha256": digest(ROOT / "owner-layout-provenance.json"),
    "visited_objects": len(seen),
    "mortal_entries": len(mortal),
    "retained_entries": sum(row["owner"] is not None for row in proof),
    "missing": [row["name"] for row in proof if row["owner"] is None],
    "proof": proof,
    "unhandled_object_types": dict(unhandled.most_common()),
    "edge_kinds": dict(edge_kinds),
}
with (ROOT / "retained-owner-audit.json").open("x") as stream:
    stream.write(json.dumps(report, indent=2) + "\n")
print(
    json.dumps(
        {key: value for key, value in report.items() if key != "proof"}, indent=2
    )
)
assert report["missing"] == [], "startup table has unresolved mortal ownership"
