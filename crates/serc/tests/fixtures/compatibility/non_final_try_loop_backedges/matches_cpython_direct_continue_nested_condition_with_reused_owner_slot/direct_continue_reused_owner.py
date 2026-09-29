def token_counts(doc):
    total = 0
    text = 0
    unmasked_text = 0
    for tok in doc.get("tokens", []) or []:
        total += 1
        try:
            if tok.get("modality") == "text":
                text += 1
                if float(tok.get("loss_weight", 1.0)) > 0:
                    unmasked_text += 1
        except Exception:
            continue
    return total, text, unmasked_text
