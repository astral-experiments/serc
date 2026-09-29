def run(resource, chunks, all_chunks):
    def capture():
        return resource
    try:
        for chunk in chunks:
            consume(chunk)
        print(json.dumps({"selected": len(chunks), "total": len(all_chunks)}))
        return 0
    finally:
        if resource is not None:
            resource.close()
