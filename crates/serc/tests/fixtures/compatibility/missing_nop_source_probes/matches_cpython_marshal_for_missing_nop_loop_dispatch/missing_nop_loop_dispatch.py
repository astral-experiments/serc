from __future__ import annotations

import hashlib

def fingerprint(chunks):
    hasher = hashlib.sha256()
    if hasattr(chunks, 'read'):
        source = chunks
        while True:
            chunk = source.read(4096)
            if not chunk:
                break
            hasher.update(chunk)
    else:
        for chunk in chunks:
            hasher.update(chunk)
    return hasher.hexdigest()
