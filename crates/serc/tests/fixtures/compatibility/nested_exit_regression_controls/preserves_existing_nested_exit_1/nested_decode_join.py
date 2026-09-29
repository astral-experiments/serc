"""Public structural reduction candidate; not compiled or executed yet.

Exercise a terminal conditional following try/except inside nested if/elif
arms, with the return emitted after the outer chain.
"""
import json


def validate_payload(state, text, errors):
    if state == "empty":
        if text:
            errors.append("unexpected text")
    elif state in ("partial", "complete"):
        if state == "complete":
            if not isinstance(text, str) or not text.strip():
                errors.append("missing text")
            else:
                try:
                    decoded = json.loads(text)
                except json.JSONDecodeError:
                    decoded = None
                if not isinstance(decoded, dict):
                    errors.append("expected an object")
        elif text not in ("", None):
            errors.append("unexpected partial text")
    elif state in ("invalid", "mixed"):
        if text not in ("", None):
            errors.append("unexpected invalid text")
    return state
