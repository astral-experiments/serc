def read_size(open_resource, open_image, read_bytes, report):
    try:
        with open_resource() as resource:
            with open_image(resource) as image:
                image.load()
                size = (int(image.width), int(image.height))
    except Exception:
        try:
            data = read_bytes()
            image = open_image(data)
            image.load()
            size = (int(image.width), int(image.height))
        except Exception as error:
            report(error)
            return None
    return size
