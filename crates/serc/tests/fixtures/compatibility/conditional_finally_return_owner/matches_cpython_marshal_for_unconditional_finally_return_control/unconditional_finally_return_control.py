async def run_query(args, *, client=None) -> int:
    service_client = client
    close_client = False
    if service_client is None:
        service_client = ServiceClient(args.base_url)
        close_client = True

    try:
        list_result = await list_records(
            service_client,
            request_id=args.request_id,
            user_id=getattr(args, "user_id", None),
            page_size=getattr(args, "page_size", 200),
            max_pages=getattr(args, "max_pages", 25),
        )
        if getattr(args, "list_types", False):
            rows = await summarize_records(
                service_client,
                records=list_result.get("records", []),
            )
            render_rows(rows, json_output=getattr(args, "json", False))
            return 0
        await browse_records(
            service_client,
            request_id=args.request_id,
            list_result=list_result,
        )
    except Exception as exc:
        if isinstance(exc, ServiceError):
            print(
                "request_rejected: "
                f"{exc.base_url} returned {exc.status}: {compact_error(exc.body)}"
            )
            return 2
        if is_connection_error(exc):
            print(
                "request_unreachable: "
                f"could not connect to {args.base_url}; "
                "check the address; "
                f"cause={summarize_error(exc)}"
            )
            return 2
        raise
    finally:
        await close_service_client(service_client)
    return 0
