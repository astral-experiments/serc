def factory(ticketing_service, audit_calls=True):
    tool_name = "enrich"
    async def f(attributes, option, destination):
        status = "success"
        error = None
        support_name = str(option.get("name") or "").strip()
        support_option = str(option.get("id") or "").strip()
        attributes = (
            [attribute for attribute in attributes if isinstance(attribute, Mapping)]
            if isinstance(attributes, list)
            else []
        )
        audit_args = {
            "system": "issue",
            "operation": "enrich",
            "destination_hash": digest(destination),
            "support_name": support_name,
            "support_option": support_option,
            "attributes_hash": digest([
                {
                    "label": str(attribute.get("label") or "").strip(),
                    "value": str(attribute.get("value") or "").strip(),
                }
                for attribute in attributes
            ]),
        }
        try:
            if not support_option:
                raise ValueError("missing option")
            payload = await ticketing_service.visit(support_option, audit_args)
        except Exception as exc:
            status = "error"
            error = repr(exc)
            payload = {"error": "failed", "message": str(exc), "system": "issue"}
            report(tool_name, error)
        if audit_calls:
            await record(tool_name, audit_args, status, payload, error)
        return payload
    return f
