def target(local_path, root, metadata, destination, stats, count):
    try:
        if local_path is None:
            return []
        frames = list(
            extract(
                local_path,
                root,
                count,
                stats=stats,
                enabled=True,
            )
        )
        started = clock.time()
        uploaded = upload(
            frames,
            destination,
        )
        elapsed = clock.time() - started
        print(
            f"id={metadata.get('id')} "
            f"bytes={metadata.get('size')} "
            f"frames={len(frames)} "
            f"elapsed={elapsed:.2f}"
        )
        return uploaded
    finally:
        try:
            cleanup.remove_tree(root, ignore_errors=True)
        finally:
            if local_path:
                try:
                    cleanup.remove(local_path)
                except Exception:
                    pass
