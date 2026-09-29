from __future__ import annotations


def build_preview(path, directory, tag):
    from preview_codec import Buffer, Document

    directory.ensure(visible=True, create=True)
    document = Document.open(Buffer.read(str(path)))
    label = None
    try:
        summary = document.inspect({"kind": "tab", "include": "key,label", "limit": 2000})
        for row in str(summary.entries).splitlines():
            try:
                item = codec.loads(row)
            except codec.DecodeError:
                continue
            label = item.get("label") or item.get("alias") or item.get("key")
            if label:
                break
    except Exception:
        label = None

    settings: dict[str, object] = {"area": "B2:Q10", "scale": 1}
    if label:
        settings["label"] = label
    payload = document.render(settings)
    output = directory / f"{tag}_preview.bin"
    payload.save(str(output))
    return [str(output)]
